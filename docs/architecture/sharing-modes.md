# Sharing modes

[← Code map](code-map.md) · [Contents](../README.md) · [Platforms and adapters →](platforms.md)

The product covers four ways of using devices together. Device roles describe the direction of data and authority; the same device may take different roles in different sessions. Each mode depends on the capabilities available on both endpoints.

## Share input

One device supplies keyboard and mouse input; another receives control. Input ownership can move between devices without presenting the receiver's screen on the sender.

> [!NOTE]
> Implemented for macOS devices. Native input adapters for other platforms are not supported yet.

The current implementation uses screen-edge handoff, ordered encrypted events, native permission checks, and emergency stop. See [input sessions](input-control.md) for that concrete path.

## Extend display

One device supplies an additional logical display; another presents it. This expands the source device's desktop rather than duplicating an existing screen.

> [!NOTE]
> **Not supported yet.** Virtual-display creation, display capture, media transport, and presentation are not implemented. There is no supported setup procedure or released display-extension mode.

## Mirror screen

One device supplies an image of an existing display; another presents the image. Mirroring duplicates a screen rather than creating additional desktop space. Displaying an image does not itself authorize input control.

> [!NOTE]
> **Not supported yet.** Screen capture, encoding, media transport, decoding, and presentation are not implemented.

## Remote desktop

A device presents another device's desktop and supplies authorized control input. Viewing and controlling are distinct capabilities: permission to see a screen must not be treated as permission to control it.

> [!NOTE]
> **Not supported yet.** Input sharing and internet relay exist, but a remote desktop viewer and display-streaming pipeline do not. A relay connection alone is not a remote desktop implementation.

## Shared boundaries

Device discovery, cryptographic identity, consent, OS permissions, routing, and session lifecycle apply across the product. Their concrete implementations are documented in the following chapters. Detailed media and platform implementation proposals remain in the [archive](../archived/README.md); these mode definitions do not prescribe an unimplemented protocol or release schedule.

---

[← Code map](code-map.md) · [Contents](../README.md) · [Platforms and adapters →](platforms.md)
