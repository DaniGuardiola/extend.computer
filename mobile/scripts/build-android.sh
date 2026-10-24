#!/bin/sh
# Build a standalone development APK without running Metro on the host.
# Opt in to run-as diagnostics with EXTEND_ANDROID_STANDALONE_DEBUGGABLE=1.
# This changes only the development package manifest; the release variant still bundles JS.
set -eu
cd "$(dirname "$0")/.."
: "${JAVA_HOME:?Set JAVA_HOME to JDK 17}"
: "${ANDROID_HOME:?Set ANDROID_HOME to an Android SDK with platform 36}"
export GRADLE_USER_HOME="${GRADLE_USER_HOME:-$PWD/.local/gradle}"
npm run check
npx expo prebuild --platform android --no-install
./android/gradlew -p android app:assembleRelease \
  --max-workers=4 \
  '-Dorg.gradle.jvmargs=-Xmx4g -XX:MaxMetaspaceSize=1g'

python3 scripts/verify-android-abis.py android/app/build/outputs/apk/release/app-release.apk
