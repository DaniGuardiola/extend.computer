# Project TODOs

- [ ] QR login support: sign in by scanning a QR code with another already signed-in device and approving the login there, similar to Steam, for easier login. Explore QR scanning as an optional device-pairing flow in the future too.

- [ ] Define the product tagline and supporting brand copy, then apply them consistently across the website, desktop app, transactional emails, documentation, and release/download pages. Review existing phrases, including the provisional email footer “Your devices, working together.” Keep approved wording in a shared source where practical so future changes stay consistent.

- [ ] Revisit native USB video pacing when performance work resumes. On 2026-10-10, bypassing network packet pacing for native USB retained queue bounds and recovery but showed essentially unchanged median optical video lag in one clip per build (62.47 ms baseline, 62.53 ms candidate; 1138×710, HEVC, 60 Hz). The candidate subjectively felt smoother; this limited test does not rule out a benefit in delivery consistency or stalls. The experiment was reverted; the faster Android compositor cursor remains enabled separately. A later functional check overflowed the unchanged 16-frame queue after roughly two seconds without decoder input attempts: the queued frames arrived within one millisecond but spanned 1.22 seconds of source timestamps, with zero failed codec dequeue attempts. The pause's origin remains unproved; an unchanged-build reconnect succeeded. Start any future investigation from the saved local receipts and that observation, without repeating a broad benchmark campaign.

- [ ] Track Android continued-touch cleanup after external Accessibility permission removal. On a Redmi Pad 2 / Android 16 the authenticated source disconnected immediately, but its target still recorded a held touch after the service was unbound. Public continued strokes intentionally remain down, and an unbound service loses its gesture-dispatch connection; the exact Xiaomi framework implementation remains unverified. Improve best-effort pending-stroke cleanup while still bound and record rejected terminal dispatches. Check framework/vendor behavior on other supported versions before promising post-unbind cancellation. Finite gesture chunks change live drag semantics and must not silently replace continuous dragging. See `mobile/PARITY.md` for primary API/framework references and the verified limits.

- [ ] Report receiver-side display start failures to the source. In one Mac-to-Mac USB start, the receiver failed its first datagram send and showed an error, while the source silently returned to idle. A retry worked. Capture logs from both apps when it recurs.

- [ ] Watch for a physical display freeze after Mac-to-Mac USB stop/reconnect. An earlier build left the receiving Mac's panel frozen until reboot; the cause was never established. Start, stop and reconnect in both directions passed on 2026-10-24 without recurrence.

- [ ] Report real remote display capabilities (hardware modes, refresh rates, Retina) during negotiation. The desktop Display settings currently derive stream-size and frame-rate choices from the negotiated profile, and remembered lower frame rates can limit later choices.

- [ ] Decide the opt-in bounded presentation queue (`native/macos/display/PresentationQueue.swift`). In one trial it improved readable cadence but added lag and CPU. Keep immediate rendering as the default unless a targeted change wins in a repeated comparison.

- [ ] Run `display::socket_timing` tests on a Mac without a network filter (or rely on CI). Filters such as LuLu hold a new process's first datagrams and make the queued-arrival test fail locally.

- [ ] Android acceptance still pending: trackpad focus loss while a hold is in progress (notification drawer), active account-session revocation during held input, tablet-to-Mac global control over production USB, and lifecycle cases (rotation, Doze, network changes). See `mobile/PARITY.md`.

- [ ] Android distribution: production signing, versioned upgrade path, clean install and backup/restore checks before publishing the app.

- [ ] Check whether display SRTP uses hardware AES in release builds. The `aes` 0.8 crate only enables ARMv8 AES instructions with `--cfg aes_armv8`, so str0m's rust-crypto SRTP may run software AES on Apple silicon and Android. Measure per-packet cost at high bitrates before changing build flags.

- [ ] Before release: write documentation about custom account servers and point the desktop login's "Learn more" link at it (`ACCOUNT_SERVER_DOCS` in `desktop/src-tauri/src/accounts.rs`, currently `https://example.com/`).

- [ ] Connection status: tell direct internet connections apart from the relay if the transport can, and consider presented frame rate and receiver-side drops as display quality signals (the live indicator uses round trips, keepalive gaps, RTCP loss and keyframe recoveries).

- [ ] Report the Android device's own link (Wi-Fi/Ethernet) in the Hello through `link::set_classifier`, so Mac explanations cover tablets too. Android currently reports none.

- [ ] Use IPv6 link-local addresses for Mac-to-Mac cables. macOS takes about 20 seconds to self-assign an IPv4 address to a new cable (it waits for DHCP first), while `fe80::` addresses exist as soon as the link is up. Cable routes and mDNS are IPv4-only today, and IPv6 mDNS was disabled for a macOS issue that would need rechecking.

- [ ] Take the peer's Wi-Fi/Ethernet link from the display session's own handshake, as keyboard and mouse sessions do. Display sessions still use the last background presence check, so the receiving Mac can lack the peer's link for a few seconds after a restart.

- [ ] Confirm on hardware that changing an extended display's size keeps other monitors' color profiles (commit `4a804bac`): extend, change size, change back, and check that the LG ULTRAGEAR does not turn darker and warmer. A prototype with fixed sizes passed; the version that sends the settings' sizes is untested. If it still happens, fall back to recreating the virtual display on resize.
