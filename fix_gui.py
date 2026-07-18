import re

with open("src/app/gui.rs", "r") as f:
    content = f.read()

content = re.sub(r'\s+pub edit_mindee_api_key: String,', '', content)
content = re.sub(r'\s+pub edit_mindee_model_id: String,', '', content)
content = re.sub(r'\s+edit_mindee_api_key: std::env::var\("MINDEE_API_KEY"\)\.unwrap_or_default\(\),', '', content)
content = re.sub(r'\s+edit_mindee_model_id: std::env::var\("MINDEE_MODEL_ID"\)\.unwrap_or_default\(\),', '', content)
content = re.sub(r'\s+"MINDEE_API_KEY",\s+self\.edit_mindee_api_key\.trim\(\)\.to_string\(\),', '', content)

with open("src/app/gui.rs", "w") as f:
    f.write(content)
