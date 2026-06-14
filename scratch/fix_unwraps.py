import os
import re
import glob

def fix_tests_in_file(filepath):
    with open(filepath, 'r', encoding='utf-8') as f:
        content = f.read()

    # Find test functions
    # #[test]
    # fn something() {
    # We want to change it to:
    # #[test]
    # fn something() -> anyhow::Result<()> {
    
    lines = content.split('\n')
    out_lines = []
    
    in_test = False
    brace_depth = 0
    test_start_line = -1
    
    i = 0
    while i < len(lines):
        line = lines[i]
        
        # If we see #[test] and the next line is a fn
        if line.strip() == '#[test]':
            out_lines.append(line)
            i += 1
            if i < len(lines):
                fn_line = lines[i]
                if 'fn ' in fn_line and '()' in fn_line and '{' in fn_line:
                    if '->' not in fn_line:
                        fn_line = fn_line.replace('{', '-> anyhow::Result<()> {')
                        in_test = True
                        brace_depth = fn_line.count('{') - fn_line.count('}')
                    else:
                        # Already has a return type
                        pass
                out_lines.append(fn_line)
        else:
            if in_test:
                brace_depth += line.count('{')
                brace_depth -= line.count('}')
                
                # if we are closing the test function
                if brace_depth == 0 and '}' in line:
                    # Insert Ok(())
                    indent = line[:len(line) - len(line.lstrip())]
                    out_lines.append(indent + '    Ok(())')
                    out_lines.append(line)
                    in_test = False
                else:
                    out_lines.append(line)
            else:
                out_lines.append(line)
        i += 1
        
    content = '\n'.join(out_lines)
    
    # Replace .unwrap() with ?
    content = re.sub(r'\.unwrap\(\)', '?', content)
    
    # Replace .expect("...") with .map_err(|e| anyhow::anyhow!("..."))?
    # Actually just .context("...")? if anyhow is in scope. 
    # But anyhow::Context might not be in scope.
    # Let's use ok_or_else(|| anyhow::anyhow!("..."))? for Option and map_err for Result
    # Simple regex for expect
    content = re.sub(r'\.expect\((.*?)\)', r'.map_err(|e| anyhow::anyhow!(\1))?', content)
    
    with open(filepath, 'w', encoding='utf-8') as f:
        f.write(content)

for filepath in glob.glob('src/**/*.rs', recursive=True):
    with open(filepath, 'r', encoding='utf-8') as f:
        if '.unwrap()' in f.read() or '.expect(' in f.read():
            print(f"Fixing {filepath}")
            fix_tests_in_file(filepath)
