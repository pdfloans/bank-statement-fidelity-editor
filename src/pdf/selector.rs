use super::engine::*;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Cache key that ties an entry to a specific file revision: when the file's
/// modified-time changes, the key changes and the stale entry is naturally
/// evicted/missed rather than served.
type RenderKey = (String, usize, u64, u64);
type BlocksKey = (String, usize, u64);

/// Recommendation #2/#7 — small, bounded LRU caches so re-navigating to a
/// page (preview) or re-reading its text blocks (preview/edit/verify) doesn't
/// re-rasterise or re-parse the same page every time.
struct EngineCaches {
    rendered: Mutex<LruCache<RenderKey, RenderedPage>>,
    blocks: Mutex<LruCache<BlocksKey, Vec<TextBlock>>>,
}

impl EngineCaches {
    fn new() -> Self {
        Self {
            // ~24 pages of rendered PNGs and 256 page-block lists is plenty
            // for snappy navigation without unbounded memory growth.
            rendered: Mutex::new(LruCache::new(NonZeroUsize::new(24).unwrap())),
            blocks: Mutex::new(LruCache::new(NonZeroUsize::new(256).unwrap())),
        }
    }
}

/// File modified-time (in nanoseconds since the epoch) used as a cheap
/// revision token. Returns 0 when unavailable so caching still works for
/// paths whose mtime can't be read (it just can't auto-invalidate them).
fn file_revision(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[derive(Clone)]
pub struct PdfEngineSelector {
    primary: Arc<dyn PdfEngine>,
    fallback: Arc<dyn PdfEngine>,
    config: Arc<std::sync::Mutex<Arc<crate::app::config::AppConfig>>>,
    caches: Arc<EngineCaches>,
}

impl std::fmt::Debug for PdfEngineSelector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PdfEngineSelector").finish_non_exhaustive()
    }
}

impl PdfEngineSelector {
    pub fn new(
        primary: Arc<dyn PdfEngine>,
        fallback: Arc<dyn PdfEngine>,
        config: Arc<std::sync::Mutex<Arc<crate::app::config::AppConfig>>>,
    ) -> Self {
        Self {
            primary,
            fallback,
            config,
            caches: Arc::new(EngineCaches::new()),
        }
    }

    fn try_primary_or_fallback<T, F>(&self, operation: F) -> Result<T, EngineError>
    where
        F: Fn(&dyn PdfEngine) -> Result<T, EngineError>,
    {
        let mode = if let Ok(guard) = self.config.try_lock() {
            guard.engine_mode
        } else {
            crate::app::config::PdfEngineMode::Auto
        };

        match mode {
            crate::app::config::PdfEngineMode::NativeOnly => operation(&*self.primary),
            crate::app::config::PdfEngineMode::PyMuPdfOnly => operation(&*self.fallback),
            crate::app::config::PdfEngineMode::Auto => {
                match operation(&*self.primary) {
                    Ok(result) => Ok(result),
                    Err(EngineError::Unsupported) => {
                        tracing::warn!(
                            engine.fallback_triggered = true,
                            primary_error = "Unsupported",
                            "Primary engine unsupported, falling back"
                        );
                        operation(&*self.fallback)
                    }
                    Err(e) => {
                        tracing::warn!(
                            engine.fallback_triggered = true,
                            primary_error = %e,
                            "Primary engine failed, falling back"
                        );
                        operation(&*self.fallback)
                    }
                }
            }
        }
    }
}

impl PdfEngine for PdfEngineSelector {
    fn capabilities(&self) -> EngineCapabilities {
        let p_cap = self.primary.capabilities();
        let f_cap = self.fallback.capabilities();
        EngineCapabilities {
            supports_redaction: p_cap.supports_redaction || f_cap.supports_redaction,
            supports_cjk: p_cap.supports_cjk || f_cap.supports_cjk,
            supports_embedded_fonts: p_cap.supports_embedded_fonts || f_cap.supports_embedded_fonts,
            estimated_fidelity: p_cap.estimated_fidelity.max(f_cap.estimated_fidelity),
        }
    }

    fn render_page(&self, path: &Path, page: usize, dpi: f32) -> Result<RenderedPage, EngineError> {
        // Recommendation #2 — serve repeated previews of the same page from
        // the LRU cache; `dpi.to_bits()` + file revision keep the key exact.
        let key: RenderKey = (
            path.to_string_lossy().to_string(),
            page,
            dpi.to_bits() as u64,
            file_revision(path),
        );
        if let Ok(mut cache) = self.caches.rendered.lock() {
            if let Some(hit) = cache.get(&key) {
                return Ok(hit.clone());
            }
        }
        let rendered = self.try_primary_or_fallback(|engine| engine.render_page(path, page, dpi))?;
        if let Ok(mut cache) = self.caches.rendered.lock() {
            cache.put(key, rendered.clone());
        }
        Ok(rendered)
    }

    fn get_text_blocks(&self, path: &Path, page: usize) -> Result<Vec<TextBlock>, EngineError> {
        // Recommendation #7 — memoise per-page text blocks; invalidated by the
        // file revision token so edits to the PDF are always re-parsed.
        let key: BlocksKey = (path.to_string_lossy().to_string(), page, file_revision(path));
        if let Ok(mut cache) = self.caches.blocks.lock() {
            if let Some(hit) = cache.get(&key) {
                return Ok(hit.clone());
            }
        }
        let blocks = self.try_primary_or_fallback(|engine| engine.get_text_blocks(path, page))?;
        if let Ok(mut cache) = self.caches.blocks.lock() {
            cache.put(key, blocks.clone());
        }
        Ok(blocks)
    }

    fn find_text_block_at_click(
        &self,
        path: &Path,
        page: usize,
        x: f32,
        y: f32,
    ) -> Result<Option<TextBlock>, EngineError> {
        self.try_primary_or_fallback(|engine| engine.find_text_block_at_click(path, page, x, y))
    }

    fn apply_change(
        &self,
        input: &Path,
        output: &Path,
        page: usize,
        bbox: [f32; 4],
        new_text: &str,
        old_text: &str,
        font_path: Option<&Path>,
    ) -> Result<ReplaceOutcome, EngineError> {
        self.try_primary_or_fallback(|engine| {
            engine.apply_change(input, output, page, bbox, new_text, old_text, font_path)
        })
    }

    fn analyze_layout(&self, path: &Path) -> Result<DocumentLayout, EngineError> {
        self.try_primary_or_fallback(|engine| engine.analyze_layout(path))
    }
}
