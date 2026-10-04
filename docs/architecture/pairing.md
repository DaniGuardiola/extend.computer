# Visual pairing

[← Platforms and adapters](platforms.md) · [Contents](../README.md) · [Accounts and routing →](accounts-and-routing.md)

Discovery is the default GUI path. Open Pair device on both computers, choose a device on one, and compare eight ordered symbols. Names/addresses are untrusted routing hints. A two-minute, single-attempt window gates incoming requests and renews automatically while the pairing dialog stays open. Closing pairing cancels pending approval and removes its advertisement. No automatic retry or unverified-trust fallback occurs. Manual SPAKE2 code pairing remains available.

EXTEND04 mode S uses Noise XX with a distinct visual-pairing prologue. After the handshake, each role generates a fresh 32-byte OS-random nonce and sends SHA-256(domain || handshake hash || role || nonce). Both commitments must be received before either nonce is revealed. Each side verifies the reveal. The SAS is SHA-256(separate domain || handshake hash || initiator nonce || responder nonce), rendered using eight six-bit indices from the first eight digest bytes: 48 bits. The handshake hash binds both static identities and ephemeral keys. A fixed 64-symbol alphabet supplies numbered labels for cross-platform and accessible comparison.

Each user explicitly confirms. Each side sends encrypted confirmation and waits for its peer's confirmation before persisting trust. Denial closes the socket, letting pending GUI approval detect EOF. Invalid commitments, protocol messages, transport loss, and timeout fail closed. The pairing connection ends without carrying input. Later desktop control still requires receiving to be enabled and effective OS permissions. Manually paired devices follow the desktop pairing-authorized control flow; account-only devices have a separate local control approval. A completion-time transport/storage failure can leave only one device remembered; retry needs a fresh local pairing window.

This is a extend.computer-specific protocol, not Matrix interoperability or an audited implementation. The commitment-before-reveal principle and symbol UX are informed by the [Matrix SAS specification](https://spec.matrix.org/v1.9/client-server-api/). Independent cryptographic review remains necessary before production security assurance. Never replace comparison with plain acceptance, derive symbols from discovery metadata, or trust a familiar discovery name.

Tests cover matching, either-side refusal, closed/expired windows, altered commitments, no trust before both approvals, cancellation, and desktop loopback lifecycle. Browser mocks cover discovery, self filtering, symbol layout, successful dismissal and manual fallback. Live two-device verification remains pending.

## Bilateral unpair

Mode U uses a separate Noise prologue. The initiator pins the remote identity before RequestUnpair; the receiver removes only the authenticated sender’s record. It grants no probe/input access. Duplicate requests are idempotent and revocation blocks remain intact. Local removal completes before a best-effort notification; failed notifications retain an explicit retry path in device removal state. The initiating device removes its row immediately. The receiving device retains metadata for a muted grey dismissible placeholder.

Mode R supports a read-only QueryPairing / PairingConfirmed exchange and an encrypted NotPaired reply. The caller verifies the saved identity before interpreting either reply. Neither silence nor transport/protocol errors are evidence of unpairing. Idle GUI checks run every 90 seconds; explicit connection attempts handle NotPaired too. GUI session epochs reject replies that predate a newer local session/pairing. A disconnected peer cannot learn of removal until receiving is enabled and contact succeeds. These guards are scoped to the GUI process; concurrent external CLI modifications of the same profile are not a supported pairing workflow.

GUI discovery uses `_extendpair._tcp.local.` only while the pairing dialog is open and the listener is idle. A per-process discovery identifier plus local-address/loopback checks suppress self entries. These are untrusted filtering hints, never authenticated identity. Background receiving alone does not advertise. Peers may take one discovery refresh to remove a withdrawn record. CLI probe discovery remains a separate service.

After manual or visual pairing, GUI peers exchange bounded display names through a fresh pinned, encrypted reconnect handshake. The receiver only accepts names from already paired identities. Names grant no control and are not identity verification. Address-generated placeholders update; user-chosen names remain unchanged. Idle peer checks retry the metadata exchange to repair older placeholder entries. Unsupported/failed metadata exchange leaves pairing intact.

---

[← Platforms and adapters](platforms.md) · [Contents](../README.md) · [Accounts and routing →](accounts-and-routing.md)
