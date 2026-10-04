# Keyboard and mouse sessions

[← Accounts and routing](accounts-and-routing.md) · [Index](../README.md) · [macOS permissions →](macos-permissions.md)

Input sharing has two roles: a device supplies input, and another receives that input. The shared engine carries authorized, ordered events; platform adapters capture and apply them.

> [!NOTE]
> macOS only. Other platforms **not supported yet**.

The current sender captures input locally and forwards it through the authenticated Rust session. The receiver validates messages and posts events through its native helper. The desktop app and CLI share this engine.

## Handoff

The configured left or right edge determines where control crosses to the peer. With multiple local displays, capture uses the screen containing the cursor. Only exposed outer edges hand off; seams between local screens preserve normal local movement. Return restores the cursor to the originating display. Display-layout changes stop capture so the user can reconnect with the new layout.

Release held keys and buttons before crossing. A remote drag stays on the receiver until its button is released. Returning locally releases the receiver's held input state. Control-Option-Escape is the emergency stop shortcut.

## Event ordering and backpressure

[`engine/src/control.rs`](../../engine/src/control.rs) reads native helper output into a bounded queue. Adjacent mouse positions coalesce to the latest position; key and button transitions remain ordered. Queue overflow ends the session rather than dropping a transition and leaving input held.

The wire window permits four pending input events. A probe acknowledges preceding events after four updates or when pending work reaches the acknowledgement interval. Sequence checks on the receiver reject unexpected ordering. Acknowledgement confirms receiver processing, not display presentation or input-to-photon latency.

[`engine/src/session.rs`](../../engine/src/session.rs) owns authorization, messages, and pinned session behavior. [`engine/src/cursor.rs`](../../engine/src/cursor.rs) owns native helper startup and command acknowledgement. The native implementation is in [`InputReceiver.swift`](../../native/macos/InputReceiver.swift) and [`EdgeCapture.swift`](../../native/macos/EdgeCapture.swift).

## Liveness and recovery

Heartbeats keep native capture and injection alive. Native helpers stop on missing heartbeats and release held input. Unexpected capture shutdowns report their cause, including display changes or lost capture access.

After an established connection suffers a transport failure, the desktop sender retries up to three times with 1-, 2-, and 4-second backoff. It pins the same peer and requests already remembered control consent. Each replacement capture starts locally with fresh input state. Canceling the job also cancels reconnect. Permission, identity, helper, and protocol failures are terminal.

## Input scope

The adapter forwards mouse movement, supported buttons, pixel scrolling, modifiers, and tagged Mac virtual key codes. Receiver keyboard layout determines characters.

Clipboard sharing and file transfer are not supported yet.

---

[← Accounts and routing](accounts-and-routing.md) · [Index](../README.md) · [macOS permissions →](macos-permissions.md)
