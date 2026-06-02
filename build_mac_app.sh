#!/usr/bin/env bash
set -e

APP_NAME="BankStatementModifier"
APP_BUNDLE="${APP_NAME}.app"
CONTENTS_DIR="${APP_BUNDLE}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
FRAMEWORKS_DIR="${CONTENTS_DIR}/Frameworks"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"

# Determine architecture for python-build-standalone
ARCH=$(uname -m)
if [ "$ARCH" = "arm64" ]; then
    PYTHON_URL="https://github.com/indygreg/python-build-standalone/releases/download/20240224/cpython-3.11.8+20240224-aarch64-apple-darwin-install_only.tar.gz"
else
    PYTHON_URL="https://github.com/indygreg/python-build-standalone/releases/download/20240224/cpython-3.11.8+20240224-x86_64-apple-darwin-install_only.tar.gz"
fi

echo "🚀 Building ${APP_NAME} for Mac..."

# 1. Compile the Rust binary
echo "📦 Compiling Rust binary in release mode..."
cargo build --release

# 2. Create the .app structure
echo "📂 Creating .app directory structure..."
rm -rf "${APP_BUNDLE}"
mkdir -p "${MACOS_DIR}"
mkdir -p "${FRAMEWORKS_DIR}"
mkdir -p "${RESOURCES_DIR}"

# 3. Download and extract Python Standalone
echo "🐍 Downloading Python Standalone for ${ARCH}..."
curl -L -o python_standalone.tar.gz "$PYTHON_URL"

echo "🐍 Extracting Python..."
tar -xzf python_standalone.tar.gz
mv python "${FRAMEWORKS_DIR}/python"
rm python_standalone.tar.gz

# 4. Install dependencies (pymupdf) into the standalone python
echo "📦 Installing PyMuPDF into the standalone Python environment..."
# Get the path to the standalone python executable
PYTHON_EXE="${FRAMEWORKS_DIR}/python/bin/python3"
# Install pip if not present (usually present in install_only)
if ! "${PYTHON_EXE}" -m pip --version > /dev/null 2>&1; then
    echo "Pip not found, installing..."
    curl -sS https://bootstrap.pypa.io/get-pip.py | "${PYTHON_EXE}"
fi
"${PYTHON_EXE}" -m pip install --no-cache-dir pymupdf PyMuPDFPro

# 5. Copy the Rust binary
echo "⚙️ Copying executable..."
cp target/release/dual-core-pdf-pipeline "${MACOS_DIR}/${APP_NAME}"

# 6. Create Info.plist
echo "📝 Generating Info.plist..."
cat > "${CONTENTS_DIR}/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>${APP_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>com.dualcore.bankstatementmodifier</string>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleVersion</key>
    <string>1.0.0</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>10.15</string>
</dict>
</plist>
EOF

echo "✅ Build complete! You can now run or distribute ${APP_BUNDLE}."
