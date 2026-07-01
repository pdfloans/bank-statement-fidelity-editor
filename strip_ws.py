import os

def clean_file(filepath):
    with open(filepath, 'r', encoding='utf-8') as f:
        lines = f.readlines()
    
    cleaned_lines = [line.rstrip() + '\n' for line in lines]
    
    with open(filepath, 'w', encoding='utf-8', newline='\n') as f:
        f.writelines(cleaned_lines)
    print(f"Cleaned {filepath}")

for root, _, files in os.walk('src'):
    for file in files:
        if file.endswith('.rs'):
            clean_file(os.path.join(root, file))

for root, _, files in os.walk('tests'):
    for file in files:
        if file.endswith('.rs'):
            clean_file(os.path.join(root, file))
