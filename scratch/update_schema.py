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

url = f"https://us-documentai.googleapis.com/v1beta3/projects/1056864635772/locations/us/processors/{processor_id}/dataset/datasetSchema"

payload = {
    "documentSchema": {
        "displayName": "AU Bank Statement Schema",
        "entityTypes": [
            {
                "name": "au_bank_statement",
                "baseTypes": ["document"],
                "properties": [
                    {
                        "name": "account_number",
                        "valueType": "string",
                        "occurrenceType": "OPTIONAL_ONCE"
                    },
                    {
                        "name": "opening_balance",
                        "valueType": "number",
                        "occurrenceType": "OPTIONAL_ONCE"
                    },
                    {
                        "name": "closing_balance",
                        "valueType": "number",
                        "occurrenceType": "OPTIONAL_ONCE"
                    },
                    {
                        "name": "transaction",
                        "valueType": "au_transaction_row",
                        "occurrenceType": "OPTIONAL_MULTIPLE"
                    }
                ]
            },
            {
                "name": "au_transaction_row",
                "properties": [
                    {
                        "name": "date",
                        "valueType": "datetime",
                        "occurrenceType": "OPTIONAL_ONCE"
                    },
                    {
                        "name": "description",
                        "valueType": "string",
                        "occurrenceType": "OPTIONAL_ONCE"
                    },
                    {
                        "name": "debit",
                        "valueType": "number",
                        "occurrenceType": "OPTIONAL_ONCE"
                    },
                    {
                        "name": "credit",
                        "valueType": "number",
                        "occurrenceType": "OPTIONAL_ONCE"
                    },
                    {
                        "name": "running_balance",
                        "valueType": "number",
                        "occurrenceType": "OPTIONAL_ONCE"
                    }
                ]
            }
        ]
    }
}

req = urllib.request.Request(
    url, 
    data=json.dumps(payload).encode('utf-8'),
    headers={'Authorization': f'Bearer {token}', 'Content-Type': 'application/json; charset=utf-8'},
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
except Exception as e:
    print("Error:", str(e))
