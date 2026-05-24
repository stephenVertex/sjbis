#!/bin/bash
set -e

echo "Building SJBIS iMessage Plugin .app bundle..."

PLUGIN_DIR="$(cd "$(dirname "$0")" && pwd)"
BUNDLE_DIR="$PLUGIN_DIR/bundle"
APP_DIR="$BUNDLE_DIR/sjbis-imessage.app"
MACOS_DIR="$APP_DIR/Contents/MacOS"

cd "$PLUGIN_DIR"

# Build the release binary
cargo build --release

# Ensure bundle structure exists
mkdir -p "$MACOS_DIR"

# Copy binary into bundle
cp "$PLUGIN_DIR/target/release/sjbis-imessage" "$MACOS_DIR/"

# Ensure binary is executable
chmod +x "$MACOS_DIR/sjbis-imessage"

echo ""
echo "Bundle created at: $APP_DIR"
echo ""
echo "NEXT STEPS:"
echo "1. Grant Full Disk Access to the bundle:"
echo "   System Settings → Privacy & Security → Full Disk Access"
echo "   Click '+' and select: $APP_DIR"
echo ""
echo "2. Run the test:"
echo "   $APP_DIR/Contents/MacOS/sjbis-imessage test --minutes 60"
echo ""
