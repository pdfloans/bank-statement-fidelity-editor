import os
import requests
from dotenv import load_dotenv

load_dotenv()
api_key = os.getenv("MINDEE_API_KEY")

if not api_key:
    print("NO KEY")
    exit(1)

headers = {"Authorization": f"Token {api_key}"}

# Test 1: ping synchronous financial_document
url = "https://api.mindee.net/v1/products/mindee/financial_document/v1/predict"
resp = requests.post(url, headers=headers)
print(f"Financial Doc Sync: {resp.status_code}")

# Test 2: ping async financial_document
url = "https://api.mindee.net/v1/products/mindee/financial_document/v1/predict_async"
resp = requests.post(url, headers=headers)
print(f"Financial Doc Async: {resp.status_code}")

# Test 3: ping async bank_statement?
url = "https://api.mindee.net/v1/products/mindee/bank_statement/v1/predict_async"
resp = requests.post(url, headers=headers)
print(f"Bank Statement Async: {resp.status_code}")
