#!/bin/sh
# Cross-compile the shared session engine. No private data enters build output.
set -eu
cd "$(dirname "$0")/../.."
: "${ANDROID_HOME:?Set ANDROID_HOME to an Android SDK}"
task_ndk="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/27.1.12297006}"
case "$(uname -s)" in
  Darwin) task_host=darwin-x86_64 ;;
  Linux) task_host=linux-x86_64 ;;
  *) echo "Android engine build requires a macOS or Linux NDK toolchain." >&2; exit 1 ;;
esac
task_tools="$task_ndk/toolchains/llvm/prebuilt/$task_host/bin"
# Gradle supplies its actual requested ABIs; standalone builds use app config.
task_archs="${EXTEND_ANDROID_ARCHITECTURES:-$(python3 mobile/scripts/verify-android-abis.py --configured)}"
task_archs=$(python3 mobile/scripts/verify-android-abis.py --configured --expected "$task_archs")
case ",$task_archs," in
  *,arm64-v8a,*|*,x86_64,*) ;;
  *) echo "No supported Android architectures requested." >&2; exit 1 ;;
esac
# Validate the whole list before compiling or replacing any artifact.
task_seen=,
task_old_ifs=$IFS
IFS=,
for task_abi in $task_archs; do
  case "$task_abi" in arm64-v8a|x86_64) ;; *) echo "Unsupported Android ABI: $task_abi" >&2; exit 1 ;; esac
  case "$task_seen" in *,$task_abi,*) echo "Duplicate Android ABI: $task_abi" >&2; exit 1 ;; esac
  task_seen="$task_seen$task_abi,"
done
IFS=$task_old_ifs
# Android std components belong to the workspace-pinned rustup toolchain.
export RUSTC="$(rustup which rustc)"
task_cargo="$(rustup which cargo)"
for task_abi in $(echo "$task_archs" | tr ',' ' '); do
  case "$task_abi" in
    arm64-v8a) task_target=aarch64-linux-android; task_clang=aarch64-linux-android29-clang ;;
    x86_64) task_target=x86_64-linux-android; task_clang=x86_64-linux-android29-clang ;;
  esac
  task_upper=$(echo "$task_target" | tr '[:lower:]-' '[:upper:]_')
  task_lower=$(echo "$task_target" | tr '-' '_')
  env "CARGO_TARGET_${task_upper}_LINKER=$task_tools/$task_clang" \
    "CC_${task_lower}=$task_tools/$task_clang" "AR_${task_lower}=$task_tools/llvm-ar" \
    "CARGO_TARGET_${task_upper}_RUSTFLAGS=-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384" \
    "$task_cargo" build --locked --release -p extend-android-engine --target "$task_target"
  task_destination="mobile/modules/extend-display/android/src/main/jniLibs/$task_abi"
  mkdir -p "$task_destination"
  cp "target/$task_target/release/libextend_android_engine.so" "$task_destination/"
done
python3 mobile/scripts/verify-android-abis.py --jni-dir mobile/modules/extend-display/android/src/main/jniLibs --expected "$task_archs"
