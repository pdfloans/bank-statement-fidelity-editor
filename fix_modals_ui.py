import sys

def main():
    with open("src/app/modals.rs", "r") as f:
        code = f.read()

    target = "                            ui.checkbox(&mut self.settings.interactive_fallbacks, \"Pause & prompt on semi-failures (Interactive Fallback)\")"
    replacement = """                            ui.checkbox(&mut self.settings.interactive_fallbacks, "Pause & prompt on semi-failures (Interactive Fallback)")
                                .on_hover_text("If a background process encounters a recoverable error, pause and ask for your input.");
                                
                            ui.checkbox(&mut self.settings.transfer_consensus_mode, "Multi-AI Transfer Consensus Mode")
                                .on_hover_text("Use multiple AI parsers concurrently and cross-reference them to guarantee zero-hallucination layout mapping.");
                                
                            ui.checkbox(&mut self.settings.auto_match_dpi, "Auto-Match DPI to Document")
                                .on_hover_text("Automatically calculate safe DPI based on physical document dimensions (max 600 DPI)");"""
    
    code = code.replace(target, replacement)
    
    with open("src/app/modals.rs", "w") as f:
        f.write(code)

if __name__ == "__main__":
    main()
