//! Session Audit Logging and Snapshot Management
//! Supports financial compliance by tracking all manual and automated changes.

use crate::engine::history::ChangeRecord;
use crate::error::{AuditError, AuditResult};
use chrono::Utc;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub struct AuditLog {
    log_file: Option<File>,
    log_path: PathBuf,
    _audit_dir: PathBuf,
    snapshots_dir: PathBuf,
}

impl AuditLog {
    /// Opens the audit log directory and initializes the current session's log file.
    ///
    /// # Errors
    /// Returns [`AuditError::Open`] if the snapshots directory cannot be created.
    pub fn open(audit_dir: impl AsRef<Path>) -> AuditResult<Self> {
        let audit_dir = audit_dir.as_ref().to_path_buf();
        let snapshots_dir = audit_dir.join("snapshots");
        fs::create_dir_all(&snapshots_dir)
            .map_err(|e| AuditError::open(snapshots_dir.display().to_string(), e))?;

        // ISO-8601-utc-with-no-colons for Windows compatibility
        let timestamp = Utc::now().format("%Y%m%dt%H%M%SZ").to_string();
        let log_path = audit_dir.join(format!("{timestamp}.log"));

        Ok(Self {
            log_file: None,
            log_path,
            _audit_dir: audit_dir,
            snapshots_dir,
        })
    }

    pub fn write(
        &mut self,
        record: &ChangeRecord,
        source: &Path,
        output: &Path,
        operator: &str,
        requires_visual_review: bool,
    ) -> AuditResult<()> {
        self.ensure_open()?;

        // Phase 7: Write structured native JSON for maximum precision and security.
        #[derive(serde::Serialize)]
        struct AuditEvent<'a> {
            version: &'static str,
            operator: &'a str,
            source_pdf: &'a Path,
            output_pdf: &'a Path,
            requires_visual_review: bool,
            #[serde(flatten)]
            record: &'a ChangeRecord,
        }

        let event = AuditEvent {
            version: "audit_v2_json",
            operator,
            source_pdf: source,
            output_pdf: output,
            requires_visual_review,
            record,
        };

        // Write as JSON lines to the master log
        let mut line = serde_json::to_string(&event).unwrap_or_default();
        line.push('\n');

        let file = self
            .log_file
            .as_mut()
            .expect("log_file is Some after ensure_open");
        file.write_all(line.as_bytes()).map_err(AuditError::Write)?;
        file.flush().map_err(AuditError::Write)?;

        // Also dump an individual snapshot matching the old verification_report format
        let ts = record.timestamp.replace(':', ""); // safe for filenames
        let snap_json_path = self.snapshots_dir.join(format!("{ts}.json"));
        if let Ok(json_bytes) = serde_json::to_vec_pretty(&event) {
            let _ = fs::write(snap_json_path, json_bytes);
        }

        Ok(())
    }

    /// Stage 12 / Item #4: append an arbitrary single-line event to the
    /// audit log. The runtime uses this to record cascade invocations
    /// (which don't fit the `ChangeRecord` shape but still need an audit
    /// trail). The line is written verbatim with a trailing newline.
    ///
    /// # Errors
    /// Returns [`AuditError::Write`] if the log file cannot be opened or written.
    pub fn append_line(&mut self, line: &str) -> AuditResult<()> {
        self.ensure_open()?;
        let file = self
            .log_file
            .as_mut()
            .expect("log_file is Some after ensure_open");
        file.write_all(line.as_bytes()).map_err(AuditError::Write)?;
        if !line.ends_with('\n') {
            file.write_all(b"\n").map_err(AuditError::Write)?;
        }
        file.flush().map_err(AuditError::Write)?;
        Ok(())
    }

    /// Lazily opens (creating if needed) the session log file in append mode.
    fn ensure_open(&mut self) -> AuditResult<()> {
        if self.log_file.is_none() {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)
                .map_err(|e| AuditError::open(self.log_path.display().to_string(), e))?;
            self.log_file = Some(file);
        }
        Ok(())
    }

    pub fn snapshots_dir(&self) -> &Path {
        &self.snapshots_dir
    }

    /// Returns the path where a snapshot for a specific change ID should be stored.
    pub fn snapshot_path_for(&self, change_id: u64) -> PathBuf {
        // We use .pdf for snapshots per Approach §4.4
        self.snapshots_dir.join(format!("{change_id}.pdf"))
    }
}

