import urllib.request
import urllib.error
import json
import subprocess
import sys

def get_token():
    result = subprocess.run(["gcloud", "auth", "application-default", "print-access-token"], capture_output=True, text=True, shell=True)
    return result.stdout.strip()

token = get_token()
processor_id = "773734d4360df8c"

url = f"https://us-documentai.googleapis.com/v1beta3/projects/1006635704194/locations/us/processors/{processor_id}/dataset:importDocuments"

payload = {
    "batchDocumentsImportConfigs": [
        {
            "datasetSplit": "DATASET_SPLIT_UNASSIGNED",
            "batchInputConfig": {
                "gcsPrefix": {
                    "gcsUriPrefix": "gs://docai-training-1006635704194/AU Bank Statements/"
                }
            }
        }
    ]
}

req = urllib.request.Request(
    url, 
    data=json.dumps(payload).encode('utf-8'),
    headers={
        'Authorization': f'Bearer {token}', 
        'Content-Type': 'application/json; charset=utf-8',
        'X-Goog-User-Project': 'project-c8e3ae09-df5e-4bb3-8cd'
    },
    method='POST'
)

print(f"Sending request to {url}...")
try:
    with urllib.request.urlopen(req) as response:
        print("Status:", response.status)
        print("Response:", response.read().decode('utf-8'))
except urllib.error.HTTPError as e:
    print("HTTP Error:", e.code)
    print("Error body:", e.read().decode('utf-8'))
except Exception as e:
    print("Error:", str(e))
