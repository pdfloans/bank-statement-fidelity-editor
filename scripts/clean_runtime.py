"""Strip Python actor code from the Rust runtime, replacing match arms with stubs."""
import sys

with open('src/app/runtime.rs', encoding='utf-8') as f:
    lines = f.read().splitlines()

# Targets to completely replace with a stub
targets = {
    'Job::AiFixVisualFidelity {': 'ai_fidelity',
    'Job::TransferTransactions {': 'transfer',
    'Job::AdjustDatePeriods {': 'date_adjust',
    'Job::CompleteFont {': 'font_completion',
    'Job::BalanceStatement {': 'balance_engine',
    'Job::ApplyProposedChanges {': 'apply_edits',
    'Job::BalanceAndApplyAll {': 'balance_engine',
    'Job::AnalyzeFonts {': 'analyze_fonts',
    'Job::Python(': 'python_raw',
}

new_lines = []
skip_mode = False
skip_count = 0
current_label = ""

i = 0
while i < len(lines):
    line = lines[i]
    
    # Check if we should start skipping a match arm
    matched_target = None
    if not skip_mode:
        for t, label in targets.items():
            if line.strip().startswith(t):
                matched_target = label
                break
                
    if matched_target:
        skip_mode = True
        skip_count = 0
        current_label = matched_target
        # Output the start of the arm, but change the block
        arm_start = line.split('{')[0] + '{' if '{' in line else line.split('=>')[0] + '=> {'
        new_lines.append(arm_start)
        new_lines.append(f'                        let _ = result_tx_clone.send(JobResult::Error {{')
        new_lines.append(f'                            job_label: "{current_label}".to_string(),')
        new_lines.append(f'                            message: "Feature disabled — pending pure Rust rewrite".to_string(),')
        new_lines.append(f'                        }});')
        
    if skip_mode:
        skip_count += line.count('{')
        skip_count -= line.count('}')
        if skip_count <= 0:
            skip_mode = False
            new_lines.append('                    }')
    else:
        # Also remove python thread spawn
        if 'let _python_stub_thread = thread::spawn(' in line:
            skip_mode = True
            skip_count = line.count('{') - line.count('}')
            current_label = "THREAD"
            # Don't append anything
            new_lines.append('        // thread removed')
            if skip_count <= 0: skip_mode = False
        elif 'let (python_tx, python_rx)' in line:
            # Skip this line and the next if it's the channel
            i += 1
        elif 'let python_tx_clone = python_tx.clone();' in line:
            pass
        elif 'let _py_tx = python_tx_clone.clone();' in line:
            pass
        elif 'python_tx' in line or 'py_tx' in line:
            # Comment out any remaining lines with python_tx
            new_lines.append('// ' + line)
        elif 'pub enum PythonJob {' in line:
            skip_mode = True
            skip_count = line.count('{') - line.count('}')
            current_label = "ENUM"
            if skip_count <= 0: skip_mode = False
        elif 'pub enum PythonJobResult {' in line:
            skip_mode = True
            skip_count = line.count('{') - line.count('}')
            current_label = "ENUM2"
            if skip_count <= 0: skip_mode = False
        elif 'fn dispatch_python_job(' in line:
            skip_mode = True
            skip_count = line.count('{') - line.count('}')
            current_label = "FN"
            if skip_count <= 0: skip_mode = False
        elif line.strip() == 'Python(PythonJob, oneshot::Sender<PythonJobResult>),':
            pass
        elif line.strip() == '#[derive(Debug, Clone)]' and i+1 < len(lines) and 'pub enum PythonJob {' in lines[i+1]:
            pass
        elif line.strip() == '#[derive(Debug)]' and i+1 < len(lines) and 'pub enum PythonJobResult {' in lines[i+1]:
            pass
        else:
            new_lines.append(line)
            
    i += 1

with open('src/app/runtime.rs', 'w', encoding='utf-8') as f:
    f.write('\n'.join(new_lines))
print("Done")
