#!/bin/sh
# Builds the Liquid key helper into Liquid.app as a login item (ADR-0205). Runs as an Xcode
# build phase of the Runner target, before Xcode signs the app around it.
set -eu
here="$PROJECT_DIR/LiquidKey"
app="$TARGET_BUILD_DIR/$CONTENTS_FOLDER_PATH/Library/LoginItems/LiquidKey.app"
work="$DERIVED_FILE_DIR/LiquidKey"
rm -rf "$app" "$work"
mkdir -p "$app/Contents/MacOS" "$work"

set --
for arch in $ARCHS; do
    xcrun --sdk macosx swiftc -O -swift-version 5 \
        -target "$arch-apple-macos$MACOSX_DEPLOYMENT_TARGET" \
        -o "$work/LiquidKey-$arch" "$here/Mapping.swift" "$here/main.swift"
    set -- "$@" "$work/LiquidKey-$arch"
done
lipo -create "$@" -output "$app/Contents/MacOS/LiquidKey"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key>
	<string>LiquidKey</string>
	<key>CFBundleIdentifier</key>
	<string>$PRODUCT_BUNDLE_IDENTIFIER.key</string>
	<key>CFBundleName</key>
	<string>Liquid Key</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>LSBackgroundOnly</key>
	<true/>
	<key>LSMinimumSystemVersion</key>
	<string>$MACOSX_DEPLOYMENT_TARGET</string>
</dict>
</plist>
PLIST

if [ "${CODE_SIGNING_ALLOWED:-YES}" = YES ]; then
    identity="${EXPANDED_CODE_SIGN_IDENTITY:--}"
    set -- --force --sign "$identity"
    if [ "${ENABLE_HARDENED_RUNTIME:-NO}" = YES ]; then set -- "$@" --options runtime; fi
    if [ "$identity" != - ]; then set -- "$@" --timestamp; fi
    # Xcode's extra flags, such as a keychain, apply here too; they are words by design.
    # shellcheck disable=SC2086
    codesign "$@" ${OTHER_CODE_SIGN_FLAGS:-} "$app"
fi
