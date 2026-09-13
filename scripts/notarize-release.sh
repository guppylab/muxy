#!/bin/bash
set -euo pipefail

if [[ $# -ne 1 || ! -f "$1" ]]; then
    echo "Usage: $0 <signed.dmg|signed.zip>" >&2
    exit 1
fi
case "$1" in
    *.dmg|*.zip) ;;
    *) echo "Error: expected a DMG or ZIP" >&2; exit 1 ;;
esac
: "${APPLE_ID:?APPLE_ID is required}"
: "${APPLE_APP_SPECIFIC_PASSWORD:?APPLE_APP_SPECIFIC_PASSWORD is required}"
: "${APPLE_TEAM_ID:?APPLE_TEAM_ID is required}"

TEMP="$(mktemp -d)"
trap 'rm -rf "$TEMP"' EXIT
CREDENTIALS=(--apple-id "$APPLE_ID" --password "$APPLE_APP_SPECIFIC_PASSWORD" --team-id "$APPLE_TEAM_ID")
for ATTEMPT in 1 2 3; do
    STATUS=0
    xcrun notarytool submit "$1" "${CREDENTIALS[@]}" --wait --timeout 30m \
        --output-format json > "$TEMP/submission.json" 2> "$TEMP/submission.error" || STATUS=$?
    cat "$TEMP/submission.json"
    cat "$TEMP/submission.error" >&2
    if [[ "$STATUS" -eq 0 || "$ATTEMPT" -eq 3 ]] || ! grep -q NSURLErrorDomain "$TEMP/submission.error"; then
        break
    fi
    echo "Notarization network error; retrying in 5 seconds ($ATTEMPT/3)" >&2
    sleep 5
done
SUBMISSION_ID="$(python3 -c 'import json, sys; print(json.load(sys.stdin).get("id", ""))' \
    < "$TEMP/submission.json" 2>/dev/null || true)"
if [[ -n "$SUBMISSION_ID" ]]; then
    xcrun notarytool log "$SUBMISSION_ID" "${CREDENTIALS[@]}" || true
fi
if [[ "$STATUS" -ne 0 ]]; then
    exit "$STATUS"
fi
python3 -c 'import json, sys; sys.exit(0 if json.load(sys.stdin).get("status") == "Accepted" else 1)' \
    < "$TEMP/submission.json"
case "$1" in
    *.dmg)
        xcrun stapler staple "$1"
        xcrun stapler validate "$1"
        spctl --assess --type open --context context:primary-signature --verbose=2 "$1"
        ;;
    *.zip)
        # ZIPs and bare executables cannot be stapled. Verify the submitted bytes.
        for BINARY in muxy muxy-server; do
            unzip -p "$1" "$BINARY" > "$TEMP/$BINARY"
            chmod 755 "$TEMP/$BINARY"
            codesign --verify --strict --verbose=2 "$TEMP/$BINARY"
            codesign --verify --verbose=4 --requirement notarized --check-notarization "$TEMP/$BINARY"
            "$TEMP/$BINARY" --build-info
        done
        ;;
esac
