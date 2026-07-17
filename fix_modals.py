import sys

def main():
    with open("src/app/modals.rs", "r") as f:
        code = f.read()

    # Fix AppModals trait
    trait_target = """    fn draw_ai_confirmation_dialog(&mut self, ctx: &egui::Context);
    fn draw_workflow_hitl_modal(&mut self, ctx: &egui::Context);"""
    trait_replacement = """    fn draw_ai_confirmation_dialog(&mut self, ctx: &egui::Context);
    fn draw_interactive_fallback_modal(&mut self, ctx: &egui::Context);
    fn draw_workflow_hitl_modal(&mut self, ctx: &egui::Context);"""
    code = code.replace(trait_target, trait_replacement)
    
    # Fix iter flattening
    iter_target = """                            if let Ok(iter) = dotenvy::from_path_iter(&path) {
                                for item in iter {
                                    if let Ok((key, val)) = item {
                                        match key.as_str() {"""
    iter_replacement = """                            if let Ok(iter) = dotenvy::from_path_iter(&path) {
                                for (key, val) in iter.flatten() {
                                        match key.as_str() {"""
    code = code.replace(iter_target, iter_replacement)

    # Need to remove the extra closing brace for iter fix
    # find exactly this block
    brace_target = """                                            _ => {}
                                        }
                                    }
                                }
                                self.toast(ToastKind::Success, "Imported keys from file");"""
    brace_replacement = """                                            _ => {}
                                        }
                                }
                                self.toast(ToastKind::Success, "Imported keys from file");"""
    code = code.replace(brace_target, brace_replacement)

    with open("src/app/modals.rs", "w") as f:
        f.write(code)

if __name__ == "__main__":
    main()
