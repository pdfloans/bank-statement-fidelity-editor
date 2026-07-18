import re

with open("src/app/runtime.rs", "r") as f:
    content = f.read()

# Remove Mindee logic from runtime
content = re.sub(r'\s+// 3\. Try Mindee.*?\}\s+\}', '', content, flags=re.DOTALL)
content = re.sub(r'req = req\.add_alternative\("mindee", "Try Mindee API", None\);', '', content)
content = re.sub(r'"mindee" => Some\(DocumentParserMode::MindeeFinDoc\),', '', content)

with open("src/app/runtime.rs", "w") as f:
    f.write(content)
