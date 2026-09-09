#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
FORK_REPO="muxy-app/ghostty"
XCFRAMEWORK_DIR="$PROJECT_ROOT/GhosttyKit.xcframework"
RESOURCES_DIR="$PROJECT_ROOT/Muxy/Resources/ghostty"
TERMINFO_DIR="$PROJECT_ROOT/Muxy/Resources/terminfo"

LOCAL_XCFRAMEWORK_TAR="${1:-}"
if [[ -n "$LOCAL_XCFRAMEWORK_TAR" ]]; then
    if [[ ! -f "$LOCAL_XCFRAMEWORK_TAR" ]]; then
        echo "Error: local xcframework tar not found: $LOCAL_XCFRAMEWORK_TAR"
        exit 1
    fi
    LOCAL_XCFRAMEWORK_TAR="$(cd "$(dirname "$LOCAL_XCFRAMEWORK_TAR")" && pwd)/$(basename "$LOCAL_XCFRAMEWORK_TAR")"
fi

GHOSTTY_ARCHIVE="$XCFRAMEWORK_DIR/macos-arm64_x86_64/ghostty-internal.a"
if [[ -f "$GHOSTTY_ARCHIVE" ]]; then
    "$SCRIPT_DIR/normalize-ghostty-archive.swift" "$GHOSTTY_ARCHIVE"
fi

if [[ -d "$XCFRAMEWORK_DIR" && -d "$RESOURCES_DIR/shell-integration" && -d "$TERMINFO_DIR" ]]; then
    echo "==> GhosttyKit.xcframework and resources already present, skipping download"
    echo "    To re-download, remove: rm -rf GhosttyKit.xcframework Muxy/Resources/ghostty Muxy/Resources/terminfo"
    exit 0
fi

cd "$PROJECT_ROOT"

NEEDS_XCFRAMEWORK_DOWNLOAD=false
if [[ ! -d "$XCFRAMEWORK_DIR" && -z "$LOCAL_XCFRAMEWORK_TAR" ]]; then
    NEEDS_XCFRAMEWORK_DOWNLOAD=true
fi

NEEDS_RESOURCES_DOWNLOAD=false
if [[ ! -d "$RESOURCES_DIR/shell-integration" || ! -d "$TERMINFO_DIR" ]]; then
    NEEDS_RESOURCES_DOWNLOAD=true
fi

LATEST_TAG=""
if [[ "$NEEDS_XCFRAMEWORK_DOWNLOAD" == "true" || "$NEEDS_RESOURCES_DOWNLOAD" == "true" ]]; then
    echo "==> Fetching latest GhosttyKit release from $FORK_REPO"
    LATEST_TAG=$(gh release list --repo "$FORK_REPO" --limit 1 --json tagName -q '.[0].tagName')
    if [[ -z "$LATEST_TAG" ]]; then
        echo "Error: No releases found on $FORK_REPO"
        exit 1
    fi
    echo "    Tag: $LATEST_TAG"
fi

if [[ ! -d "$XCFRAMEWORK_DIR" ]]; then
    if [[ -n "$LOCAL_XCFRAMEWORK_TAR" ]]; then
        echo "==> Extracting GhosttyKit.xcframework from $LOCAL_XCFRAMEWORK_TAR"
        tar xzf "$LOCAL_XCFRAMEWORK_TAR"
    else
        echo "==> Downloading GhosttyKit.xcframework"
        gh release download "$LATEST_TAG" \
            --pattern "GhosttyKit.xcframework.tar.gz" \
            --repo "$FORK_REPO"
        tar xzf GhosttyKit.xcframework.tar.gz
        rm GhosttyKit.xcframework.tar.gz
    fi

    echo "==> Syncing ghostty.h from xcframework"
    cp "$XCFRAMEWORK_DIR/macos-arm64_x86_64/Headers/ghostty.h" "$PROJECT_ROOT/GhosttyKit/ghostty.h"
    "$SCRIPT_DIR/normalize-ghostty-archive.swift" "$GHOSTTY_ARCHIVE"
fi

if [[ "$NEEDS_RESOURCES_DOWNLOAD" == "true" ]]; then
    echo "==> Downloading GhosttyKit runtime resources"
    gh release download "$LATEST_TAG" \
        --pattern "GhosttyKit-resources.tar.gz" \
        --repo "$FORK_REPO"
    THEMES_BACKUP=""
    if [[ -d "$RESOURCES_DIR/themes" ]]; then
        THEMES_BACKUP="$(mktemp -d)/themes"
        mv "$RESOURCES_DIR/themes" "$THEMES_BACKUP"
    fi
    rm -rf "$RESOURCES_DIR" "$TERMINFO_DIR"
    mkdir -p "$(dirname "$RESOURCES_DIR")"
    tar xzf GhosttyKit-resources.tar.gz -C "$(dirname "$RESOURCES_DIR")"
    rm GhosttyKit-resources.tar.gz
    rm -rf "$RESOURCES_DIR/themes"
    if [[ -n "$THEMES_BACKUP" ]]; then
        mv "$THEMES_BACKUP" "$RESOURCES_DIR/themes"
        rmdir "$(dirname "$THEMES_BACKUP")" 2>/dev/null || true
    fi
fi

echo "==> Done"
echo "    Run 'swift build' to build the project"