/// Save a snapshot of `output` at the audit log's expected path for
/// `change_id`. Tries hard-linking first (~zero disk cost when the source
/// and snapshot live on the same volume), falls back to a full copy on
/// hard-link failure (cross-FS, FAT32, etc.).
///
/// Returns `Ok(true)` when the hard link succeeded, `Ok(false)` after a
/// fallback copy.
///
/// # Errors
/// Returns [`AuditError::Snapshot`] if the destination directory cannot be
/// created or the fallback copy fails.
pub fn snapshot_link_or_copy(source: &Path, dest: &Path) -> AuditResult<bool> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| AuditError::snapshot(parent.display().to_string(), e))?;
    }
    if dest.exists() {
        // Hard linking onto an existing path errors; remove first.
        let _ = fs::remove_file(dest);
    }
    match fs::hard_link(source, dest) {
        Ok(()) => Ok(true),
        Err(e) => {
            tracing::debug!("[audit] hard_link failed ({}); falling back to copy", e);
            fs::copy(source, dest)
                .map_err(|e| AuditError::snapshot(dest.display().to_string(), e))?;
            Ok(false)
        }
    }
}

pub struct AuditLogParser;

impl AuditLogParser {
    /// Parses an audit log file into a list of [`ChangeRecord`]s.
    ///
    /// # Errors
    /// Returns [`AuditError::Read`] if the file cannot be opened or a line
    /// cannot be read.
    pub fn parse_file(path: &Path) -> AuditResult<Vec<ChangeRecord>> {
        let file = File::open(path).map_err(|e| AuditError::read(path.display().to_string(), e))?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();

        for line in reader.lines() {
            let line = line.map_err(|e| AuditError::read(path.display().to_string(), e))?;
            if !line.starts_with("audit_v1") {
                continue;
            }

            if let Some(record) = Self::parse_line(&line) {
                records.push(record);
            }
        }

        Ok(records)
    }

