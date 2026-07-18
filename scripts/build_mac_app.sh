#!/bin/bash
set -e

APP_NAME="BankStatementFidelityEditor"
APP_DIR="target/release/mac_app/$APP_NAME.app"
BIN_NAME="dual-core-pdf-pipeline"

echo "Building release binary..."
cargo build --release

echo "Creating Mac App bundle structure..."
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

echo "Copying binary and runtime assets..."
cp "target/release/$BIN_NAME" "$APP_DIR/Contents/MacOS/$BIN_NAME"

# Copy essential runtime assets into Resources
if [ -f .env ]; then
    cp .env "$APP_DIR/Contents/Resources/"
else
    cp .env.example "$APP_DIR/Contents/Resources/.env"
fi

cp libpdfium.dylib "$APP_DIR/Contents/MacOS/" || true

# Copy necessary directories
cp -r models "$APP_DIR/Contents/Resources/" 2>/dev/null || true
cp -r scripts "$APP_DIR/Contents/Resources/" 2>/dev/null || true

# Set rpath and adjust dynamic library path natively so macOS handles it
install_name_tool -change "libpdfium.dylib" "@executable_path/libpdfium.dylib" "$APP_DIR/Contents/MacOS/$BIN_NAME"

echo "Generating Info.plist..."
cat << 'PLIST_EOF' > "$APP_DIR/Contents/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>dual-core-pdf-pipeline</string>
    <key>CFBundleIdentifier</key>
    <string>com.dualcore.pdfpipeline</string>
    <key>CFBundleName</key>
    <string>BankStatementFidelityEditor</string>
    <key>CFBundleVersion</key>
    <string>1.0.0</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>10.13.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST_EOF

echo "Applying Code Signature to bypass Gatekeeper..."
codesign --force --deep --sign - "$APP_DIR"

echo "Done! App bundle is located at: $APP_DIR"
