#!/bin/sh
# Build, sign, install and launch a standalone (JS-bundled Release) app on a USB iPhone.
# env: IOS_DEVICE (UDID; default first paired iPhone), IOS_CONFIGURATION (Release),
#      ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_PATH, IOS_SKIP_PREBUILD=1, IOS_NO_LAUNCH=1
set -eu
cd "$(dirname "$0")/.."
device="${IOS_DEVICE:-$(xcrun devicectl list devices 2>/dev/null | awk '/available \(paired\)/ {for(i=1;i<=NF;i++) if ($i ~ /^[0-9A-F]{8}-[0-9A-F]{16}$/) {print $i; exit}}')}"
: "${device:?No paired iPhone found; set IOS_DEVICE}"
configuration="${IOS_CONFIGURATION:-Release}"
key_id="${ASC_KEY_ID:-C953LAWJHB}"
issuer="${ASC_ISSUER_ID:-d476577a-d873-4264-895a-af18c586e3a6}"
key="${ASC_KEY_PATH:-$HOME/.appstoreconnect/private_keys/AuthKey_${key_id}.p8}"
bundle_id="$(node -p "require('./app.json').expo.ios.bundleIdentifier")"

if [ "${IOS_SKIP_PREBUILD:-0}" != 1 ]; then
  CI=1 npx expo prebuild --platform ios --no-install
  (cd ios && pod install)
fi
xcodebuild -workspace ios/extendcomputer.xcworkspace -scheme extendcomputer \
  -configuration "$configuration" -destination "id=$device" -derivedDataPath ios/build/dd \
  -allowProvisioningUpdates -allowProvisioningDeviceRegistration \
  -authenticationKeyPath "$key" -authenticationKeyID "$key_id" -authenticationKeyIssuerID "$issuer" \
  build
app="ios/build/dd/Build/Products/$configuration-iphoneos/extendcomputer.app"
xcrun devicectl device install app --device "$device" "$app"
[ "${IOS_NO_LAUNCH:-0}" = 1 ] || xcrun devicectl device process launch --terminate-existing --device "$device" "$bundle_id"
