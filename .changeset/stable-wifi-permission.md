---
"extend-computer-desktop": patch
---

Keep Wi-Fi optimization showing as allowed while a USB cable is connected. Background permission checks over a cable skip the Wi-Fi check, and they no longer reset its status, so the setting stops flipping to "Permission needed" and devices on the local network can keep connecting.
