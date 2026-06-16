
The robust fix is to **avoid embedding fenced code blocks inside the heredoc content at all**. I'll write `AGENTS.md` using indented examples (4 spaces) instead of triple-backtick fences. That way nothing inside the heredoc collides with Markdown fences, and the script itself renders cleanly.

Save this as `setup_agent_autonomy.sh` in your project root and run:

```bash
bash setup_agent_autonomy.sh
```

```bash
#!/usr/bin/env bash
set -u

echo "== Agent autonomy setup =="

if [ ! -f "Cargo.toml" ]; then
  echo "ERROR: Cargo.toml not found. Run this from the project root."
  exit 1
fi

mkdir -p scripts .github/workflows

# --- rust-toolchain.toml ---
cat > rust-toolchain.toml <<'EOF'
[toolchain]
channel = "1.88.0"
components = ["rustfmt", "clippy"]
EOF

# --- AGENTS.md (no backtick fences inside; uses 4-space indented examples) ---
cat > AGENTS.md <<'EOF'
# AGENTS.md

## Project type

Rust desktop/CLI project using Cargo.
May include GUI, CLI, Python bridge, PDF processing, AI integrations, tests, scripts, and CI.

## Autonomy level

The agent may operate with high autonomy for development, debugging, repair, and validation.

The agent may:

- inspect the repository
- inspect build/test/runtime logs
- modify source files, configuration, tests, scripts, and CI workflows
- run terminal commands needed for diagnosis and validation
- install or select the required Rust toolchain
- run formatting, linting, build, and test commands
- repeat the diagnose -> fix -> validate loop until resolved or blocked

Prefer durable project-level fixes over local temporary workarounds.

## Preferred Rust toolchain

Use Rust 1.88.0 unless a task explicitly requires another version.

If rust-toolchain.toml contains channel = "dev", replace it with:

    [toolchain]
    channel = "1.88.0"
    components = ["rustfmt", "clippy"]

If the toolchain is missing, run:

    rustup install 1.88.0
    rustup override unset

Then retry the original command.

## Files the agent may modify automatically

- rust-toolchain.toml
- rustfmt.toml
- .cargo/config.toml
- Cargo.toml
- Cargo.lock
- Rust source files under src/
- Rust tests under tests/
- examples under examples/
- scripts under scripts/
- Python support code under python/
- docs: README.md, QUICKSTART.md, CONTRIBUTING.md, CHANGELOG.md
- CI files under .github/workflows/
- Docker/deployment config when the task is about build/deploy repair
- .env.example

## Files requiring explicit confirmation

- .env
- private keys, tokens, credentials
- production deployment secrets
- private user PDFs
- real customer or banking data
- generated audit/history files
- generated output PDFs
- large generated output directories
- Git history
- remote repository state

## Terminal permissions

Allowed (non-destructive):

    cargo check
    cargo build
    cargo build --release
    cargo test
    cargo clippy --all-targets --all-features -- -D warnings
    cargo fmt
    rustup install 1.88.0
    rustup override unset
    rustc --version
    cargo --version
    python --version
    pip --version
    git status
    git diff

Health checks (only if they do not expose secrets):

    cargo run -- doctor
    cargo run -- verify-api-keys

## Commands requiring confirmation

    git reset --hard
    git clean -fdx
    git push
    git commit
    git rebase
    cargo publish
    docker push
    railway up
    railway deploy
    rm -rf
    Remove-Item -Recurse -Force

Also requires confirmation:

- uploading files
- deleting generated PDFs, logs, or audit files
- modifying real bank statements
- commands that may spend significant API credits
- commands that may expose secrets in logs

## Secrets policy

Never print, copy, rewrite, or commit secrets.
May report whether a variable is set, but never its value.

Allowed:

    GEMINI_API_KEY is set
    PDFREST_API_KEY is missing

Forbidden:

    GEMINI_API_KEY=actual-secret-value

May read .env.example. Must not read or modify .env unless explicitly instructed.
If a variable is missing, update .env.example or docs instead of inventing a value.

## Standard validation commands

Run the narrowest useful check first.

    cargo check
    cargo test
    cargo clippy --all-targets --all-features -- -D warnings
    cargo fmt

Full validation:

    cargo fmt
    cargo check
    cargo test
    cargo clippy --all-targets --all-features -- -D warnings

If full validation is too slow, run targeted checks and report what was skipped.

## Runtime debugging strategy

1. Read the exact error text.
2. Identify the failure category:
   toolchain/setup, dependency, compile, test, runtime panic,
   missing configuration, external API, filesystem permissions,
   Python bridge, or PDF engine.
3. Inspect only relevant files.
4. Make the smallest durable fix.
5. Re-run the failing command.
6. Continue until fixed or blocked by secrets/external services/private files/confirmation.

## Known setup issue: toolchain 'dev' is not installed

Cause: project or local rustup override targets a toolchain named "dev".

Fix:

1. Open rust-toolchain.toml.
2. Replace channel = "dev" with channel = "1.88.0".
3. Run:

    rustup install 1.88.0
    rustup override unset
    cargo check

## Error-handling strategy

Prefer typed errors, useful context, clear messages, validation before expensive work,
and fail-safe behavior for documents/outputs.

Avoid silent failures, swallowed errors, unchecked unwraps in production paths,
and temporary duct-tape fixes.

## Reporting format

At the end of each session, summarize:

- root cause
- files changed
- commands run
- validation result
- remaining manual steps, if any
EOF

# --- AGENTS_Overview.txt (plain pointer) ---
cat > AGENTS_Overview.txt <<'EOF'
Agent instructions have moved to AGENTS.md.

Use AGENTS.md as the source of truth for autonomous fix policy, terminal
permissions, validation commands, Rust toolchain rules, safety boundaries,
secrets policy, and reporting format.
EOF

# --- scripts/doctor.sh ---
cat > scripts/doctor.sh <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

echo "== Project Doctor =="

if [ ! -f "Cargo.toml" ]; then
  echo "ERROR: Cargo.toml not found. Run from the project root."
  exit 1
fi

cat > rust-toolchain.toml <<'TOOLCHAIN'
[toolchain]
channel = "1.88.0"
components = ["rustfmt", "clippy"]
TOOLCHAIN

if command -v rustup >/dev/null 2>&1; then
  rustup install 1.88.0
  rustup override unset || true
else
  echo "WARNING: rustup not found. Install from https://rustup.rs/"
fi

command -v rustc >/dev/null 2>&1 && rustc --version
command -v cargo >/dev/null 2>&1 && cargo --version

if command -v cargo >/dev/null 2>&1; then
  cargo check
else
  echo "WARNING: cargo not found. Skipping cargo check."
fi

echo "Project doctor completed."
EOF
chmod +x scripts/doctor.sh

# --- scripts/doctor.ps1 ---
cat > scripts/doctor.ps1 <<'EOF'
$ErrorActionPreference = "Stop"
Write-Host "== Project Doctor =="

if (-not (Test-Path "Cargo.toml")) {
    Write-Host "ERROR: Cargo.toml not found. Run from the project root."
    exit 1
}

@'
[toolchain]
channel = "1.88.0"
components = ["rustfmt", "clippy"]
'@ | Set-Content "rust-toolchain.toml"

if (Get-Command rustup -ErrorAction SilentlyContinue) {
    rustup install 1.88.0
    rustup override unset
} else {
    Write-Host "WARNING: rustup not found. Install from https://rustup.rs/"
}

if (Get-Command rustc -ErrorAction SilentlyContinue) { rustc --version }
if (Get-Command cargo -ErrorAction SilentlyContinue) {
    cargo --version
    cargo check
} else {
    Write-Host "WARNING: cargo not found. Skipping cargo check."
}

Write-Host "Project doctor completed."
EOF

# --- .github/workflows/rust-ci.yml ---
cat > .github/workflows/rust-ci.yml <<'EOF'
name: Rust CI

on:
  push:
  pull_request:

jobs:
  check:
    name: Check, test, and lint
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install Rust 1.88.0
        uses: dtolnay/rust-toolchain@1.88.0
        with:
          components: rustfmt, clippy
      - name: Show toolchain
        run: |
          rustc --version
          cargo --version
      - name: Format check
        run: cargo fmt --all -- --check
      - name: Cargo check
        run: cargo check
      - name: Tests
        run: cargo test
      - name: Clippy
        run: cargo clippy --all-targets --all-features -- -D warnings
EOF

# --- Optional: install toolchain and run cargo check ---
if command -v rustup >/dev/null 2>&1; then
  rustup install 1.88.0
  rustup override unset || true
else
  echo "WARNING: rustup not found. Install from https://rustup.rs/"
fi

if command -v cargo >/dev/null 2>&1; then
  cargo check
else
  echo "WARNING: cargo not found. Skipping cargo check."
fi

echo
echo "== Done =="
echo "Created/updated:"
echo "  AGENTS.md"
echo "  AGENTS_Overview.txt"
echo "  rust-toolchain.toml"
echo "  scripts/doctor.sh"
echo "  scripts/doctor.ps1"
echo "  .github/workflows/rust-ci.yml"
```

## Why this version is correct

The earlier output broke because the file content written by the script contained triple-backtick code fences, and when that whole script was shown inside one Markdown block, the inner fences closed the block early.

This version fixes it by:

- writing `AGENTS.md` with **4-space indented examples** instead of ```` ``` ```` fences, so there are no nested fences anywhere in the script, and
- keeping the entire script inside a single clean code block.

## One cleanup note

You now have several duplicate/partial agent files in the project:

- `AgentDevelopmentGuide.md`
- `AgentManagement.md`
- `agents_documentation.txt`
- `agents_documentation2.txt`
- `AGENTS_Overview.txt`
- plus several `*Toolchain*` / `*toolchain*` config text files

After running the script, I recommend keeping only:

- `AGENTS.md` (the real policy)
- `AGENTS_Overview.txt` (the pointer)
- `rust-toolchain.toml`

and deleting the rest to avoid agents reading stale or malformed instructions.
