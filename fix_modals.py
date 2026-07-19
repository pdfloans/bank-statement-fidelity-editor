import re

files = [
    'src/app/gui.rs',
    'src/app/modals.rs',
]

for file in files:
    with open(file, 'r') as f:
        content = f.read()

    # Fix (self.active_modal == ActiveModal::XYZ) = open;
    def replace_assign_open(match):
        modal = match.group(1)
        return f"if open {{ self.active_modal = ActiveModal::{modal}; }} else if self.active_modal == ActiveModal::{modal} {{ self.active_modal = ActiveModal::None; }}"

    content = re.sub(r'\(self\.active_modal == ActiveModal::(\w+)\)\s*=\s*open;', replace_assign_open, content)
    
    # Fix (self.active_modal == ActiveModal::XYZ) = false;
    def replace_assign_false(match):
        modal = match.group(1)
        return f"if self.active_modal == ActiveModal::{modal} {{ self.active_modal = ActiveModal::None; }}"
        
    content = re.sub(r'\(self\.active_modal == ActiveModal::(\w+)\)\s*=\s*false;', replace_assign_false, content)
    
    # Fix (self.active_modal == ActiveModal::XYZ) = true;
    def replace_assign_true(match):
        modal = match.group(1)
        return f"self.active_modal = ActiveModal::{modal};"
        
    content = re.sub(r'\(self\.active_modal == ActiveModal::(\w+)\)\s*=\s*true;', replace_assign_true, content)

    # Missing ActiveModal enum imports in modals.rs
    if file == 'src/app/modals.rs' and 'use crate::app::gui::ActiveModal;' not in content:
        content = content.replace('use crate::app::gui::AppView;', 'use crate::app::gui::{AppView, ActiveModal};')
        # If it doesn't exist, we might need to find something else
        if 'use crate::app::gui::{AppView, ActiveModal};' not in content:
            content = content.replace('use crate::app::gui::', 'use crate::app::gui::{ActiveModal, ')
    
    with open(file, 'w') as f:
        f.write(content)

print("Fixed")
