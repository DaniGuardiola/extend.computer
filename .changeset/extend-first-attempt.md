---
"extend-computer-desktop": patch
---

Fix extending a display failing on the first attempt with "placement rejected": the app now waits for macOS to bring the new display online before placing it.
