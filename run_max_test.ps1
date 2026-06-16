$ErrorActionPreference = "Continue"

New-Item -ItemType Directory -Force -Path ".\out\max-test" | Out-Null
New-Item -ItemType Directory -Force -Path ".\out\max-test\render" | Out-Null
New-Item -ItemType Directory -Force -Path ".\out\max-test\verify" | Out-Null

cargo check *>&1 | Tee-Object ".\out\max-test\cargo_check.log"

cargo test --all --all-features *>&1 | Tee-Object ".\out\max-test\cargo_test_all_features.log"

cargo build --release *>&1 | Tee-Object ".\out\max-test\cargo_build_release.log"

$exe = ".\target\release\dual-core-pdf-pipeline.exe"

& $exe --help *>&1 | Tee-Object ".\out\max-test\help.log"

& $exe doctor *>&1 | Tee-Object ".\out\max-test\doctor.log"

& $exe verify-api-keys --json *>&1 | Tee-Object ".\out\max-test\verify_api_keys.json"

& $exe selftest --input ".\test_doc.pdf" *>&1 | Tee-Object ".\out\max-test\selftest.log"

& $exe render -i ".\test_doc.pdf" -o ".\out\max-test\render" -p 0 --dpi 150 *>&1 | Tee-Object ".\out\max-test\render.log"

& $exe extract -i ".\test_doc.pdf" -o ".\out\max-test\transactions.json" *>&1 | Tee-Object ".\out\max-test\extract.log"

& $exe balance -i ".\test_doc.pdf" -o ".\out\max-test\balanced_proposed.pdf" *>&1 | Tee-Object ".\out\max-test\balance.log"

& $exe auto-balance -i ".\test_doc.pdf" -o ".\out\max-test\balanced.pdf" *>&1 | Tee-Object ".\out\max-test\auto_balance.log"

if (Test-Path ".\out\max-test\balanced.pdf") {
    & $exe verify --original ".\test_doc.pdf" --edited ".\out\max-test\balanced.pdf" --output-dir ".\out\max-test\verify" *>&1 | Tee-Object ".\out\max-test\verify.log"
}

& $exe analyze-fonts -i ".\test_doc.pdf" *>&1 | Tee-Object ".\out\max-test\analyze_fonts.log"

& $exe adjust-dates -i ".\test_doc.pdf" -o ".\out\max-test\dates_adjusted.pdf" --mode shift-forward-1-month *>&1 | Tee-Object ".\out\max-test\adjust_dates.log"

if ((Test-Path ".\test_doc.pdf") -and (Test-Path ".\test_doc_edited.pdf")) {
    & $exe transfer-transactions --source-pdf ".\test_doc.pdf" --target-pdf ".\test_doc_edited.pdf" -o ".\out\max-test\transfer.pdf" *>&1 | Tee-Object ".\out\max-test\transfer_transactions.log"
}

# Instead of starting the GUI in background which could hang, let's omit the GUI test or just output success so far.
# Start-Process -FilePath $exe -ArgumentList "gui"
