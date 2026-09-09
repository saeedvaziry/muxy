#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
readonly PROJECT_ROOT
readonly PROFILE="${1:-debug}"

if (($# > 1)) || [[ "$PROFILE" != "debug" && "$PROFILE" != "release" ]]; then
    printf 'Usage: scripts/build-app.sh [debug|release]\n' >&2
    exit 2
fi

for command_name in bash cargo cmp codesign grep iconutil plutil sed xcode-select xcrun; do
    command -v "$command_name" >/dev/null 2>&1 || {
        printf 'error: required command not found: %s\n' "$command_name" >&2
        exit 1
    }
done

DEVELOPER_DIR="$(xcode-select -p)"
readonly DEVELOPER_DIR
[[ -d "$DEVELOPER_DIR/Platforms/MacOSX.platform" ]] || {
    printf 'error: full Xcode is required; xcode-select points to %s\n' "$DEVELOPER_DIR" >&2
    exit 1
}
export DEVELOPER_DIR
MACOS_SDK_PATH="$(xcrun --sdk macosx --show-sdk-path)"
readonly MACOS_SDK_PATH
[[ -d "$MACOS_SDK_PATH" ]] || {
    printf 'error: xcrun returned a missing macOS SDK: %s\n' "$MACOS_SDK_PATH" >&2
    exit 1
}

readonly GHOSTTY_RESOURCES="$PROJECT_ROOT/resources/ghostty"
readonly TERMINFO_RESOURCES="$PROJECT_ROOT/resources/terminfo"
readonly OVERRIDE_RESOURCES="$PROJECT_ROOT/resources/ghostty-overrides"
readonly ICONSET="$PROJECT_ROOT/resources/AppIcon.iconset"
readonly INFO_PLIST="$PROJECT_ROOT/resources/Info.plist"
readonly CLI_SOURCE="$PROJECT_ROOT/Muxy/Resources/scripts/muxy-cli"
readonly DEVELOPMENT_CLI_SOURCE="$PROJECT_ROOT/resources/muxy-dev-bin/muxy"
readonly DEVELOPMENT_SHELL_INTEGRATION="$PROJECT_ROOT/resources/muxy-shell-integration"

[[ -s "$GHOSTTY_RESOURCES/shell-integration/zsh/ghostty-integration" ]] || {
    printf 'error: Ghostty resources are missing; run scripts/setup.sh first\n' >&2
    exit 1
}
[[ -s "$TERMINFO_RESOURCES/67/ghostty" && -s "$TERMINFO_RESOURCES/78/xterm-ghostty" ]] || {
    printf 'error: Ghostty terminfo is missing; run scripts/setup.sh first\n' >&2
    exit 1
}
[[ -d "$OVERRIDE_RESOURCES" && -d "$ICONSET" && -f "$INFO_PLIST" ]] || {
    printf 'error: committed application resources are incomplete\n' >&2
    exit 1
}
[[ -s "$CLI_SOURCE" ]] || {
    printf 'error: retained CLI source is missing\n' >&2
    exit 1
}
if [[ "$PROFILE" == debug && ! -x "$DEVELOPMENT_CLI_SOURCE" ]]; then
    printf 'error: development CLI launcher is missing or not executable\n' >&2
    exit 1
fi
if [[ "$PROFILE" == debug ]]; then
    bash -n "$DEVELOPMENT_CLI_SOURCE" "$DEVELOPMENT_SHELL_INTEGRATION/bash"
    for integration in bash fish zsh; do
        [[ -s "$DEVELOPMENT_SHELL_INTEGRATION/$integration" ]] || {
            printf 'error: development %s shell integration is missing\n' "$integration" >&2
            exit 1
        }
    done
fi
plutil -lint "$INFO_PLIST" >/dev/null

cargo_arguments=(build --package muxy --package muxy-extension-host --locked --target-dir "$PROJECT_ROOT/target")
if [[ "$PROFILE" == "release" ]]; then
    cargo_arguments+=(--release)
fi

cd "$PROJECT_ROOT"
printf '==> Building muxy (%s)\n' "$PROFILE"
MACOSX_DEPLOYMENT_TARGET=14.0 cargo "${cargo_arguments[@]}"

readonly PROFILE_DIRECTORY="$PROJECT_ROOT/target/$PROFILE"
readonly APP_BUNDLE="$PROFILE_DIRECTORY/Muxy.app"
readonly STAGING_BUNDLE="$PROFILE_DIRECTORY/.Muxy.app.staging.$$"

cleanup() {
    if [[ -d "$STAGING_BUNDLE" ]]; then
        rm -rf "$STAGING_BUNDLE"
    fi
}
trap cleanup EXIT

mkdir -p "$STAGING_BUNDLE/Contents/MacOS" \
    "$STAGING_BUNDLE/Contents/Resources/Muxy_Muxy.bundle/scripts"
install -m 0755 "$PROFILE_DIRECTORY/muxy" "$STAGING_BUNDLE/Contents/MacOS/muxy"
install -m 0755 "$PROFILE_DIRECTORY/muxy-extension-host" \
    "$STAGING_BUNDLE/Contents/MacOS/muxy-extension-host"
install -m 0755 "$CLI_SOURCE" \
    "$STAGING_BUNDLE/Contents/Resources/Muxy_Muxy.bundle/scripts/muxy-cli"
if [[ "$PROFILE" == debug ]]; then
    mkdir -p "$STAGING_BUNDLE/Contents/Resources/muxy-dev-bin"
    install -m 0755 "$DEVELOPMENT_CLI_SOURCE" \
        "$STAGING_BUNDLE/Contents/Resources/muxy-dev-bin/muxy"
fi
install -m 0644 "$INFO_PLIST" "$STAGING_BUNDLE/Contents/Info.plist"
if [[ "$PROFILE" == debug ]]; then
    plutil -replace CFBundleName -string "Muxy Dev" "$STAGING_BUNDLE/Contents/Info.plist"
    plutil -replace CFBundleDisplayName -string "Muxy Dev" "$STAGING_BUNDLE/Contents/Info.plist"
    plutil -replace CFBundleIdentifier -string "com.muxy.dev" "$STAGING_BUNDLE/Contents/Info.plist"
fi
iconutil --convert icns --output "$STAGING_BUNDLE/Contents/Resources/AppIcon.icns" "$ICONSET"

cp -R "$GHOSTTY_RESOURCES" "$STAGING_BUNDLE/Contents/Resources/ghostty"
cp -R "$TERMINFO_RESOURCES" "$STAGING_BUNDLE/Contents/Resources/terminfo"
cp -R "$OVERRIDE_RESOURCES" "$STAGING_BUNDLE/Contents/Resources/ghostty-overrides"
if [[ "$PROFILE" == debug ]]; then
    cat "$DEVELOPMENT_SHELL_INTEGRATION/bash" >> \
        "$STAGING_BUNDLE/Contents/Resources/ghostty/shell-integration/bash/ghostty.bash"
    readonly BUNDLED_ZSH_INTEGRATION="$STAGING_BUNDLE/Contents/Resources/ghostty/shell-integration/zsh/ghostty-integration"
    [[ "$(grep -c '^    # Sudo$' "$BUNDLED_ZSH_INTEGRATION")" == 1 ]] || {
        printf 'error: bundled zsh shell integration has an unexpected structure\n' >&2
        exit 1
    }
    sed -n '1,/^    # Sudo$/p' "$BUNDLED_ZSH_INTEGRATION" > "$BUNDLED_ZSH_INTEGRATION.muxy"
    cat "$DEVELOPMENT_SHELL_INTEGRATION/zsh" >> "$BUNDLED_ZSH_INTEGRATION.muxy"
    sed -n '/^    # Sudo$/,$p' "$BUNDLED_ZSH_INTEGRATION" | sed '1d' >> \
        "$BUNDLED_ZSH_INTEGRATION.muxy"
    mv "$BUNDLED_ZSH_INTEGRATION.muxy" "$BUNDLED_ZSH_INTEGRATION"
    install -m 0644 "$DEVELOPMENT_SHELL_INTEGRATION/fish" \
        "$STAGING_BUNDLE/Contents/Resources/ghostty/shell-integration/fish/vendor_conf.d/zz-muxy-development-cli.fish"
fi

readonly BUNDLED_CLI="$STAGING_BUNDLE/Contents/Resources/Muxy_Muxy.bundle/scripts/muxy-cli"
[[ -x "$BUNDLED_CLI" ]] || {
    printf 'error: bundled legacy CLI is not executable before signing\n' >&2
    exit 1
}
cmp -s "$CLI_SOURCE" "$BUNDLED_CLI" || {
    printf 'error: bundled legacy CLI differs before signing\n' >&2
    exit 1
}
readonly BUNDLED_DEVELOPMENT_CLI="$STAGING_BUNDLE/Contents/Resources/muxy-dev-bin/muxy"
if [[ "$PROFILE" == debug ]]; then
    [[ -x "$BUNDLED_DEVELOPMENT_CLI" ]] || {
        printf 'error: bundled development CLI is not executable before signing\n' >&2
        exit 1
    }
    cmp -s "$DEVELOPMENT_CLI_SOURCE" "$BUNDLED_DEVELOPMENT_CLI" || {
        printf 'error: bundled development CLI differs before signing\n' >&2
        exit 1
    }
elif [[ -e "$BUNDLED_DEVELOPMENT_CLI" ]]; then
    printf 'error: release bundle contains the development CLI\n' >&2
    exit 1
fi

printf '==> Ad-hoc signing Muxy.app\n'
codesign --force --sign - --timestamp=none \
    "$STAGING_BUNDLE/Contents/MacOS/muxy-extension-host"
codesign --force --sign - --timestamp=none "$STAGING_BUNDLE"
"$SCRIPT_DIR/verify-bundle.sh" "$STAGING_BUNDLE" "$PROFILE"

rm -rf "$APP_BUNDLE"
mv "$STAGING_BUNDLE" "$APP_BUNDLE"
trap - EXIT

printf '==> Built %s\n' "$APP_BUNDLE"
