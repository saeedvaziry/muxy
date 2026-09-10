#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ARCH=""
VERSION=""
SIGN_IDENTITY="-"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch|--version|--sign-identity)
            if [[ $# -lt 2 || -z "$2" ]]; then
                echo "Error: $1 requires a value" >&2
                exit 1
            fi
            case "$1" in
                --arch) ARCH="$2" ;;
                --version) VERSION="$2" ;;
                --sign-identity) SIGN_IDENTITY="$2" ;;
            esac
            shift 2
            ;;
        *)
            echo "Usage: $0 --arch <arm64|x86_64> --version <2.0.0-alpha-N> [--sign-identity <identity>]" >&2
            exit 1
            ;;
    esac
done

case "$ARCH" in
    arm64) TARGET="aarch64-apple-darwin" ;;
    x86_64) TARGET="x86_64-apple-darwin" ;;
    *) echo "Error: arch must be arm64 or x86_64" >&2; exit 1 ;;
esac
python3 "$ROOT/scripts/alpha_release.py" check-version "$VERSION"
if [[ "$(uname -s)" != Darwin ]]; then
    echo "Error: packaging requires macOS and Xcode" >&2
    exit 1
fi

export MACOSX_DEPLOYMENT_TARGET=14.0
export LIBGHOSTTY_VT_SYS_OPTIMIZE=ReleaseFast
export CARGO_TARGET_DIR="$ROOT/target"
export CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO=packed
OUTPUT_DIR="$CARGO_TARGET_DIR/alpha/$VERSION/$ARCH"
if [[ -e "$OUTPUT_DIR" ]]; then
    echo "Error: output already exists: $OUTPUT_DIR" >&2
    exit 1
fi

cd "$ROOT"
echo "==> Building app and server for $TARGET"
cargo build --locked --release --target "$TARGET" -p muxy-app -p muxy-server
BIN_DIR="$CARGO_TARGET_DIR/$TARGET/release"

mkdir -p "$(dirname "$OUTPUT_DIR")"
STAGING="$(mktemp -d "$(dirname "$OUTPUT_DIR")/.${ARCH}.XXXXXX")"
trap 'rm -rf "$STAGING"' EXIT
APP="$STAGING/dmg/Muxy Alpha.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$STAGING/symbols" "$STAGING/artifacts"

for BINARY in muxy-app muxy-server; do
    install -m 755 "$BIN_DIR/$BINARY" "$APP/Contents/MacOS/$BINARY"
    EXECUTABLE="$APP/Contents/MacOS/$BINARY"
    if [[ "$(lipo -archs "$EXECUTABLE")" != "$ARCH" ]]; then
        echo "Error: $BINARY does not contain exactly $ARCH" >&2
        exit 1
    fi
    # A release must not depend on Homebrew or Cargo build-directory dylibs.
    otool -L "$EXECUTABLE" > "$STAGING/dependencies.txt"
    while IFS= read -r LIBRARY; do
        case "$LIBRARY" in
            /System/Library/*|/usr/lib/*) ;;
            *) echo "Error: unbundled dependency in $BINARY: $LIBRARY" >&2; exit 1 ;;
        esac
    done < <(tail -n +2 "$STAGING/dependencies.txt" | awk '{print $1}')
    ditto "$BIN_DIR/$BINARY.dSYM" "$STAGING/symbols/$BINARY.dSYM"
    strip -Sx "$EXECUTABLE"
done

python3 "$ROOT/scripts/alpha_release.py" plist "$VERSION" "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"
cp "$ROOT/LICENSE" "$APP/Contents/Resources/LICENSE"

ICON_SOURCE="$ROOT/packaging/macos/AppIconAlpha.png"
ICONSET="$STAGING/AppIcon.iconset"
mkdir -p "$ICONSET"
for SIZE in 16 32 128 256 512; do
    sips -z "$SIZE" "$SIZE" "$ICON_SOURCE" \
        --out "$ICONSET/icon_${SIZE}x${SIZE}.png" >/dev/null
    DOUBLE=$((SIZE * 2))
    sips -z "$DOUBLE" "$DOUBLE" "$ICON_SOURCE" \
        --out "$ICONSET/icon_${SIZE}x${SIZE}@2x.png" >/dev/null
done
iconutil --convert icns --output "$APP/Contents/Resources/AppIcon.icns" "$ICONSET"

SIGN_ARGS=(--force --sign "$SIGN_IDENTITY")
if [[ "$SIGN_IDENTITY" == - ]]; then
    SIGN_ARGS+=(--timestamp=none)
else
    SIGN_ARGS+=(--options runtime --timestamp)
fi
# Sign the server and app executable before sealing the containing bundle.
for BINARY in muxy-server muxy-app; do
    codesign "${SIGN_ARGS[@]}" "$APP/Contents/MacOS/$BINARY"
done
codesign "${SIGN_ARGS[@]}" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

ln -s /Applications "$STAGING/dmg/Applications"
DMG="$STAGING/artifacts/Muxy-${VERSION}-${ARCH}.dmg"
hdiutil create -volname "Muxy Alpha" -srcfolder "$STAGING/dmg" -format UDZO -fs HFS+ "$DMG"
codesign "${SIGN_ARGS[@]}" "$DMG"
codesign --verify --strict --verbose=2 "$DMG"
ditto -c -k --sequesterRsrc --keepParent "$STAGING/symbols" \
    "$STAGING/artifacts/Muxy-${VERSION}-${ARCH}-symbols.zip"
mv "$APP" "$STAGING/artifacts/"
mv "$STAGING/artifacts" "$OUTPUT_DIR"
echo "==> Built $OUTPUT_DIR"
