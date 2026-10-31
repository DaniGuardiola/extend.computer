# Native helpers

[← Input control](input-control.md) · [Index](../README.md)

The app does capture, encoding and input through two helper executables per platform. Each platform implements the same command-line and stdin/stdout contract, so the Rust side stays platform-neutral.

| Helper | macOS | Windows |
| --- | --- | --- |
| Input (`ExtendComputerCursor`) | `native/macos/Input/` | `native/windows/input/` |
| Display (`ExtendComputerDisplay`) | `native/macos/Display/` | `native/windows/display/` |

Every helper exits on stdin EOF and releases anything it holds first: pressed keys and buttons, a virtual display.

## Input helper

Spawned by [`engine/src/cursor.rs`](../../engine/src/cursor.rs). A command writes one line and waits for exactly one reply line.

- `status`: prints `listen=<bool> post=<bool>` (permission to capture and to inject input).
- `inject-control`: receives input. Prints `READY <w> <h> [tokens]`, where the size is the main display in logical points. Tokens advertise `input-text-v1` and `input-shortcuts-command-v1` or `input-shortcuts-control-v1`. Each stdin line is `ALIVE`, an [`InputEvent`](../../engine/src/input.rs) as JSON, or a normalized `x y` pointer position. The helper replies `OK` only after it has applied the line. On an invalid line, a 2 s silence or a display change, it releases held input and prints `STOP`.
- `capture-control-{left,right} <remoteW> <remoteH> <offsetY>`: sends input to the peer. Prints `READY <w> <h>`, then `REMOTE`, normalized `x y` positions on the peer's display, `InputEvent` JSON lines and `LOCAL` as the pointer crosses the shared edge and returns. It finishes with `STOP <reason>`; [`engine/src/control.rs`](../../engine/src/control.rs) lists the reasons. Stdin carries `ALIVE` at least every 2 s, plus `EDGE left|right`. Events the helper injects itself are tagged, so it never captures them.
- `inject-display <descriptor>` (macOS only): applies a display viewer's input to the captured display. See [`runtime/display/input.rs`](../../desktop/src-tauri/src/runtime/display/input.rs).

Keys travel as macOS virtual key codes, which identify physical positions. Windows translates them to scancodes in `native/windows/input/src/keys.rs`. Command maps to the Windows key and Option to Alt.

## Display helper

Spawned by [`runtime/display/helper.rs`](../../desktop/src-tauri/src/runtime/display/helper.rs).

- `screen`: prints the main display as JSON `{width, height, scale}`.
- `mirror|extend|viewer W H FPS BITRATE retina|standard DISPLAY CODEC cursor|embedded`: Windows implements `mirror` only, with H.264 and the pointer embedded in the image.
- Stdout carries framed packets: `[kind u8][length u32 BE][payload]`.
  - Kind 0: the JSON startup message.
  - Kind 1: an encoded frame, `[timestamp ns u64 BE][keyframe u8][Annex B]`. Keyframes carry their parameter sets.
  - Kind 2: a JSON control message, such as `configured`, `keyframe` or `target`.
  - Kind 3: a pointer.
  - Kind 4: viewer input.
- A source reads stdin as JSON lines: `keyframe`, `pause`, `bitrate`, `configuration` (answered by `configured`) and `stop`.

---

[← Input control](input-control.md) · [Index](../README.md)
