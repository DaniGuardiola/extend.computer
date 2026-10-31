# Native helpers

[← Input control](input-control.md) · [Index](../README.md)

The app does capture, encoding and input through two helper executables per platform. Each platform implements the same command-line and stdin/stdout contract, so the Rust side stays platform-neutral.

| Helper | macOS | Windows |
| --- | --- | --- |
| Input (`ExtendComputerCursor`) | `native/macos/input/` | `native/windows/input/` |
| Display (`ExtendComputerDisplay`) | `native/macos/display/` | `native/windows/display/` |

Every helper exits on stdin EOF and releases anything it holds first: pressed keys and buttons, a virtual display.

[`engine/src/helper/`](../../engine/src/helper/mod.rs) types the contract: `packet.rs` the display framing, `display.rs` the display helper's arguments and messages, `input.rs` the input helper's lines. The desktop runtime and both Windows helpers use it; the Swift helpers implement it by hand. Messages the app writes keep sorted keys, the bytes of the hand-built JSON they replaced, except viewer input state, which keeps its field order.

`native/shared/fixtures/` pins the wire: input events and edge-crossing cases, display helper and inject-display messages (`display-helper.json`) and framed packets (`display-packets.json`). The engine, the Windows input helper and the macOS Swift tests (`scripts/macos/test-*.sh`) replay them.

## Input helper

Spawned by [`engine/src/cursor.rs`](../../engine/src/cursor.rs). A command writes one line and waits for exactly one reply line.

- `status`: prints `listen=<bool> post=<bool>` (permission to capture and to inject input). The app skips fields it does not know.
- `inject-control`: receives input. Prints `READY <w> <h> [tokens]`, where the size is the main display in logical points. Tokens advertise `input-text-v1` and `input-shortcuts-command-v1` or `input-shortcuts-control-v1`, each at most once. Each stdin line is `ALIVE`, an [`InputEvent`](../../engine/src/input.rs) as JSON, or a normalized `x y` pointer position. The helper replies `OK` only after it has applied the line. On an invalid line, a 2 s silence or a display change, it releases held input and prints `STOP`.
- `capture-control-{left,right} <remoteW> <remoteH> <offsetY>`: sends input to the peer. Prints `READY <w> <h>`, then `REMOTE`, normalized `x y` positions on the peer's display, `InputEvent` JSON lines and `LOCAL` as the pointer crosses the shared edge and returns. It finishes with `STOP <reason>`; [`engine/src/control.rs`](../../engine/src/control.rs) lists the reasons. Stdin carries `ALIVE` at least every 2 s, plus `EDGE left|right`. Events the helper injects itself are tagged, so it never captures them.
- `inject-display <target>` (macOS only): applies a display viewer's input to the captured display, driven by [`runtime/display/input.rs`](../../desktop/src-tauri/src/runtime/display/input.rs).
  - The target is JSON with exactly `session`, `epoch`, `revision` (the input lease), `display_id`, `bounds` and `content` (`{x, y, width, height}`) and `canvas` (`{width, height}`). `bounds` is the captured display in its own coordinates; `content` is where it sits in the encoded `canvas`.
  - The helper echoes the target as `READY <json>`, adding optional `text_supported` and `shortcut_policy` (`command` or `control`). No other fields are allowed.
  - Each stdin line, at most 1024 bytes, is `{session, epoch, revision, sequence, event}`. `event` is an `InputEvent`, or `{"kind": "move"}`, `{"kind": "alive"}` or `{"kind": "release"}`; moves, buttons and scrolls also carry `x` and `y` normalized to the canvas. Sequences increase.
  - The helper answers `APPLIED {session, epoch, revision, sequence}` within 400 ms, after applying the line.

Keys travel as macOS virtual key codes, which identify physical positions. Windows translates them to and from scancodes in `native/windows/shared/keys.rs`, which the input helper and the display viewer share. Command maps to the Windows key and Option to Alt.

## Display helper

Spawned by [`runtime/display/helper.rs`](../../desktop/src-tauri/src/runtime/display/helper.rs).

