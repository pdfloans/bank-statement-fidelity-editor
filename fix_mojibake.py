import os
import glob
import re

def fix_file(filepath):
    with open(filepath, 'r', encoding='utf-8') as f:
        content = f.read()

    # Find sequences of characters that are likely mojibake
    # We can try to regex match words containing â or ð
    # Actually, we can just find all strings in quotes and try to fix them?
    
    replacements = {
        "ðŸ“„": "📄",
        "ðŸ”‘": "🔑",
        "ðŸ“¥": "📥",
        "ðŸ“...": "📅", # wait, what is ðŸ“...? U+1F4C5 is 📅 (f0 9f 93 85 -> ðŸ“…)
        "ðŸŸ¢": "🟢",
        "ðŸ”µ": "🔵",
        "ðŸ“Š": "📊",
        "ðŸ“ˆ": "📈",
        "ðŸ”„": "🔄",
        "ðŸ” ": "🔍",
        "ðŸ“¤": "📤",
        "ðŸ“œ": "📜",
        "ðŸ“¦": "📦",
        "ðŸ” ": "🔠",
        "ðŸ¤–": "🤖",
        "ðŸ§ª": "🧪",
        "ðŸ“‹": "📋",
        "ðŸ”¤": "🔤",
        "ðŸ”§": "🔧",
        "ðŸ“‚": "📂",
        "ðŸ” -": "🔍-",
        "ðŸ” +": "🔍+",
        "ðŸŽ¯": "🎯",
        "ðŸ“‘": "📑",
        "â‡„": "⇄",
        "âš™": "⚙",
        "âš™ï¸ ": "⚙️",
        "â Œ": "❌",
        "âœ“": "✓",
        "âœ-": "✗",
        "âœ...": "✅",
        "â‰¡": "≡",
        "âš¡": "⚡",
        "âš ": "⚠",
        "â–¶": "▶",
        "âš–": "⚖",
        "âŒ¨": "⌨",
        "âœ•": "✕",
        "âž•": "➕",
        "â†¶": "↶",
        "â–º": "►",
        "â‘¡": "②",
        "â ¯": "⏯",
        "â€¢": "•"
    }

    new_content = content
    for k, v in replacements.items():
        new_content = new_content.replace(k, v)
        
    # Also handle the dates one: "ðŸ“..." might be ðŸ“… (U+1F4C5 📅)
    new_content = new_content.replace("ðŸ“…", "📅")

    if new_content != content:
        with open(filepath, 'w', encoding='utf-8') as f:
            f.write(new_content)
        print(f"Fixed {filepath}")

for root, _, files in os.walk('src'):
    for file in files:
        if file.endswith('.rs'):
            fix_file(os.path.join(root, file))

for root, _, files in os.walk('tests'):
    for file in files:
        if file.endswith('.rs'):
            fix_file(os.path.join(root, file))
