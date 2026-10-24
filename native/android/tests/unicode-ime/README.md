# Unicode InputConnection test IME

Separate test-only package; does not instrument, rebuild, or restart Extend. Its exported command receiver requires `android.permission.DUMP` (shell/system). The IME only captures connections whose `EditorInfo.packageName` is exactly `computer.extend.android.development`. Fullscreen extract mode is disabled. No editor getters, clipboard access, or arbitrary text command exist.

Build into a fresh directory, using the same existing local debug signing key as the observer:

```sh
python3 scripts/android/build_test_ime.py --sdk mobile/.local/sdk \
  --java-bin mobile/.local/jdk/jdk-17.0.20.1+1/Contents/Home/bin \
  --output native/android/.local/unicode-ime-test
```

The operator owns device selection and restoration. Replace `$serial` with the tablet; never use unscoped ADB when multiple devices are attached:

```sh
previous_ime=$(adb -s "$serial" shell settings get secure default_input_method | tr -d '\r')
adb -s "$serial" install -r -t native/android/.local/unicode-ime-test/unicode-ime-test.apk
adb -s "$serial" shell ime enable computer.extend.android.testime/.UnicodeIme
adb -s "$serial" shell ime set computer.extend.android.testime/.UnicodeIme
adb -s "$serial" shell am broadcast -n computer.extend.android.testime/.CommandReceiver --es command state
```

Focus Extend's display or independent-control keyboard with an active authenticated text grant; focus the separate source observer's own editable field. Save source observer events/text and guard counters before each command. Commands return JSON in the ordered-broadcast result and log the same receipt under `ExtendTestIme`:

```sh
adb -s "$serial" shell am broadcast -n computer.extend.android.testime/.CommandReceiver --es command draft
# Verify zero source text changes/events before final commit.
adb -s "$serial" shell am broadcast -n computer.extend.android.testime/.CommandReceiver --es command commit
adb -s "$serial" shell am broadcast -n computer.extend.android.testime/.CommandReceiver --es command finish
# Verify exact FINAL once, including tail after the first 20 UTF16 units; finish must not duplicate it.
```

`FINAL` is `Unicode é café 👩‍💻 中文测试 日本語 ✅` (32 UTF16 units, 30 Unicode scalars, 56 UTF8 bytes). The first accent is `e` + U+0301; the emoji is U+1F469 U+200D U+1F4BB. `draft` sets local composition to `Draft é 👩‍💻 中文`. A separate `draft` → `finish` case must deliver that draft exactly once.

For stale-connection rejection, call `retain` while the Extend editor is focused. Turn the product keyboard/session off, record source state, then call `stale-commit` and `stale-finish`. These deliberately invoke the saved connection even after `onFinishInput`; they never substitute a newly focused connection. If another package currently owns the editor, the harness refuses the stale call. Expected source result: no new text/events. Fixture `STALE é 👩‍💻 中文 forbidden` is distinguishable from valid final text. `clear` releases the retained object.

InputConnection return values are dispatch evidence, **not remote application delivery**. A stale binder can be rejected by Android before reaching the app; native stale IME unit regressions separately cover the product guard. Actual source observer evidence is required for both successful delivery and absence checks. No screen capture or benchmark is needed.

Always restore the original IME before disabling/removing this test package:

```sh
adb -s "$serial" shell am broadcast -n computer.extend.android.testime/.CommandReceiver --es command clear
adb -s "$serial" shell ime set "$previous_ime"
adb -s "$serial" shell ime disable computer.extend.android.testime/.UnicodeIme
adb -s "$serial" uninstall computer.extend.android.testime
```

Primary API contracts: [InputMethodService](https://developer.android.com/reference/android/inputmethodservice/InputMethodService), [InputConnection](https://developer.android.com/reference/android/view/inputmethod/InputConnection), [AOSP InputConnection contract](https://android.googlesource.com/platform/frameworks/base/+/master/core/java/android/view/inputmethod/InputConnection.java).
