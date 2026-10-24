# Sharing modes

[← Code map](code-map.md) · [Index](../README.md) · [Platforms and adapters →](platforms.md)

The product covers four ways of using devices together. Device roles describe the direction of data and authority; the same device may take different roles in different sessions. Each mode depends on the capabilities available on both endpoints.

## Share input

One device supplies keyboard and mouse input; another receives that input. Input ownership can move between devices without presenting the receiver's screen on the sender.

> [!NOTE]
> macOS only. Other platforms **not supported yet**.

The current implementation uses screen-edge handoff, ordered encrypted events, native permission checks, and emergency stop. See [input sessions](input-control.md) for that concrete path.

## Extend display

One device supplies an additional logical display; another presents it. This expands the source device's desktop rather than duplicating an existing screen.

> [!NOTE]
> Experimental macOS LAN and USB-network support.

The source creates a virtual display and captures it with ScreenCaptureKit. VideoToolbox uses hardware HEVC encoding by default, with H.264 available through codec negotiation, WebRTC carries encrypted video directly between devices, and the receiving Mac decodes and presents it with Metal. The source needs Screen Recording permission. Internet connectivity and automatic reconnection for display streaming are not implemented yet.

## Mirror screen

One device supplies an image of an existing display; another presents the image. Mirroring duplicates a screen rather than creating additional desktop space. Displaying an image does not itself authorize input control.

> [!NOTE]
> Experimental macOS LAN and USB-network support.

Mirroring uses the same video pipeline as extend display, capturing the source's main display instead of creating a virtual display.

Both display viewers can request a separate authenticated input lease when the user deliberately clicks, scrolls, or types. The source must grant the negotiated capability and pass its native permission checks. Input remains limited to the current display geometry, and losing viewer focus releases control. See [display streaming](../development/display.md#viewer-input) for lifecycle and text-input details.

## Remote desktop

A device presents another device's desktop and supplies authorized control input. Viewing and controlling are distinct capabilities: permission to see a screen must not be treated as permission to control it.

> [!NOTE]
> **Not supported yet.**

## Shared boundaries

Device discovery, cryptographic identity, pairing or account membership, OS permissions, routing, and session lifecycle apply across all sharing modes.

---

[← Code map](code-map.md) · [Index](../README.md) · [Platforms and adapters →](platforms.md)
