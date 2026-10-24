---
"extend-computer-desktop": minor
---

Add experimental macOS display extension and screen mirroring over the local network. Authenticate paired and account devices before exchanging WebRTC setup, encode and decode video with native hardware codecs, present with Metal, and release display resources when the connection ends. Use Retina capture, bounded frame queues, keyframe recovery, and adaptive bitrate to reduce latency and keep text sharp.

Service already-due WebRTC pacer timeouts in bounded batches so the application loop does not add a sleep to every video packet. This prevents large LAN frames from timing out during reassembly while preserving future pacing deadlines and regular input processing.
