import urllib.request
import urllib.error
import json
import subprocess
import sys

def get_token():
    result = subprocess.run(["gcloud", "auth", "application-default", "print-access-token"], capture_output=True, text=True, shell=True)
    return result.stdout.strip()

token = get_token()
op_name = "projects/1006635704194/locations/us/operations/12501342148320633079"

url = f"https://us-documentai.googleapis.com/v1beta3/{op_name}"

req = urllib.request.Request(
    url, 
    headers={
        'Authorization': f'Bearer {token}', 
        'Content-Type': 'application/json; charset=utf-8',
        'X-Goog-User-Project': 'project-c8e3ae09-df5e-4bb3-8cd'
    },
    method='GET'
)

print(f"Sending request to {url}...")
try:
    with urllib.request.urlopen(req) as response:
        print("Response:", response.read().decode('utf-8'))
except urllib.error.HTTPError as e:
    print("HTTP Error:", e.code)
    print("Error body:", e.read().decode('utf-8'))
except Exception as e:
    print("Error:", str(e))
