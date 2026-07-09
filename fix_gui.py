import re
with open(r'src\app\gui.rs', 'r', encoding='utf-8') as f:
    content = f.read()

pattern = re.compile(r'let _ = self\.job_tx\.send\((.*?)\);', re.DOTALL)

def replacer(match):
    inner = match.group(1)
    return f'if let Err(e) = self.job_tx.send({inner}) {{ tracing::error!("Runtime disconnected: {{}}", e); }}'

new_content = pattern.sub(replacer, content)

with open(r'src\app\gui.rs', 'w', encoding='utf-8') as f:
    f.write(new_content)
