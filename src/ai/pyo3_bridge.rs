use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule, PyTuple};
use std::ffi::CString;

pub struct PyEngine {
    module: Py<PyModule>,
}

impl PyEngine {
    pub fn init() -> Result<Self, String> {
        // Safe to call multiple times, but we only call once in actor thread
        pyo3::prepare_freethreaded_python();

        let py_code = include_str!("../../python/pymupdf_pro_integration.py");

        Self::safe_python_with_gil(|py| {
            // Stage 11: ensure `python/` (where font_replicator.py lives) is
            // on sys.path so the integration module can `import font_replicator`.
            // We try in order: (1) the path baked in via PYO3_PYTHON_DIR env
            // var if set, (2) ./python relative to cwd, (3) the module's own
            // file path resolved at compile time. Each one's added only if
            // it actually exists.
            let sys = py.import("sys").map_err(|e| e.to_string())?;
            let path = sys.getattr("path").map_err(|e| e.to_string())?;
            let path_list = path
                .downcast::<pyo3::types::PyList>()
                .map_err(|e| e.to_string())?;
            let candidates: Vec<std::path::PathBuf> = [
                std::env::var("PYO3_PYTHON_DIR")
                    .ok()
                    .map(std::path::PathBuf::from),
                Some(std::path::PathBuf::from("python")),
                Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("python")),
            ]
            .into_iter()
            .flatten()
            .collect();
            for cand in candidates {
                if cand.is_dir() {
                    let s = cand.to_string_lossy().to_string();
                    let _ = path_list.insert(0, s);
                }
            }

            let module = PyModule::new(py, "pymupdf_pro_integration").map_err(|e| e.to_string())?;

            let c_code = CString::new(py_code).map_err(|e| e.to_string())?;
            py.run(&c_code, Some(&module.dict()), None)
                .map_err(|e| e.to_string())?;

            Ok(Self {
                module: module.into(),
            })
        })
    }

    /// Safely executes a Python closure. If called from within a Tokio async
    /// task, it wraps the execution in `tokio::task::block_in_place` to prevent
    /// thread starvation. If called from an OS thread, it runs directly.
    fn safe_python_with_gil<F, R>(f: F) -> R
    where
        F: FnOnce(Python<'_>) -> R,
    {
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::block_in_place(|| Python::with_gil(f))
        } else {
            Python::with_gil(f)
        }
    }

    fn call_json<'py, N>(&self, py: Python<'py>, fn_name: &str, args: N) -> Result<String, String>
    where
        N: IntoPyObject<'py, Target = PyTuple>,
    {
        let func = self
            .module
            .getattr(py, fn_name)
            .map_err(|e| e.to_string())?;
        let result = func.call1(py, args).map_err(|e| e.to_string())?;

        let json = py.import("json").map_err(|e| e.to_string())?;
        let dumps = json.getattr("dumps").map_err(|e| e.to_string())?;
        let json_str: String = dumps
            .call1((result,))
            .map_err(|e| e.to_string())?
            .extract()
            .map_err(|e| e.to_string())?;

        Ok(json_str)
    }

    pub fn get_text_blocks(&self, pdf_path: &str, page_num: usize) -> Result<String, String> {
        // The Python `get_text_blocks` enforces the PyMuPDF Pro <=3-page limit
        // before unlocking Pro; if the segment is over-limit it raises
        // RuntimeError("PRO_PAGE_LIMIT_EXCEEDED: ..."). `call_json` propagates
        // that Python exception message verbatim as Err(String), so the stable
        // PRO_PAGE_LIMIT_EXCEEDED token reaches the runtime unchanged.
        Self::safe_python_with_gil(|py| self.call_json(py, "get_text_blocks", (pdf_path, page_num)))
    }

    pub fn replace_text_in_rect(
        &self,
        pdf_path: &str,
        output_path: &str,
        page_num: usize,
        rect: [f32; 4],
        new_text: &str,
        font_path: Option<&str>,
    ) -> Result<Option<String>, String> {
        Self::safe_python_with_gil(|py| {
            let bg_func = self
                .module
                .getattr(py, "analyze_background")
                .map_err(|e| e.to_string())?;
            let bg_result = bg_func
                .call1(py, (pdf_path, page_num, rect.to_vec()))
                .map_err(|e| e.to_string())?;

            let (is_simple, avg_color): (bool, (f32, f32, f32)) =
                bg_result.extract(py).map_err(|e| e.to_string())?;

            let mut warning: Option<String> = None;
            if !is_simple {
                warning = Some("Complex background detected in region. Replaced with dominant color, but visual review is required.".to_string());
            }

            let fill_color = avg_color;

            let func = self
                .module
                .getattr(py, "replace_text_in_rect")
                .map_err(|e| e.to_string())?;
            let kwargs = PyDict::new(py);
            kwargs
                .set_item("pdf_path", pdf_path)
                .map_err(|e| e.to_string())?;
            kwargs
                .set_item("output_path", output_path)
                .map_err(|e| e.to_string())?;
            kwargs
                .set_item("page_num", page_num)
                .map_err(|e| e.to_string())?;
            kwargs
                .set_item("rect", rect.to_vec())
                .map_err(|e| e.to_string())?;
            kwargs
                .set_item("new_text", new_text)
                .map_err(|e| e.to_string())?;
            kwargs
                .set_item("fill_color", fill_color)
                .map_err(|e| e.to_string())?;
            if let Some(fp) = font_path {
                kwargs
                    .set_item("font_path", fp)
                    .map_err(|e| e.to_string())?;
            }

            // The Python function returns a dict on success and raises
            // ValueError(json.dumps({error: "FONT_COVERAGE_INSUFFICIENT", missing_chars: [...]}))
            // when the embedded font subset can't render the new text. We
            // surface that as a structured error string so the runtime can
            // decide whether to invoke deep font replication.
            let result = func.call(py, (), Some(&kwargs));
            match result {
                Ok(obj) => {
                    // Read .get("method") for a friendly suffix in the warning.
                    if let Ok(method) = obj
                        .getattr(py, "get")
                        .and_then(|g| g.call1(py, ("method",)))
                        .and_then(|m| m.extract::<String>(py))
                    {
                        if method == "embedded-fallback" {
                            warning.get_or_insert_with(|| {
                                "Embedded font reuse failed; falling back to default placement."
                                    .to_string()
                            });
                        }
                    }
                    Ok(warning)
                }
                Err(e) => {
                    // Capture the Python exception value (which is a JSON string for our
                    // structured failures) and propagate it.
                    let msg = e.to_string();
                    Err(
                        if msg.contains("FONT_COVERAGE_INSUFFICIENT")
                            || msg.contains("PDF_NOT_EDITABLE")
                            || msg.contains("PRO_PAGE_LIMIT_EXCEEDED")
                        {
                            // Already structured (incl. the PyMuPDF Pro 3-page
                            // limit token); pass through unchanged.
                            msg
                        } else {
                            format!("PyMuPDF replace failed: {msg}")
                        },
                    )
                }
            }
        })
    }

    pub fn find_text_block_at_click(
        &self,
        pdf_path: &str,
        page_num: usize,
        x: f32,
        y: f32,
    ) -> Result<String, String> {
        Self::safe_python_with_gil(|py| {
            self.call_json(
                py,
                "find_text_block_at_click",
                (pdf_path, page_num, x, y, 72.0),
            )
        })
    }

    pub fn get_all_transactions(&self, pdf_path: &str) -> Result<String, String> {
        Self::safe_python_with_gil(|py| self.call_json(py, "get_all_transactions", (pdf_path,)))
    }

    pub fn analyze_document_layout(&self, pdf_path: &str) -> Result<String, String> {
        Self::safe_python_with_gil(|py| self.call_json(py, "analyze_document_layout", (pdf_path,)))
    }

    pub fn extract_font(
        &self,
        pdf_path: &str,
        output_path: &str,
    ) -> Result<String, String> {
        Self::safe_python_with_gil(|py| {
            self.call_json(
                py,
                "extract_font",
                (pdf_path, output_path),
            )
        })
    }

    pub fn complete_font_with_adaption(
        &self,
        pdf_path: &str,
        font_name: &str,
    ) -> Result<String, String> {
        Self::safe_python_with_gil(|py| {
            self.call_json(
                py,
                "complete_font_with_adaption_fallback",
                (pdf_path, font_name),
            )
        })
    }

    pub fn deep_font_replication(
        &self,
        pdf_path: &str,
        font_name: &str,
        output_dir: &str,
    ) -> Result<String, String> {
        Self::safe_python_with_gil(|py| {
            // We use the same pattern as other calls, but we need to make sure
            // the python side is ready for it.
            // Actually, my python bridge uses 'command' in __main__.
            // If I want to use call_json, I need to add a function in the python script.
            self.call_json(
                py,
                "deep_font_replication_api",
                (pdf_path, font_name, output_dir),
            )
        })
    }

    /// Apply many targeted edits in a single open/save pass. See
    /// `python/pymupdf_pro_integration.py::apply_many_edits`.
    /// `edits_json` is a JSON array of `{page, rect, new_text, fill_color?}`.
    /// Returns the JSON dict the Python function returned, or a structured
    /// error string on `FONT_COVERAGE_INSUFFICIENT`.
    pub fn apply_many_edits(
        &self,
        pdf_path: &str,
        output_path: &str,
        edits_json: &str,
        font_path: Option<&str>,
    ) -> Result<String, String> {
        Self::safe_python_with_gil(|py| {
            let json_mod = py.import("json").map_err(|e| e.to_string())?;
            let loads = json_mod.getattr("loads").map_err(|e| e.to_string())?;
            let edits_obj = loads.call1((edits_json,)).map_err(|e| e.to_string())?;

            let func = self
                .module
                .getattr(py, "apply_many_edits")
                .map_err(|e| e.to_string())?;
            let kwargs = PyDict::new(py);
            kwargs
                .set_item("pdf_path", pdf_path)
                .map_err(|e| e.to_string())?;
            kwargs
                .set_item("output_path", output_path)
                .map_err(|e| e.to_string())?;
            kwargs
                .set_item("edits", edits_obj)
                .map_err(|e| e.to_string())?;
            if let Some(fp) = font_path {
                kwargs
                    .set_item("font_path", fp)
                    .map_err(|e| e.to_string())?;
            }

            let result = func.call(py, (), Some(&kwargs));
            match result {
                Ok(obj) => {
                    let dumps = json_mod.getattr("dumps").map_err(|e| e.to_string())?;
                    let s: String = dumps
                        .call1((obj,))
                        .map_err(|e| e.to_string())?
                        .extract()
                        .map_err(|e| e.to_string())?;
                    Ok(s)
                }
                Err(e) => {
                    let msg = e.to_string();
                    Err(
                        if msg.contains("FONT_COVERAGE_INSUFFICIENT")
                            || msg.contains("PDF_NOT_EDITABLE")
                            || msg.contains("PRO_PAGE_LIMIT_EXCEEDED")
                        {
                            // Already structured (incl. the PyMuPDF Pro 3-page
                            // limit token from `_assert_within_pro_page_limit`);
                            // pass the message through unchanged so the runtime
                            // can match on the stable error token.
                            msg
                        } else {
                            format!("PyMuPDF apply_many_edits failed: {msg}")
                        },
                    )
                }
            }
        })
    }

    /// Split a PDF into chunks for Document AI. See
    /// `python/pymupdf_pro_integration.py::chunk_pdf_for_docai`.
    /// Returns the JSON list of `{path, page_offset, page_count}`.
    pub fn chunk_pdf_for_docai(
        &self,
        pdf_path: &str,
        output_dir: &str,
        max_pages_per_chunk: usize,
    ) -> Result<String, String> {
        Self::safe_python_with_gil(|py| {
            self.call_json(
                py,
                "chunk_pdf_for_docai",
                (pdf_path, output_dir, max_pages_per_chunk),
            )
        })
    }

    /// Stage 8.5: per-font usage and coverage analysis. See
    /// `python/pymupdf_pro_integration.py::analyze_fonts`.
    /// Returns the JSON shape documented there.
    pub fn analyze_fonts(&self, pdf_path: &str) -> Result<String, String> {
        Self::safe_python_with_gil(|py| self.call_json(py, "analyze_fonts", (pdf_path,)))
    }

    /// Stage 11: targeted font cascade.
    ///
    /// Calls `python/pymupdf_pro_integration.py::replicate_font_for_missing_chars`
    /// which delegates to `font_replicator.replicate_font_for_chars`. The
    /// cascade tries composite synthesis, donor-based subset extension,
    /// and Gemini Vision typeface ID in order.
    ///
    /// Returns the JSON dict produced by the cascade. On `success: true`
    /// the dict's `extended_font_path` points at a TTF/OTF the editor can
    /// pass back as `font_path` for the next apply attempt.
    pub fn replicate_font_for_missing_chars(
        &self,
        pdf_path: &str,
        font_name: &str,
        missing_chars_csv: &str,
        output_dir: &str,
    ) -> Result<String, String> {
        Self::safe_python_with_gil(|py| {
            self.call_json(
                py,
                "replicate_font_for_missing_chars",
                (pdf_path, font_name, missing_chars_csv, output_dir),
            )
        })
    }

    pub fn clone_pages(
        &self,
        pdf_path: &str,
        output_path: &str,
        page_indices: &[usize],
    ) -> Result<String, String> {
        Self::safe_python_with_gil(|py| {
            self.call_json(
                py,
                "clone_pages",
                (pdf_path, output_path, page_indices.to_vec()),
            )
        })
    }

    pub fn render_page_to_png(
        &self,
        pdf_path: &str,
        page_num: usize,
        dpi: f32,
    ) -> Result<String, String> {
        Self::safe_python_with_gil(|py| {
            self.call_json(py, "render_page_to_png", (pdf_path, page_num, dpi))
        })
    }

    pub fn remove_pages(
        &self,
        pdf_path: &str,
        output_path: &str,
        page_indices: &[usize],
    ) -> Result<String, String> {
        Self::safe_python_with_gil(|py| {
            self.call_json(
                py,
                "remove_pages",
                (pdf_path, output_path, page_indices.to_vec()),
            )
        })
    }

    /// Force Python garbage collection.
    /// Stage 2 Memory Management: explicit collection to prevent OOM in batch processing.
    pub fn garbage_collect() {
        if let Err(e) = Python::with_gil(|py| py.run(c"import gc; gc.collect()", None, None)) {
            tracing::warn!("Failed to run Python GC: {}", e);
        }
    }
}