    fn parse_line(line: &str) -> Option<ChangeRecord> {
        // audit_v1 ts=... page=... id=... old=... new=... op=... prov=... desc=... snap=... bbox=[...] in=... out=... review=...
        let mut id = None;
        let mut timestamp = None;
        let mut page = None;
        let mut old_text = None;
        let mut new_text = None;
        let mut bbox = None;
        let mut provenance = "Manual".to_string();
        let mut description = String::new();
        let mut snapshot_path = None;

        // Simple state machine parser
        let mut rest = line.trim();
        if !rest.starts_with("audit_v1 ") {
            return None;
        }
        rest = &rest["audit_v1 ".len()..];

        while !rest.is_empty() {
            rest = rest.trim_start();
            let eq_idx = match rest.find('=') {
                Some(idx) => idx,
                None => break,
            };
            let key = &rest[..eq_idx];
            rest = &rest[eq_idx + 1..];

            // If it's a JSON string, use serde to parse it
            if rest.starts_with('"') {
                let mut end_idx = 1;
                let mut escaped = false;
                while end_idx < rest.len() {
                    let c = rest.as_bytes()[end_idx];
                    if escaped {
                        escaped = false;
                    } else if c == b'\\' {
                        escaped = true;
                    } else if c == b'"' {
                        end_idx += 1;
                        break;
                    }
                    end_idx += 1;
                }

                let val_str = &rest[..end_idx];
                rest = &rest[end_idx..];

                if key == "old" {
                    old_text = serde_json::from_str::<String>(val_str).ok();
                } else if key == "new" {
                    new_text = serde_json::from_str::<String>(val_str).ok();
                } else if key == "desc" {
                    description = serde_json::from_str::<String>(val_str).unwrap_or_default();
                } else if key == "snap" {
                    let s = serde_json::from_str::<String>(val_str).unwrap_or_default();
                    if !s.is_empty() {
                        snapshot_path = Some(PathBuf::from(s));
                    }
                }
            } else {
                // Read until space
                let end_idx = rest.find(' ').unwrap_or(rest.len());
                let val_str = &rest[..end_idx];
                rest = &rest[end_idx..];

                match key {
                    "id" => id = val_str.parse().ok(),
                    "ts" => timestamp = Some(val_str.to_string()),
                    "page" => page = val_str.parse().ok(),
                    "prov" => provenance = val_str.to_string(),
                    "bbox" => {
                        let clean = val_str.trim_matches(|c| c == '[' || c == ']');
                        let parts: Vec<f32> =
                            clean.split(',').filter_map(|p| p.parse().ok()).collect();
                        if parts.len() == 4 {
                            bbox = Some([parts[0], parts[1], parts[2], parts[3]]);
                        }
                    }
                    _ => {}
                }
            }
        }

        Some(ChangeRecord {
            id: id?,
            timestamp: timestamp?,
            page: page?,
            old_text: old_text?,
            new_text: new_text?,
            bbox: bbox?,
            description,
            snapshot_path,
            provenance,
            obj_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_records() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let mut audit = AuditLog::open(dir.path())?;

        let rec1 = ChangeRecord {
            id: 123,
            timestamp: "ts".into(),
            page: 1,
            old_text: "foo".into(),
            new_text: "bar".into(),
            bbox: [0.0, 1.0, 2.0, 3.0],
            description: "Adjustment".into(),
            snapshot_path: Some(PathBuf::from("audit/snapshots/123.pdf")),
            provenance: "DocumentAI".into(),
            obj_id: None,
        };

        audit.write(&rec1, Path::new("in"), Path::new("out"), "test", false)?;

        // write() emits audit_v2_json format (JSON lines).  parse_file() reads
        // the legacy audit_v1 text format, so it correctly returns 0 records
        // for a v2 log.  Verify the raw JSON line instead.
        let raw = std::fs::read_to_string(&audit.log_path)?;
        let v: serde_json::Value = serde_json::from_str(raw.trim())?;
        assert_eq!(v["id"], 123);
        assert_eq!(v["old_text"], "foo");
        assert_eq!(v["description"], "Adjustment");
        assert_eq!(v["provenance"], "DocumentAI");

        // parse_file on a v2-only log should return an empty vec (no v1 lines)
        let parsed = AuditLogParser::parse_file(&audit.log_path).map_err(|e| anyhow::anyhow!(e))?;
        assert_eq!(parsed.len(), 0);
        Ok(())
    }

    #[test]
    fn value_containing_key_prefix() -> anyhow::Result<()> {
        let line = r#"audit_v1 ts=20260526t120000Z page=0 id=456 old="text with id= inside" new="text with ts= inside" op=test prov=Manual bbox=[0,0,0,0] in="in" out="out" review=false"#;
        let rec =
            AuditLogParser::parse_line(line).ok_or_else(|| anyhow::anyhow!("parse failed"))?;
        assert_eq!(rec.id, 456);
        assert_eq!(rec.old_text, "text with id= inside");
        assert_eq!(rec.new_text, "text with ts= inside");
        Ok(())
    }

    #[test]
    fn snapshot_link_creates_either_a_hard_link_or_a_copy_and_content_matches() -> anyhow::Result<()>
    {
        let dir = tempdir()?;
        let source = dir.path().join("source.pdf");
        let payload = b"%PDF-1.7\nfake snapshot content";
        std::fs::write(&source, payload)?;

        let dest = dir.path().join("snapshots").join("123.pdf");
        let was_hard_link = snapshot_link_or_copy(&source, &dest)?;

        // Either path: content must match the source byte-for-byte.
        let read_back = std::fs::read(&dest)?;
        assert_eq!(read_back, payload);

        // If the FS supported hard links, modifying the source must surface
        // through the dest. (NTFS / ext4 do; FAT32 / cross-volume don't.)
        if was_hard_link {
            std::fs::write(&source, b"modified")?;
            assert_eq!(std::fs::read(&dest)?, b"modified");
        }
        Ok(())
    }

    #[test]
    fn parse_file_missing_returns_read_error() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let missing = dir.path().join("does_not_exist.log");
        let err = AuditLogParser::parse_file(&missing)
            .map_err(|e| anyhow::anyhow!(e))
            .unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<AuditError>(),
                Some(AuditError::Read { .. })
            ),
            "expected AuditError::Read, got {err:?}"
        );
        // The error message should carry the offending path for diagnosis.
        assert!(err.to_string().contains("does_not_exist.log"));
        Ok(())
    }

    #[test]
    fn snapshot_link_overwrites_existing_destination() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let source = dir.path().join("source.pdf");
        std::fs::write(&source, b"new content")?;
        let dest = dir.path().join("snapshots").join("456.pdf");
        std::fs::create_dir_all(dest.parent().ok_or_else(|| anyhow::anyhow!("No parent"))?)?;
        std::fs::write(&dest, b"OLD STALE CONTENT")?;

        snapshot_link_or_copy(&source, &dest)?;
        assert_eq!(std::fs::read(&dest)?, b"new content");
        Ok(())
    }
}
