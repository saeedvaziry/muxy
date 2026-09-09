#!/bin/bash
set -euo pipefail

if [[ $# -ne 1 || ! -f "$1" ]]; then
    echo "Usage: $0 <signed.dmg>" >&2
    exit 1
fi
: "${APPLE_ID:?APPLE_ID is required}"
: "${APPLE_APP_SPECIFIC_PASSWORD:?APPLE_APP_SPECIFIC_PASSWORD is required}"
: "${APPLE_TEAM_ID:?APPLE_TEAM_ID is required}"

TEMP="$(mktemp -d)"
trap 'rm -rf "$TEMP"' EXIT
CREDENTIALS=(--apple-id "$APPLE_ID" --password "$APPLE_APP_SPECIFIC_PASSWORD" --team-id "$APPLE_TEAM_ID")
STATUS=0
xcrun notarytool submit "$1" "${CREDENTIALS[@]}" --wait --timeout 30m \
    --output-format json > "$TEMP/submission.json" || STATUS=$?
cat "$TEMP/submission.json"
SUBMISSION_ID="$(python3 -c 'import json, sys; print(json.load(sys.stdin).get("id", ""))' < "$TEMP/submission.json")"
if [[ -n "$SUBMISSION_ID" ]]; then
    xcrun notarytool log "$SUBMISSION_ID" "${CREDENTIALS[@]}" || true
fi
if [[ "$STATUS" -ne 0 ]]; then
    exit "$STATUS"
fi
python3 -c 'import json, sys; sys.exit(0 if json.load(sys.stdin).get("status") == "Accepted" else 1)' \
    < "$TEMP/submission.json"
xcrun stapler staple "$1"
xcrun stapler validate "$1"
spctl --assess --type open --context context:primary-signature --verbose=2 "$1"
