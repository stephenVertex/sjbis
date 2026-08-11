#!/usr/bin/env bash
# scripts/ios-deploy.sh — build, sign, and install the SJBIS iOS app onto a
# connected iPhone via devicectl, without ever opening Xcode.
#
# Requires (one-time): Xcode signed into your Apple ID, a paid Apple Developer
# Program, and an Apple Development certificate in your login keychain.
#
# Usage:
#   ./scripts/ios-deploy.sh                    # auto-detect connected device
#   ./scripts/ios-deploy.sh --device <UDID>    # specify device UDID
#   ./scripts/ios-deploy.sh --sim              # build for simulator only

set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
IOS_DIR="$PROJECT_DIR/ios"
PROJECT="Sjbis.xcodeproj"
SCHEME="Sjbis"
BUILD_DIR="$IOS_DIR/build"

DEVICE_UDID=""
SIMULATOR_ONLY=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --device) DEVICE_UDID="$2"; shift 2 ;;
    --sim) SIMULATOR_ONLY=true; shift ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

step() { printf "\n\033[1;36m▸ %s\033[0m\n" "$*"; }
fail() { printf "\n\033[1;31m✗ %s\033[0m\n" "$*" >&2; exit 1; }

step "Pre-flight"
command -v xcodegen >/dev/null || fail "xcodegen not installed (brew install xcodegen)."
[ -f "$IOS_DIR/Local.xcconfig" ] || fail "Local.xcconfig missing in ios/. Copy Local.xcconfig.example and set your team ID."

cd "$IOS_DIR"

step "Regenerating project"
xcodegen generate

rm -rf "$BUILD_DIR"; mkdir -p "$BUILD_DIR"

if [ "$SIMULATOR_ONLY" = true ]; then
  step "Building for iOS Simulator"
  xcodebuild -project "$PROJECT" -scheme "$SCHEME" \
    -destination 'generic/platform=iOS Simulator' \
    -derivedDataPath "$BUILD_DIR/derived" \
    -configuration Debug \
    CODE_SIGNING_ALLOWED=NO build
  printf "\n\033[1;32m✓ Built for simulator.\033[0m\n"
  exit 0
fi

step "Finding connected device"
if [ -z "$DEVICE_UDID" ]; then
  DEVICE_UDID=$(xcrun devicectl list devices --json-output - 2>/dev/null | \
    python3 -c "
import sys, json
data = json.load(sys.stdin)
for d in data.get('result', {}).get('devices', []):
    props = d.get('connectionProperties', {})
    if props.get('transportType') == 'wired' or props.get('isPaired'):
        print(d.get('hardwareProperties', {}).get('udid', ''))
        break
" 2>/dev/null || true)
fi
[ -n "$DEVICE_UDID" ] || fail "No connected device found. Connect an iPhone or pass --device <UDID>."
echo "  Device: $DEVICE_UDID"

step "Building (Release) for device"
xcodebuild -project "$PROJECT" -scheme "$SCHEME" \
  -destination "id=$DEVICE_UDID" \
  -configuration Release \
  -derivedDataPath "$BUILD_DIR/derived" \
  -allowProvisioningUpdates \
  CODE_SIGN_STYLE=Automatic build

APP_PATH="$BUILD_DIR/derived/Build/Products/Release-iphoneos/Sjbis.app"
[ -d "$APP_PATH" ] || fail "Built app not found at $APP_PATH."

step "Installing to device"
xcrun devicectl device install app --device "$DEVICE_UDID" "$APP_PATH"

printf "\n\033[1;32m✓ Sjbis installed on device %s.\033[0m\n" "$DEVICE_UDID"
