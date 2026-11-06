#!/usr/bin/env bash
# Drive the USB-connected iPhone with scripted XCUITest actions and export screenshots/trees.
# usage: drive.sh <out-dir> < actions.txt      (see DriverUITests/DriverTests.swift for the action list)
# env: IOS_DEVICE (UDID; default first paired iPhone), ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_PATH
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:?out dir}"; mkdir -p "$out"
device="${IOS_DEVICE:-$(xcrun devicectl list devices 2>/dev/null | awk '/available \(paired\)/ {for(i=1;i<=NF;i++) if ($i ~ /^[0-9A-F]{8}-[0-9A-F]{16}$/) {print $i; exit}}')}"
key_id="${ASC_KEY_ID:-C953LAWJHB}"
issuer="${ASC_ISSUER_ID:-d476577a-d873-4264-895a-af18c586e3a6}"
key="${ASC_KEY_PATH:-$HOME/.appstoreconnect/private_keys/AuthKey_${key_id}.p8}"
build="$here/.build"; mkdir -p "$build"
[ -d "$here/IOSDriver.xcodeproj" ] || (cd "$here" && xcodegen -q)
actions="$(cat)"
rm -rf "$build/r.xcresult"
set +e
DRIVER_ACTIONS="$actions" TEST_RUNNER_DRIVER_ACTIONS="$actions" xcodebuild -project "$here/IOSDriver.xcodeproj" -scheme IOSDriver \
  -destination "id=$device" -derivedDataPath "$build/dd" -resultBundlePath "$build/r.xcresult" \
  -allowProvisioningUpdates -allowProvisioningDeviceRegistration \
  -authenticationKeyPath "$key" -authenticationKeyID "$key_id" -authenticationKeyIssuerID "$issuer" \
  test > "$build/log.txt" 2>&1
status=$?
set -e
if grep -q "enabling automation mode\|Enable UI Automation" "$build/log.txt"; then
  echo "BLOCKED: the iPhone is showing a Touch ID prompt to enable UI automation. Someone must approve it on the phone, then rerun." >&2
  osascript -e 'display notification "Approve the Touch ID prompt on the iPhone (Enable UI Automation)" with title "extend.computer iPhone tests" sound name "Glass"' >/dev/null 2>&1 || true
fi
grep -E "error:|XCTAssertTrue failed|failed -|not found:|timeout:|\*\* TEST" "$build/log.txt" | grep -v '^    export' | head -20 || true
rm -rf "$build/att" && xcrun xcresulttool export attachments --path "$build/r.xcresult" --output-path "$build/att" >/dev/null 2>&1 || true
if [ -f "$build/att/manifest.json" ]; then
  python3 - "$build/att" "$out" <<'PY'
import json, shutil, sys, os
src, dst = sys.argv[1], sys.argv[2]
for t in json.load(open(os.path.join(src, "manifest.json"))):
    for a in t["attachments"]:
        name = a["suggestedHumanReadableName"].split("_0_")[0]
        ext = os.path.splitext(a["exportedFileName"])[1]
        shutil.copy(os.path.join(src, a["exportedFileName"]), os.path.join(dst, name + ext))
        print(os.path.join(dst, name + ext))
PY
fi
exit $status
