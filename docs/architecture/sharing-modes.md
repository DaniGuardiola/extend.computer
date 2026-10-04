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
> **Not supported yet.**

## Mirror screen

One device supplies an image of an existing display; another presents the image. Mirroring duplicates a screen rather than creating additional desktop space. Displaying an image does not itself authorize input control.

> [!NOTE]
> **Not supported yet.**

## Remote desktop

A device presents another device's desktop and supplies authorized control input. Viewing and controlling are distinct capabilities: permission to see a screen must not be treated as permission to control it.

> [!NOTE]
> **Not supported yet.**

## Shared boundaries

Device discovery, cryptographic identity, consent, OS permissions, routing, and session lifecycle apply across all sharing modes.

---

[← Code map](code-map.md) · [Index](../README.md) · [Platforms and adapters →](platforms.md)
