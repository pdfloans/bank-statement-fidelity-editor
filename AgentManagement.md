# AGENTS.md

## Project type

Rust desktop/CLI project using Cargo.

The project may include GUI, CLI, Python bridge, PDF processing, AI-service integrations, tests, scripts, and CI configuration.

## Autonomy level

The agent is allowed to operate with high autonomy for development, debugging, repair, and validation tasks.

The agent may:

- inspect the repository
- inspect build/test/runtime logs
- modify source files
- modify project configuration
- modify tests
- modify scripts
- modify CI workflows
- run terminal commands needed for diagnosis and validation
- install or select the required Rust toolchain
- run formatting, linting, build, and test commands
- repeat the diagnose → fix → validate loop until the issue is resolved or blocked

The agent should prefer durable project-level fixes over local temporary workarounds.

## Preferred Rust toolchain

Use Rust 1.88.0 unless a task explicitly requires another version.

If `rust-toolchain.toml` contains:

```toml
channel = "dev"
```

replace it with:

```toml
[toolchain]
channel = "1.88.0"
components = ["rustfmt", "clippy"]
```

If the required toolchain is missing, run:

```bash
rustup install 1.88.0
rustup override unset
```

Then retry the original command.

## Files the agent may modify automatically

The agent may automatically modify:

- `rust-toolchain.toml`
- `rustfmt.toml`
- `.cargo/config.toml`
- `Cargo.toml`
- `Cargo.lock`
- Rust source files under `src/`
- Rust tests under `tests/`
- examples under `examples/`
- scripts under `scripts/`
- Python support code under `python/`
- documentation files such as `README.md`, `QUICKSTART.md`, `CONTRIBUTING.md`, and `CHANGELOG.md`
- CI files under `.github/workflows/`
- Docker and deployment configuration files, when the task is clearly about build/deployment repair
- `.env.example`

## Files requiring explicit confirmation

The agent must ask before modifying:

- `.env`
- files containing private keys
- files containing tokens or credentials
- production deployment secrets
- private user PDFs
- real customer or banking data
- generated audit/history files
- generated output PDFs
- large generated output directories
- Git history
- remote repository state

## Terminal permissions

The agent may run non-destructive terminal commands needed for development and validation.

Allowed examples:

```bash
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
```

The agent may run project health checks such as:

```bash
cargo run -- doctor
cargo run -- verify-api-keys
```

only when doing so does not expose secrets in output.

## Commands requiring confirmation

The agent must ask before running destructive, external, or production-impacting commands.

Requires confirmation:

```bash
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
```

Also requires confirmation:

- uploading files
- deleting generated PDFs
- deleting logs/audit files
- modifying real bank statements
- running commands that may spend API credits heavily
- running commands that may expose secrets in logs

## Secrets policy

The agent must never print, copy, rewrite, or commit secrets.

The agent may check whether an environment variable exists, but must not reveal its value.

Allowed:

```text
GEMINI_API_KEY is set
PDFREST_API_KEY is missing
```

Forbidden:

```text
GEMINI_API_KEY=actual-secret-value
```

The agent may read `.env.example`.

The agent must not read or modify `.env` unless explicitly instructed.

If a required variable is missing, update `.env.example` or documentation instead of inventing a value.

## Standard validation commands

After code or configuration changes, run the narrowest useful validation first.

For Rust build fixes:

```bash
cargo check
```

For tests:

```bash
cargo test
```

For linting:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

For formatting:

```bash
cargo fmt
```

For full validation:

```bash
cargo fmt
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

If full validation is too slow, run targeted checks first and report what was skipped.

## Runtime debugging strategy

When given a console error, the agent should:

1. Read the exact error text.
2. Identify the failure category:
   - Rust toolchain/setup
   - Cargo dependency
   - compile error
   - test failure
   - runtime panic
   - missing configuration
   - external API failure
   - filesystem permission problem
   - Python bridge problem
   - PDF engine problem
3. Inspect only the files relevant to that category.
4. Make the smallest durable fix.
5. Re-run the failing command.
6. Continue until:
   - the issue is fixed, or
   - the remaining problem requires secrets, external services, private files, or confirmation.

## Known setup issue: toolchain `dev` is not installed

Console error:

```text
error: toolchain 'dev' is not installed
```

Cause:

The project or local Rustup override is trying to use a Rust toolchain named `dev`.

Fix:

1. Check `rust-toolchain.toml`.
2. Replace `channel = "dev"` with `channel = "1.88.0"`.
3. Run:

```bash
rustup install 1.88.0
rustup override unset
cargo check
```

## Dependency strategy

The agent should avoid adding new dependencies unless necessary.

Before adding a dependency, the agent should check whether the project already has a suitable dependency.

If adding a dependency is necessary, prefer stable, widely used crates and explain why.

## Error-handling strategy

The agent should avoid hiding errors with broad fallbacks.

Prefer:

- typed errors
- useful context with `anyhow` or project error types
- clear user-facing messages
- validation before expensive operations
- fail-safe behavior for documents and generated outputs

Avoid:

- silent failures
- swallowing errors
- unchecked unwraps in production paths
- temporary duct-tape fixes

## Reporting format

At the end of each autonomous fix session, summarize:

- root cause
- files changed
- commands run
- validation result
- remaining manual steps, if any
```
