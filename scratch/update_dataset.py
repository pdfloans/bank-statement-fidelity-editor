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
processor_id = "773734d4360df8c"

# Initialize Dataset using GCS-managed storage
url = f"https://us-documentai.googleapis.com/v1beta3/projects/1006635704194/locations/us/processors/{processor_id}/dataset"
payload = {
    "name": f"projects/1006635704194/locations/us/processors/{processor_id}/dataset",
    "gcsManagedConfig": {
         "gcsPrefix": {
             "gcsUriPrefix": "gs://docai-training-1006635704194/dataset_storage/"
         }
     }
}

req = urllib.request.Request(
    url, 
    data=json.dumps(payload).encode('utf-8'),
    headers={
        'Authorization': f'Bearer {token}', 
        'Content-Type': 'application/json; charset=utf-8',
        'X-Goog-User-Project': 'project-c8e3ae09-df5e-4bb3-8cd'
    },
    method='PATCH'
)

print(f"Sending request to {url}...")
try:
    with urllib.request.urlopen(req) as response:
        print("Status:", response.status)
        print("Response:", response.read().decode('utf-8'))
except urllib.error.HTTPError as e:
    print("HTTP Error:", e.code)
    print("Error body:", e.read().decode('utf-8'))
    # If it fails, maybe DocumentWarehouseConfig isn't right, fallback to gcsManagedConfig
    print("Trying gcsManagedConfig...")
    payload = {
        "name": f"projects/1006635704194/locations/us/processors/{processor_id}/dataset",
        "gcsManagedConfig": {
             "gcsPrefix": {
                 "gcsUriPrefix": "gs://docai-training-1006635704194/au_custom_dataset/"
             }
         }
    }
    req = urllib.request.Request(
        url, data=json.dumps(payload).encode('utf-8'),
        headers={'Authorization': f'Bearer {token}', 'Content-Type': 'application/json'},
        method='PATCH'
    )
    try:
        with urllib.request.urlopen(req) as response2:
            print("Status:", response2.status)
            print("Response:", response2.read().decode('utf-8'))
    except urllib.error.HTTPError as e2:
        print("HTTP Error 2:", e2.code)
        print("Error body 2:", e2.read().decode('utf-8'))
        sys.exit(1)
except Exception as e:
    print("Error:", str(e))
    sys.exit(1)