- `screen`: prints the main display as JSON `{width, height, scale}`, in points. macOS adds `usable_width` and `usable_height`, the size without the notch. Windows reports physical pixels at scale 1.
- `capabilities`: prints what this build can do as one JSON line, [`Abilities`](../../engine/src/helper/display.rs): `{version, source, target, viewer, viewer_input, encode, decode, cursor_send, cursor_overlay}`. `source` lists `mirror` and `extend`, `encode` and `decode` list codec names, `target` means a source reports its capture target for viewer input, and the cursor fields cover sending and drawing kind 3 pointer packets. Unknown keys are skipped and missing ones mean the helper can't. The app asks once at launch, off the UI thread, and keeps its per-platform defaults ([`runtime/abilities.rs`](../../desktop/src-tauri/src/runtime/abilities.rs)) until the helper answers or if it can't.
- `mirror|extend|viewer W H FPS BITRATE retina|standard DISPLAY CODEC cursor|embedded`: the size is in points and Retina doubles the encoded size, up to 4096. `DISPLAY` 0 is the main display; `CODEC` is `h264` or `hevc`; `cursor` sends the pointer as kind 3 packets instead of drawing it. Windows implements `mirror` and `viewer`, with H.264 and the pointer embedded in the image.
- Stdout carries framed packets: `[kind u8][length u32 BE][payload]`, at most 16 MiB.
  - Kind 0, startup: `{"ready": true}`. A source adds `width`, `height` and `retina`; the macOS source also adds `target`, its capture target: `configuration_id`, `display_id`, `bounds` and `content` (`{x, y, width, height}`), `canvas` and `raster` (`{width, height}`, the encoded size before and after Retina).
  - Kind 1, a source's encoded frame: `[timestamp ns u64 BE][keyframe u8][Annex B]`. Keyframes carry their parameter sets.
  - Kind 2, JSON control, by `type`; the app ignores types it does not know:
    - `configured {id, ok}`: answers a configuration.
    - `keyframe`: asks for a keyframe, as a viewer does after a decode error.
    - `target {target}`: a source's new capture target, sent before `configured`.
    - `render_ready {ready}`: a viewer rendered on the GPU. This message allows no other fields.
    - `presented {frames}`: a viewer's presented frame count, for the first frame and every 60th.
    - `viewport {id, fullscreen, ...screen}`: answers a viewer's viewport request with the `screen` fields, plus `window_number` and `viewport_cg` (`{x, y, width, height}`, the video view in display coordinates).
    - `viewport_evidence {fullscreen, ...screen}`: the same geometry, sent on entering fullscreen when `EXTEND_DISPLAY_METRICS` is set.
    - `closed`: the viewer window closed.
  - Kind 3, the pointer: a shape `{revision, png, width, height, hotspot_x, hotspot_y}` (base64 PNG, at most 512 points) or a position `{revision, sequence, x, y, visible}` normalized to the canvas, at most 128 KiB. The app forwards these bytes to the viewer unchanged.
  - Kind 4, a viewer's input intent, 1 to 1024 bytes: `{"type": "focus", active, revision}` or `{"type": "input", command, revision}`, where `command` is a display input [`Command`](../../engine/src/display/input.rs).
- A source reads stdin as JSON lines and skips any it cannot read: `configuration {id, width, height, retina, fps, bitrate}` (answered by `configured`), `pause {value}`, `keyframe`, `bitrate {value}` and `stop`.
- A viewer reads framed stdin: kind 1 frames, kind 3 pointer packets and kind 2 JSON `configuration` (as above), `viewport {id}`, `input_state` and `stop`. `input_state` is `{available, enabled, focused, text_supported, shortcut_policy, content, revision}`, what the viewer may offer for input; only the source's grant enables it.
- The Windows viewer ([`native/windows/display/src/viewer`](../../native/windows/display/src/viewer/mod.rs)) follows the macOS one, with these differences:
  - A borderless window covers the primary monitor from the start. `viewport` reports its size in physical pixels at scale 1, like `screen`, and `fullscreen` once the window and its swap chain match the monitor. It ignores kind 3 packets.
  - It decodes with Media Foundation's H.264 decoder, on the GPU through DXVA when it can and in software otherwise. A frame it can't read or decode asks for a `keyframe`, and frames before the next keyframe are dropped.
  - Each key press leaves either as the text it types or as a physical `mac_key`, never both. Text needs `text_supported` and no Ctrl or Windows key held, with AltGr counting as text: printable keys, dead-key sequences and input method commits are text, and Enter, Tab, Backspace, Escape, arrows and chords are physical keys. A key-up goes out only for a key whose press did. When `shortcut_policy` is `command`, Ctrl with A, Z, X, C, V or Y becomes Command. Modifier masks carry no sides.
  - Win, Alt+Tab and other shell shortcuts stay with Windows, and losing focus gives up the lease, which releases what the source holds. Alt+F4 and Ctrl+Alt+Esc close the viewer, which reports `closed`.
  - It exits within about 2 s of `stop`, the end of stdin or its parent's exit.
- `self-test` (Windows) encodes synthetic frames with the display encoder and decodes them with the viewer's decoder, through the frame packets, on the GPU and in software, and checks the pictures' size and content without capturing or showing anything. `self-test stream W H FRAMES` prints a synthetic stream as kind 1 packets.

---

[← Input control](input-control.md) · [Index](../README.md)
