#!/bin/bash
# Builds "File Flier.app" for macOS (universal: Apple silicon + Intel) and packages it
# as dist/File-Flier-macos.zip (used by the in-app updater) and dist/File-Flier.dmg.
#
# Signing and notarization happen when these environment variables are set (GitHub
# secrets in the release workflow); otherwise the app is ad-hoc signed, which runs
# after right-click → Open the first time.
#   MACOS_CERTIFICATE           base64 of a "Developer ID Application" .p12
#   MACOS_CERTIFICATE_PASSWORD  its password
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD   for notarization (app-specific password)
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP="dist/File Flier.app"
rm -rf dist && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

for target in aarch64-apple-darwin x86_64-apple-darwin; do
    rustup target add "$target" >/dev/null
    MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release --locked --target "$target"
done
lipo -create -output "$APP/Contents/MacOS/file-flier" \
    target/aarch64-apple-darwin/release/file-flier target/x86_64-apple-darwin/release/file-flier
sed "s/__VERSION__/$VERSION/g" assets/macos/Info.plist > "$APP/Contents/Info.plist"
cp assets/macos/FileFlier.icns "$APP/Contents/Resources/"

IDENTITY="-"
if [ -n "${MACOS_CERTIFICATE:-}" ]; then
    KEYCHAIN="$RUNNER_TEMP/build.keychain-db"
    KEYCHAIN_PASSWORD=$(uuidgen)
    echo "$MACOS_CERTIFICATE" | base64 --decode > "$RUNNER_TEMP/cert.p12"
    security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
    security set-keychain-settings -lut 21600 "$KEYCHAIN"
    security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
    security import "$RUNNER_TEMP/cert.p12" -k "$KEYCHAIN" -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
    security set-key-partition-list -S apple-tool:,apple: -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
    security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')
    IDENTITY=$(security find-identity -v -p codesigning "$KEYCHAIN" | sed -n 's/.*"\(Developer ID Application:.*\)"/\1/p' | head -1)
    echo "Signing as: $IDENTITY"
    codesign --force --options runtime --timestamp --entitlements assets/macos/entitlements.plist \
        --sign "$IDENTITY" "$APP"
else
    echo "No signing certificate: ad-hoc signing"
    codesign --force --sign - "$APP"
fi
codesign --verify --deep --strict "$APP"

# Notarizes $1; returns 1 (skipped) without a signing identity or Apple ID.
notarize() {
    [ -n "${APPLE_ID:-}" ] && [ "$IDENTITY" != "-" ] || return 1
    xcrun notarytool submit "$1" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
        --password "$APPLE_APP_PASSWORD" --wait
}

# Zip for the updater (notarized, then the ticket is stapled to the app and it's zipped again).
ditto -c -k --keepParent "$APP" dist/File-Flier-macos.zip
if notarize dist/File-Flier-macos.zip; then
    xcrun stapler staple "$APP"
    rm dist/File-Flier-macos.zip
    ditto -c -k --keepParent "$APP" dist/File-Flier-macos.zip
fi

# Disk image with the usual drag-to-Applications layout.
mkdir -p dist/dmg
cp -R "$APP" dist/dmg/
ln -s /Applications dist/dmg/Applications
hdiutil create -volname "File Flier" -srcfolder dist/dmg -ov -format UDZO dist/File-Flier.dmg
rm -rf dist/dmg
if [ "$IDENTITY" != "-" ]; then
    codesign --force --timestamp --sign "$IDENTITY" dist/File-Flier.dmg
    if notarize dist/File-Flier.dmg; then
        xcrun stapler staple dist/File-Flier.dmg
    fi
fi
ls -la dist
