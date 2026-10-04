> Archived proposal or historical report. Claims and instructions describe the document’s original milestone, not the current app. See the [current documentation](../README.md).

> 🤖🔧 ai generated

# local-first, cross-platform input and display sharing

initial implementation: mac ↔ mac. shared rust engine, tauri settings ui, thin native adapters. future cross-platform and internet connections are architectural requirements, not promises of first-release support.

this is a design proposal based on primary documentation and selected input-protocol source inspection. alternatives have not been benchmarked on these two macs, and their security implementations have not been audited here.

## what to learn from alternatives

| reference | verified design | proposed takeaway |
|---|---|---|
| [lan mouse protocol](https://raw.githubusercontent.com/feschber/lan-mouse/main/lan-mouse-proto/src/lib.rs), [project](https://github.com/feschber/lan-mouse) | small typed input messages; enter/leave/ack and ping/pong; dtls and authorized fingerprints described by project | compact input path, explicit handoff/liveness, known-device authentication. add capability negotiation and mac scroll fidelity to our own design |
| [deskflow protocol](https://raw.githubusercontent.com/deskflow/deskflow/master/src/lib/deskflow/ProtocolTypes.h) | version handshake; screen-entry coordinates/sequence/modifier state; physical key identity used for key release | preserve physical key identity separately from text/layout; reconcile modifiers; make ownership transitions explicit |
| [sunshine](https://docs.lizardbyte.dev/projects/sunshine/latest/), [moonlight](https://github.com/moonlight-stream/moonlight-qt) | sunshine documents mac ScreenCaptureKit and VideoToolbox support; moonlight documents hardware decode, multiple codecs and 4:4:4 support | hardware media pipeline with native surfaces; benchmark text clarity/chroma as well as motion. codec support must be negotiated per device; documented capabilities do not guarantee every hardware combination |
| [rustdesk](https://rustdesk.com/docs/en/self-host/) | separate rendezvous and relay services; attempts direct connection, relays on failure | optional self-hostable remote infrastructure; same application trust model regardless of route |
| [syncthing](https://docs.syncthing.net/users/security.html) | tls certificate fingerprints checked against accepted devices; relays forward encrypted data; documents discovery metadata leakage | durable device identity, explicit trust list, revocation; distinguish discovery metadata from encrypted content |
| [magic wormhole](https://magic-wormhole.readthedocs.io/en/latest/welcome.html) | human-readable one-time codes bootstrap encrypted connections using pake; separate rendezvous/transit roles | short-code pairing backed by established cryptography, not a home-made pin check. do not copy its server requirement into local pairing |

these are useful patterns, not a claim that combining implementations produces a secure product. implement a coherent protocol and threat model; reuse mature libraries where appropriate. copying source is a separate decision requiring compatible project licensing.

## product rules

**discovery, identity, permission and routing are separate.**

- discovery finds a candidate endpoint. same wi-fi, matching name, account membership or remembered ip grants no authority.
- identity proves which cryptographic device is connected. names are labels; keys establish identity.
- permission determines what that device may do, in which direction, for how long.
- routing chooses lan, direct remote or relay. changing route cannot broaden permission or weaken authentication.

local operation needs no account, internet, public rendezvous server or subscription check. nearby pairing/discovery uses bonjour/mdns. manual address entry handles networks that block multicast, with identical authentication. advertise minimal information; detailed device information is exchanged only after authentication. local discovery is still observable on the lan.

new-device pairing is enabled by a user-opened, time-limited pairing window. already paired devices can reconnect through a separate authenticated path while that window is closed. suppress unsolicited permission popups from arbitrary network clients; only successfully authenticated pairing attempts can reach the approval stage.

## one-off flow

1. on receiving/controlled mac, choose the intended operation and open pairing window.
2. other mac selects nearby device and enters a fresh word code shown by first mac. later offer qr/invite as an alternative bootstrap, without requiring cameras on laptops.
3. use a vetted password-authenticated key exchange implementation. pair code is not a reusable password or the encryption key. enforce expiration, bounded attempts, single-use consumption and concurrent-attempt limits.
4. cryptographically bind device identity keys, roles, protocol version and session context to the authenticated pairing exchange. exact library/protocol integration must be reviewed before implementation is called secure.
5. approval screen names requested action and direction, e.g. “Allow this Mac to control your keyboard and mouse for this session?” or “Share this display with the other Mac?” consent is required on each side whose input/display access is granted.
6. default grant is this session only. ending session invalidates grant and resume credentials. brief interrupted reconnection may resume only that still-active session using bounded, authenticated resumption; it cannot become permanent trust.

short code possession alone never silently grants remote control. display names are untrusted text; render as text, truncate safely, and do not let them impersonate trusted ui. code guessing, code leakage and approval phishing remain threat cases to test.

## long-term flow

“remember this device” stores verified device identity. default remains ask-per-session. separate option enables automatic reconnect for selected capabilities while both user sessions are unlocked. this supports daily laptop pairing without re-entering codes or silently granting all future features.

grants are directional: control this mac, view selected screen, receive an extended display, clipboard read/write. additional capabilities require new consent. virtual-display creation and screen content sharing are separately enforced from input injection. input control is powerful and can operate apps; describing clipboard as disabled does not make full remote control harmless.

store local identity secret in mac keychain; authenticate later sessions with pinned peer identities and standard encrypted transport. changed identity requires explicit re-pairing. never fall back to trusting the same hostname or ip. avoid automatic identity export/sync in first version.

disconnect stops current session. revoke device also removes future access and invalidates resumptions, including already-connected sessions. grant validity is checked when opening every input/media/clipboard channel and after permission changes. visible connection indicator, local stop action and escape shortcut remain available. lock/sleep/user-switch ends or suspends access according to explicit policy; first version does not support unattended locked-screen access.

## remote access without cloud dependence

remote access disabled by default. first mac release uses lan; engine accepts authenticated endpoints independently of discovery. later allow direct connections over user-configured networks, then optional rendezvous/nat traversal and encrypted relay fallback.

preserve authenticated end-to-end identity across direct and relayed routes. rendezvous may offer addresses but cannot authorize devices or replace their keys. relays forward ciphertext and still observe metadata such as ip addresses, timing and traffic volume. relay access needs authentication, quotas and rate limits to prevent becoming an open bandwidth service. remote discovery must not become a public directory of stable device ids.

lan availability always works independently of remote service availability. opt-in remote capability is explicit per device/grant; no silent router port forwarding. remote invitations are short-lived and one-use; their expiry does not implicitly revoke a separately approved remembered-device grant.

## implementation boundaries

rust engine owns authenticated session state and capability enforcement. ui requests actions through restricted local ipc; it cannot bypass authorization. bind secondary media/input connections to the authenticated session, preventing a random socket from joining an approved session. transport encryption alone is not authorization.

native adapter receives only validated, authorized requests. maintain bounded packet/frame sizes and queues, reject malformed/non-finite coordinates, limit decode dimensions and resource allocation, and isolate media decoding where practical. input control runs without a network-facing root daemon. keep sensitive keys, pair codes, typed text, clipboard contents and frame contents out of diagnostics.

use standard cryptographic protocols/libraries; no custom cipher or ad hoc authenticated handshake. transport and pake library selection remains a feasibility/security-review task. handle replay, downgrade, channel binding, key confirmation, resumption and identity changes explicitly. reject replayable early-data control actions.

## mac-first milestones and acceptance gates

1. pairing/session prototype on two macs: no input or video yet. prove offline pairing, wrong-code rejection, expiry, one-use behavior, consent and revoke.
2. input sharing through mac adapters: native events, scroll fidelity, handoff, held-key recovery and escape shortcut; compare against duet and an existing open-source tool under identical conditions.
3. in parallel as a workstream, early virtual-display/media feasibility prototype: capture, hardware encode/decode, native presentation, text quality and queue latency. mac virtual-display compatibility is still an unresolved dependency from previous investigation.
4. combine display modes with capability-scoped sessions; test that view-only grants cannot inject input and input-only grants cannot access frames.
5. second operating-system adapter before freezing protocol; optional internet rendezvous/relay after authentication and permission model passes review.

security acceptance cases: hostile mdns advertisement, same-name impersonator, incorrect/expired/reused code, concurrent pairing attempts, identity replacement, injected/replayed packets, reconnect after revocation, mode escalation, session-channel hijack, malformed input/video, abusive relay client, ui injection via peer name, lock transition and abrupt disconnect with modifiers/buttons held. test rate limits without relying only on source ip. fuzz parsers/state transitions, audit dependencies and arrange independent review before broad remote-access release.

cannot promise “no security holes.” target is explicit threat model, conservative grants, standard cryptography, reproducible negative tests and ongoing review. those requirements belong in first prototype, not a later security add-on.
