#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
readonly PROJECT_ROOT
readonly APP_BUNDLE="${1:-$PROJECT_ROOT/target/debug/Muxy.app}"
readonly PROFILE="${2:-}"

if (($# > 2)) || [[ -n "$PROFILE" && "$PROFILE" != debug && "$PROFILE" != release ]]; then
    printf 'Usage: scripts/verify-bundle.sh [path/to/Muxy.app] [debug|release]\n' >&2
    exit 2
fi

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

for command_name in cmp codesign grep iconutil plutil; do
    command -v "$command_name" >/dev/null 2>&1 || fail "required command not found: $command_name"
done
[[ -x /usr/libexec/PlistBuddy ]] || fail "required command not found: /usr/libexec/PlistBuddy"

readonly CONTENTS="$APP_BUNDLE/Contents"
readonly PLIST="$CONTENTS/Info.plist"
readonly RESOURCES="$CONTENTS/Resources"
readonly EXECUTABLE="$CONTENTS/MacOS/muxy"
readonly EXTENSION_HOST="$CONTENTS/MacOS/muxy-extension-host"
readonly ICON="$RESOURCES/AppIcon.icns"
readonly CLI_SOURCE="$PROJECT_ROOT/Muxy/Resources/scripts/muxy-cli"
readonly BUNDLED_CLI="$RESOURCES/Muxy_Muxy.bundle/scripts/muxy-cli"
readonly DEVELOPMENT_CLI_SOURCE="$PROJECT_ROOT/resources/muxy-dev-bin/muxy"
readonly BUNDLED_DEVELOPMENT_CLI="$RESOURCES/muxy-dev-bin/muxy"

[[ -d "$APP_BUNDLE" ]] || fail "app bundle not found: $APP_BUNDLE"
[[ -x "$EXECUTABLE" ]] || fail "bundle executable is missing or not executable"
[[ -x "$EXTENSION_HOST" && -s "$EXTENSION_HOST" ]] || {
    fail "extension host is missing or not executable"
}
[[ -f "$PLIST" ]] || fail "Info.plist is missing"
[[ -s "$ICON" ]] || fail "AppIcon.icns is missing or empty"
[[ -x "$BUNDLED_CLI" && -s "$BUNDLED_CLI" ]] || {
    fail "bundled legacy CLI is missing or not executable"
}
cmp -s "$CLI_SOURCE" "$BUNDLED_CLI" || fail "bundled legacy CLI differs from retained source"
if [[ "$PROFILE" == debug || -x "$BUNDLED_DEVELOPMENT_CLI" ]]; then
    [[ -x "$BUNDLED_DEVELOPMENT_CLI" && -s "$BUNDLED_DEVELOPMENT_CLI" ]] || {
        fail "bundled development CLI is missing or not executable"
    }
    cmp -s "$DEVELOPMENT_CLI_SOURCE" "$BUNDLED_DEVELOPMENT_CLI" || {
        fail "bundled development CLI differs from its source"
    }
fi
if [[ "$PROFILE" == release && -e "$BUNDLED_DEVELOPMENT_CLI" ]]; then
    fail "release bundle contains the development CLI"
fi
plutil -lint "$PLIST" >/dev/null

plist_value() {
    /usr/libexec/PlistBuddy -c "Print :$1" "$PLIST"
}

expected_name="Muxy"
expected_identifier="com.muxy.app"
if [[ "$PROFILE" == debug ]]; then
    expected_name="Muxy Dev"
    expected_identifier="com.muxy.dev"
fi
readonly expected_name expected_identifier
[[ "$(plist_value CFBundleName)" == "$expected_name" ]] || {
    fail "CFBundleName must be $expected_name for ${PROFILE:-release}"
}
[[ "$(plist_value CFBundleDisplayName)" == "$expected_name" ]] || {
    fail "CFBundleDisplayName must be $expected_name for ${PROFILE:-release}"
}
[[ "$(plist_value CFBundleExecutable)" == "muxy" ]] || fail "CFBundleExecutable must be muxy"
[[ "$(plist_value CFBundleIdentifier)" == "$expected_identifier" ]] || {
    fail "CFBundleIdentifier must be $expected_identifier for ${PROFILE:-release}"
}
[[ "$(plist_value CFBundlePackageType)" == "APPL" ]] || fail "CFBundlePackageType must be APPL"
[[ "$(plist_value CFBundleIconFile)" == "AppIcon" ]] || fail "CFBundleIconFile must be AppIcon"
[[ "$(plist_value LSMinimumSystemVersion)" == "14.0" ]] || {
    fail "LSMinimumSystemVersion must be 14.0"
}

[[ -s "$RESOURCES/ghostty/shell-integration/bash/ghostty.bash" ]] || {
    fail "bundled bash shell integration is missing"
}
[[ -s "$RESOURCES/ghostty/shell-integration/fish/vendor_conf.d/ghostty-shell-integration.fish" ]] || {
    fail "bundled fish shell integration is missing"
}
[[ -s "$RESOURCES/ghostty/shell-integration/zsh/ghostty-integration" ]] || {
    fail "bundled zsh shell integration is missing"
}
readonly BUNDLED_DEVELOPMENT_FISH_INTEGRATION="$RESOURCES/ghostty/shell-integration/fish/vendor_conf.d/zz-muxy-development-cli.fish"
if [[ "$PROFILE" == debug || -x "$BUNDLED_DEVELOPMENT_CLI" ]]; then
    for integration in \
        "$RESOURCES/ghostty/shell-integration/bash/ghostty.bash" \
        "$RESOURCES/ghostty/shell-integration/zsh/ghostty-integration" \
        "$BUNDLED_DEVELOPMENT_FISH_INTEGRATION"; do
        grep -Fq 'MUXY_DEVELOPMENT_CLI_BIN' "$integration" || {
            fail "bundled shell integration does not activate the development CLI: $integration"
        }
    done
fi
if [[ "$PROFILE" == release ]]; then
    for integration in \
        "$RESOURCES/ghostty/shell-integration/bash/ghostty.bash" \
        "$RESOURCES/ghostty/shell-integration/zsh/ghostty-integration"; do
        if grep -Fq 'MUXY_DEVELOPMENT_CLI_BIN' "$integration"; then
            fail "release shell integration activates the development CLI: $integration"
        fi
    done
    [[ ! -e "$BUNDLED_DEVELOPMENT_FISH_INTEGRATION" ]] || {
        fail "release bundle contains the development fish integration"
    }
fi
[[ -s "$RESOURCES/terminfo/67/ghostty" ]] || fail "bundled ghostty terminfo is missing"
[[ -s "$RESOURCES/terminfo/78/xterm-ghostty" ]] || fail "bundled xterm-ghostty terminfo is missing"
[[ -s "$RESOURCES/ghostty-overrides/muxy-defaults.conf" ]] || {
    fail "bundled Muxy defaults are missing"
}
[[ -s "$RESOURCES/ghostty-overrides/transparent-surface.conf" ]] || {
    fail "bundled transparent surface overrides are missing"
}

icon_check_directory="$(mktemp -d "${TMPDIR:-/tmp}/muxy-icon-check.XXXXXX")"
cleanup() {
    rm -rf "$icon_check_directory"
}
trap cleanup EXIT
iconutil --convert iconset --output "$icon_check_directory/AppIcon.iconset" "$ICON"
for icon_name in \
    icon_16x16.png icon_16x16@2x.png \
    icon_32x32.png icon_32x32@2x.png \
    icon_128x128.png icon_128x128@2x.png \
    icon_256x256.png icon_256x256@2x.png \
    icon_512x512.png icon_512x512@2x.png; do
    [[ -s "$icon_check_directory/AppIcon.iconset/$icon_name" ]] || {
        fail "compiled icon is missing $icon_name"
    }
done

codesign --verify --strict --verbose=2 "$EXTENSION_HOST"
extension_host_signature="$(codesign --display --verbose=4 "$EXTENSION_HOST" 2>&1)"
grep -q '^Signature=adhoc$' <<<"$extension_host_signature" || {
    fail "extension host signature is not ad-hoc"
}
codesign --verify --deep --strict --verbose=2 "$APP_BUNDLE"
signature_details="$(codesign --display --verbose=4 "$APP_BUNDLE" 2>&1)"
grep -q '^Signature=adhoc$' <<<"$signature_details" || fail "bundle signature is not ad-hoc"

printf '==> Verified %s\n' "$APP_BUNDLE"
