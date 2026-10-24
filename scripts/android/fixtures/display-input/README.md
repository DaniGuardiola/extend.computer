# Validator regression fixture

The target events are the first actual physical tap (native observer sequences 47–51) from `native/android/.local/display-input-physical-20261010-1/observer-fixed-after-taps.json`, with its preceding counter baseline. The snapshots are trimmed to the fields needed by this validator; the target after-counters are restricted to that first tap. The descriptor describes the recorded 1920×1200 display; it is reconstructed for this regression fixture, not a fresh native descriptor receipt.

This is historical evidence used to test the offline validator. It does not establish acceptance for any other APK or Mac build. Tests corrupt copies to verify rejection of wrong targets, lost events, inconsistent counters, guard input and unreleased buttons. Synthetic drag/scroll mutations exercise validation only and are not physical evidence.
