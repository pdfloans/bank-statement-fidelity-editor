import urllib.request
import urllib.error
import json
import subprocess
import sys

def get_token():
    result = subprocess.run(["gcloud", "auth", "application-default", "print-access-token"], capture_output=True, text=True, shell=True)
    if result.returncode != 0:
        print("Failed to get token:", result.stderr)
        sys.exit(1)
    return result.stdout.strip()

token = get_token()

# 1. Create the Processor
create_url = "https://us-documentai.googleapis.com/v1/projects/1006635704194/locations/us/processors"
create_payload = {
    "type": "CUSTOM_EXTRACTION_PROCESSOR",
    "displayName": "AU Bank Statements Extractor"
}

req = urllib.request.Request(
    create_url, 
    data=json.dumps(create_payload).encode('utf-8'),
    headers={
        'Authorization': f'Bearer {token}',
        'Content-Type': 'application/json; charset=utf-8'
    },
    method='POST'
)

print(f"Sending request to {create_url}...")
processor_id = None
try:
    with urllib.request.urlopen(req) as response:
        body = response.read().decode('utf-8')
        print("Status:", response.status)
        print("Response:", body)
        resp_json = json.loads(body)
        processor_name = resp_json.get("name")
        processor_id = processor_name.split("/")[-1]
        print(f"Created Processor ID: {processor_id}")
except urllib.error.HTTPError as e:
    print("HTTP Error:", e.code)
    print("Error body:", e.read().decode('utf-8'))
    sys.exit(1)
except Exception as e:
    print("Error:", str(e))
    sys.exit(1)

# Write the new processor_id to a file so we can use it in the next steps
with open("scratch/new_processor_id.txt", "w") as f:
    f.write(processor_id)

