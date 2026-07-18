import logging
import os
from mindee import Client, PredictResponse, product

logging.basicConfig(level=logging.DEBUG)
mindee_client = Client(api_key=os.environ.get("MINDEE_API_KEY"))

with open(".gitignore", "rb") as f:
    try:
        response = mindee_client.enqueue_and_parse(product.BankStatementV1, f)
        print(response.document)
    except Exception as e:
        print(f"Error: {e}")
