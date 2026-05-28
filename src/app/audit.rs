//! Session Audit Logging and Snapshot Management
//! Supports financial compliance by tracking all manual and automated changes.

use crate::engine::history::ChangeRecord;
use chrono::Utc;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub struct AuditLog {
    log_file: Option<File>,
    log_path: PathBuf,
    _audit_dir: PathBuf,
    snapshots_dir: PathBuf,
}

impl AuditLog {
    /// Opens the audit log directory and initializes the current session's log file.
    pub fn open(audit_dir: impl AsRef<Path>) -> io::Result<Self> {
        let audit_dir = audit_dir.as_ref().to_path_buf();
        let snapshots_dir = audit_dir.join("snapshots");
        fs::create_dir_all(&snapshots_dir)?;

        // ISO-8601-utc-with-no-colons for Windows compatibility
        let timestamp = Utc::now().format("%Y%m%dt%H%M%SZ").to_string();
        let log_path = audit_dir.join(format!("{}.log", timestamp));

        Ok(Self {
            log_file: None,
            log_path,
            _audit_dir: audit_dir,
            snapshots_dir,
        })
    }

    /// Writes a change record to the persistent log file.
    pub fn write(
        &mut self,
        record: &ChangeRecord,
        source: &Path,
        output: &Path,
        operator: &str,
        requires_visual_review: bool,
    ) -> io::Result<()> {
        if self.log_file.is_none() {
            self.log_file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.log_path)?,
            );
        }

        let ts = Utc::now().to_rfc3339();
        let old_escaped = serde_json::to_string(&record.old_text).unwrap_or_default();
        let new_escaped = serde_json::to_string(&record.new_text).unwrap_or_default();
        let desc_escaped = serde_json::to_string(&record.description).unwrap_or_default();
        let snap_escaped = serde_json::to_string(
            &record
                .snapshot_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
        )
        .unwrap_or_default();

        let line = format!(
            "audit_v1 ts={} page={} id={} old={} new={} op={} prov={} desc={} snap={} bbox=[{},{},{},{}] in={:?} out={:?} review={}\n",
            ts, record.page, record.id, old_escaped, new_escaped, operator, record.provenance, desc_escaped, snap_escaped,
            record.bbox[0], record.bbox[1], record.bbox[2], record.bbox[3],
            source, output, requires_visual_review
        );

        let file = self.log_file.as_mut().unwrap();
        file.write_all(line.as_bytes())?;
        file.flush()?;
        Ok(())
    }

    pub fn snapshots_dir(&self) -> &Path {
        &self.snapshots_dir
    }

    /// Returns the path where a snapshot for a specific change ID should be stored.
    pub fn snapshot_path_for(&self, change_id: u64) -> PathBuf {
        // We use .pdf for snapshots per Approach §4.4
        self.snapshots_dir.join(format!("{}.pdf", change_id))
    }
}

/// Save a snapshot of `output` at the audit log's expected path for
/// `change_id`. Tries hard-linking first (~zero disk cost when the source
/// and snapshot live on the same volume), falls back to a full copy on
/// hard-link failure (cross-FS, FAT32, etc.).
///
/// Returns `Ok(true)` when the hard link succeeded, `Ok(false)` after a
/// fallback copy, and `Err` on total failure.
pub fn snapshot_link_or_copy(source: &Path, dest: &Path) -> std::io::Result<bool> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        // Hard linking onto an existing path errors; remove first.
        let _ = std::fs::remove_file(dest);
    }
    match std::fs::hard_link(source, dest) {
        Ok(()) => Ok(true),
        Err(e) => {
            tracing::debug!("[audit] hard_link failed ({}); falling back to copy", e);
            std::fs::copy(source, dest)?;
            Ok(false)
        }
    }
}

pub struct AuditLogParser;

impl AuditLogParser {
    pub fn parse_file(path: &Path) -> io::Result<Vec<ChangeRecord>> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();

        for line in reader.lines() {
            let line = line?;
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
        // audit_v1 ts=… page=… id=… old=… new=… op=… prov=… desc=… snap=… bbox=[…] in=… out=… review=…
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_records() {
        let dir = tempdir().unwrap();
        let mut audit = AuditLog::open(dir.path()).unwrap();

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
        };

        audit
            .write(&rec1, Path::new("in"), Path::new("out"), "test", false)
            .unwrap();

        let parsed = AuditLogParser::parse_file(&audit.log_path).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, 123);
        assert_eq!(parsed[0].old_text, "foo");
        assert_eq!(parsed[0].description, "Adjustment");
        assert_eq!(parsed[0].provenance, "DocumentAI");
        assert_eq!(
            parsed[0].snapshot_path,
            Some(PathBuf::from("audit/snapshots/123.pdf"))
        );
    }

    #[test]
    fn value_containing_key_prefix() {
        let line = r#"audit_v1 ts=20260526t120000Z page=0 id=456 old="text with id= inside" new="text with ts= inside" op=test prov=Manual bbox=[0,0,0,0] in="in" out="out" review=false"#;
        let rec = AuditLogParser::parse_line(line).unwrap();
        assert_eq!(rec.id, 456);
        assert_eq!(rec.old_text, "text with id= inside");
        assert_eq!(rec.new_text, "text with ts= inside");
    }

    #[test]
    fn snapshot_link_creates_either_a_hard_link_or_a_copy_and_content_matches() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.pdf");
        let payload = b"%PDF-1.7\nfake snapshot content";
        std::fs::write(&source, payload).unwrap();

        let dest = dir.path().join("snapshots").join("123.pdf");
        let was_hard_link = snapshot_link_or_copy(&source, &dest).unwrap();

        // Either path: content must match the source byte-for-byte.
        let read_back = std::fs::read(&dest).unwrap();
        assert_eq!(read_back, payload);

        // If the FS supported hard links, modifying the source must surface
        // through the dest. (NTFS / ext4 do; FAT32 / cross-volume don't.)
        if was_hard_link {
            std::fs::write(&source, b"modified").unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), b"modified");
        }
    }

    #[test]
    fn snapshot_link_overwrites_existing_destination() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.pdf");
        std::fs::write(&source, b"new content").unwrap();
        let dest = dir.path().join("snapshots").join("456.pdf");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"OLD STALE CONTENT").unwrap();

        snapshot_link_or_copy(&source, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new content");
    }
}
