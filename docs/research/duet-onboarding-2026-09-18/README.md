> 🤖🔧 ai generated

# Duet onboarding research — 2026-09-18

Source: twelve original screenshots supplied by the user after resetting Duet permissions, onboarding preferences, and its leftover AWDL helper. Files copied without editing; sequence follows attachment order. These are observations of this installation, not a claim about every Duet version or its internal implementation.

## Observed flow

1. Screen sharing explanation → macOS Screen Recording prompt → System Settings toggle → system authentication → Quit & Reopen prompt.
2. Performance Boost explanation → native Add Helper administrator-password dialog. Duet explicitly explains that this installs a helper and temporarily disables AirDrop during an active connection.
3. Touch & Gestures explanation → macOS Accessibility prompt → System Settings toggle → system authentication.
4. All three steps show checkmarks, then the user can finish onboarding.

The earlier local cleanup independently found a signed `com.kairos.awdltool.xpc` executable under `/Library/PrivilegedHelperTools` and a matching LaunchDaemon. Together, these observations support a separately installed privileged AWDL helper. The screenshots do not establish the specific installation API, lease implementation, crash recovery, or actual interface state during a session. Do not infer SMAppService or SMJobBless solely from the dialog.

## Assessment for extend.computer

Adopt the explicit explanation, one action at a time, visible progress, and native authentication. The helper is a first-class setup dependency rather than an invisible optional installation step. Separate privacy permission status from helper readiness.

Avoid copying the entire mandatory three-step funnel. Keyboard/mouse users should see only permissions required by the selected role; request Screen Recording when implementing and using screen sharing. Name input permissions by their purpose for our product, not Touch & Gestures. Offer Wi-Fi optimization as recommended with a clear skip/opt-out path. No skip is visible in these screenshots; behavior after denial was not recorded.

Suggested optimization copy: “Reduce Wi-Fi stutter. Install a helper to temporarily pause AirDrop and some Continuity features while sharing controls over Wi-Fi. macOS will ask for administrator approval.” Use “Enable” and “Not now” actions. Exact native approval wording depends on the chosen supported installation mechanism.

Recheck actual permissions and helper health on return from System Settings and on launch. Persist preferences, not an authoritative setup-complete boolean. A checked setup step means the helper is ready; show active optimization only after a session successfully acquires a lease and verifies the intended state. Missing or incompatible helpers need an in-app repair action.

Development should exercise the same readiness states and onboarding as production, with separate service identity and stable signing. GUI rebuilds should reuse a compatible helper. Provide an explicit dev reset procedure covering app preferences, scoped privacy grants, and service removal; app deletion alone did not reset this observed Duet installation.

## Screenshot index

| Step | Screenshot | Original filename |
|---|---|---|
| 1 | [screen-sharing-intro](01-screen-sharing-intro.png) | `codex-clipboard-442a9376-059a-403c-b47e-6b6103847d67.png` |
| 2 | [screen-recording-prompt](02-screen-recording-prompt.png) | `codex-clipboard-5d9e9eea-55cf-4deb-b30a-52344e378d8d.png` |
| 3 | [screen-recording-settings](03-screen-recording-settings.png) | `codex-clipboard-b0b2bd30-bf5d-471e-b523-f488d4eefe4c.png` |
| 4 | [screen-recording-authentication](04-screen-recording-authentication.png) | `codex-clipboard-c3fe532c-93a0-4986-be75-6596b8f4c98b.png` |
| 5 | [quit-and-reopen](05-quit-and-reopen.png) | `codex-clipboard-2daec6b6-bb72-46a2-8774-461447143c65.png` |
| 6 | [performance-boost-intro](06-performance-boost-intro.png) | `codex-clipboard-b6d8b7a6-5696-4a47-aac9-36ae3d7b00e6.png` |
| 7 | [add-helper-authentication](07-add-helper-authentication.png) | `codex-clipboard-20814fee-cb8d-447c-b8e4-59462c97e952.png` |
| 8 | [touch-and-gestures-intro](08-touch-and-gestures-intro.png) | `codex-clipboard-f89a2ec4-430a-4549-92ba-f80d8eaf1265.png` |
| 9 | [accessibility-prompt](09-accessibility-prompt.png) | `codex-clipboard-bc07128d-3a63-4ff9-a69f-ef7e1e67ee92.png` |
| 10 | [accessibility-settings](10-accessibility-settings.png) | `codex-clipboard-954f1365-39e1-4b84-aef3-8cc1d90a7d38.png` |
| 11 | [accessibility-authentication](11-accessibility-authentication.png) | `codex-clipboard-519028f3-d623-4ada-b3e7-de79ac0c0057.png` |
| 12 | [setup-complete](12-setup-complete.png) | `codex-clipboard-725a1aba-9ede-4c03-8f2c-297078ab82ce.png` |
