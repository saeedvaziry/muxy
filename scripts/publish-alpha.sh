#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [[ $# -ne 2 ]]; then
    echo "Usage: $0 <2.0.0-alpha-N> <artifact-directory>" >&2
    exit 1
fi
VERSION="$1"
python3 "$ROOT/scripts/alpha_release.py" check-version "$VERSION"
ARTIFACTS="$(cd "$2" && pwd)"
TAG="v$VERSION"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
: "${GITHUB_SHA:?GITHUB_SHA is required}"
if [[ "${GITHUB_REF:-}" != refs/heads/2.x ]]; then
    echo "Error: alpha releases are restricted to 2.x" >&2
    exit 1
fi
cd "$ROOT"
if [[ "$(git rev-parse HEAD)" != "$GITHUB_SHA" ]]; then
    echo "Error: checkout does not match the triggering commit" >&2
    exit 1
fi
if [[ "$(python3 scripts/alpha_release.py version | sed -n 's/^version=//p')" != "$VERSION" ]]; then
    echo "Error: version does not match the triggering commit count" >&2
    exit 1
fi

check_source() {
    git fetch origin '+refs/heads/2.x:refs/remotes/origin/2.x' --tags
    git merge-base --is-ancestor "$GITHUB_SHA" origin/2.x
    if git show-ref --verify --quiet "refs/tags/$TAG"; then
        if [[ "$(git rev-parse "refs/tags/$TAG^{}")" != "$GITHUB_SHA" ]]; then
            echo "Error: $TAG already points to a different commit" >&2
            exit 1
        fi
    fi
}
check_source

cd "$ARTIFACTS"
for ARCH in arm64 x86_64; do
    if [[ ! -s "Muxy-${VERSION}-${ARCH}.dmg" ]]; then
        echo "Error: missing $ARCH DMG" >&2
        exit 1
    fi
done
shasum -a 256 "Muxy-${VERSION}-arm64.dmg" "Muxy-${VERSION}-x86_64.dmg" > SHA256SUMS

if gh release view "$TAG" --repo "$GITHUB_REPOSITORY" \
    --json isDraft,isPrerelease,targetCommitish > release.json; then
    python3 -c 'import json, sys; r = json.load(sys.stdin); sys.exit(0 if r["isPrerelease"] and r["targetCommitish"] == sys.argv[1] else 1)' \
        "$GITHUB_SHA" < release.json
    if [[ "$(python3 -c 'import json, sys; print(json.load(sys.stdin)["isDraft"])' < release.json)" == False ]]; then
        echo "==> $TAG is already published; leaving its assets unchanged"
        exit 0
    fi
else
    cat > release-notes.md <<EOF
Experimental Rust/GPUI alpha from the \`2.x\` branch. Not intended for production use.

- macOS 14 or newer. Choose \`arm64\` for Apple Silicon or \`x86_64\` for Intel.
- Drag \`Muxy Alpha.app\` to Applications. The app includes its matching \`muxy-server\`.
- Installs alongside Muxy, with separate settings and sessions in \`~/Library/Application Support/Muxy Alpha\`.
- Updates are manual. Before replacing an earlier alpha, use **End All Sessions and Quit** to stop its persistent server. This ends running terminal sessions.

Source: https://github.com/$GITHUB_REPOSITORY/commit/$GITHUB_SHA
EOF
    PREVIOUS="$(git -C "$ROOT" describe --tags --match 'v2.0.0-alpha-*' --abbrev=0 "$GITHUB_SHA^" 2>/dev/null || true)"
    if [[ -n "$PREVIOUS" ]]; then
        gh api --method POST "repos/$GITHUB_REPOSITORY/releases/generate-notes" \
            -f "tag_name=$TAG" -f "target_commitish=$GITHUB_SHA" \
            -f "previous_tag_name=$PREVIOUS" --jq .body >> release-notes.md
    fi
    gh release create "$TAG" --repo "$GITHUB_REPOSITORY" --target "$GITHUB_SHA" \
        --title "Muxy $VERSION" --draft --prerelease --latest=false --notes-file release-notes.md
fi

# A failed upload leaves a resumable draft, never a half-populated public release.
gh release upload "$TAG" --repo "$GITHUB_REPOSITORY" --clobber \
    "Muxy-${VERSION}-arm64.dmg" "Muxy-${VERSION}-x86_64.dmg" SHA256SUMS
cd "$ROOT"
check_source
gh release edit "$TAG" --repo "$GITHUB_REPOSITORY" --target "$GITHUB_SHA" \
    --draft=false --prerelease --latest=false
