use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyModule};

pub struct PyEngine {
    module: Py<PyModule>,
}

impl PyEngine {
    pub fn init() -> Result<Self, String> {
        // Safe to call multiple times, but we only call once in actor thread
        pyo3::prepare_freethreaded_python();
        
        let py_code = include_str!("../../python/pymupdf_pro_integration.py");
        
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "pymupdf_pro_integration")
                .map_err(|e| e.to_string())?;
            
            py.run_bound(py_code, Some(&module.dict()), None)
                .map_err(|e| e.to_string())?;
            
            Ok(Self {
                module: module.into(),
            })
        })
    }

    fn call_json(&self, py: Python, fn_name: &str, args: impl IntoPy<Py<PyTuple>>) -> Result<String, String> {
        let func = self.module.getattr(py, fn_name).map_err(|e| e.to_string())?;
        let result = func.call1(py, args).map_err(|e| e.to_string())?;
        
        let json = py.import_bound("json").map_err(|e| e.to_string())?;
        let dumps = json.getattr("dumps").map_err(|e| e.to_string())?;
        let json_str: String = dumps.call1((result,)).map_err(|e| e.to_string())?.extract().map_err(|e| e.to_string())?;
        
        Ok(json_str)
    }

    pub fn get_text_blocks(&self, pdf_path: &str, page_num: usize) -> Result<String, String> {
        Python::with_gil(|py| {
            self.call_json(py, "get_text_blocks", (pdf_path, page_num))
        })
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
        Python::with_gil(|py| {
            let bg_func = self.module.getattr(py, "analyze_background").map_err(|e| e.to_string())?;
            let bg_result = bg_func.call1(py, (pdf_path, page_num, rect.to_vec())).map_err(|e| e.to_string())?;
            
            let (is_simple, avg_color): (bool, (f32, f32, f32)) = bg_result.extract(py).map_err(|e| e.to_string())?;
            
            let warning = if !is_simple {
                Some("Complex background detected in region. Replaced with dominant color, but visual review is required.".to_string())
            } else {
                None
            };
            
            let fill_color = avg_color;

            let func = self.module.getattr(py, "replace_text_in_rect").map_err(|e| e.to_string())?;
            let kwargs = PyDict::new_bound(py);
            kwargs.set_item("pdf_path", pdf_path).map_err(|e| e.to_string())?;
            kwargs.set_item("output_path", output_path).map_err(|e| e.to_string())?;
            kwargs.set_item("page_num", page_num).map_err(|e| e.to_string())?;
            kwargs.set_item("rect", rect.to_vec()).map_err(|e| e.to_string())?;
            kwargs.set_item("new_text", new_text).map_err(|e| e.to_string())?;
            kwargs.set_item("fill_color", fill_color).map_err(|e| e.to_string())?;
            if let Some(fp) = font_path {
                kwargs.set_item("font_path", fp).map_err(|e| e.to_string())?;
            }
            
            func.call_bound(py, (), Some(&kwargs)).map_err(|e| e.to_string())?;
            
            Ok(warning)
        })
    }

    pub fn find_text_block_at_click(&self, pdf_path: &str, page_num: usize, x: f32, y: f32) -> Result<String, String> {
        Python::with_gil(|py| {
            self.call_json(py, "find_text_block_at_click", (pdf_path, page_num, x, y, 72.0))
        })
    }

    pub fn get_all_transactions(&self, pdf_path: &str) -> Result<String, String> {
        Python::with_gil(|py| {
            self.call_json(py, "get_all_transactions", (pdf_path,))
        })
    }

    pub fn analyze_document_layout(&self, pdf_path: &str) -> Result<String, String> {
        Python::with_gil(|py| {
            self.call_json(py, "analyze_document_layout", (pdf_path,))
        })
    }

    pub fn complete_font_with_adaption(&self, pdf_path: &str, font_name: &str) -> Result<String, String> {
        Python::with_gil(|py| {
            self.call_json(py, "complete_font_with_adaption_fallback", (pdf_path, font_name))
        })
    }

    pub fn deep_font_replication(&self, pdf_path: &str, font_name: &str, output_dir: &str) -> Result<String, String> {
        Python::with_gil(|py| {
            // We use the same pattern as other calls, but we need to make sure 
            // the python side is ready for it.
            // Actually, my python bridge uses 'command' in __main__. 
            // If I want to use call_json, I need to add a function in the python script.
            self.call_json(py, "deep_font_replication_api", (pdf_path, font_name, output_dir))
        })
    }
}
