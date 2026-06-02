#!/bin/bash
set -e

echo "=========================================================="
echo " MAXIMUM TESTING: CASCADING BALANCE EDITS (DOMINO EFFECT)"
echo "=========================================================="
echo ""
echo "Setting up environment..."
export PYTHONPATH="$(pwd)/.venv/lib/python3.14/site-packages"

echo "Building dual-core-pdf-pipeline..."
cargo build --release

INPUT_PDF="AU Bank Statements/Westpac ChoiceBasic-1-15.pdf"
OUTPUT_PDF="output/Westpac ChoiceBasic-1-15_edited.pdf"

echo ""
echo "Starting Smart Balance Engine..."
echo "Input: $INPUT_PDF"
echo "Output: $OUTPUT_PDF"
echo ""

# The user must have DOCUMENT_AI credentials in .env or system environment
if [ -f .env ]; then
    set -a
    source <(tr -d '\r' < .env)
    set +a
fi

target/release/dual-core-pdf-pipeline balance -i "$INPUT_PDF" -o "$OUTPUT_PDF" --auto-approve

echo ""
echo "=========================================================="
echo " TEST COMPLETE"
echo "=========================================================="
