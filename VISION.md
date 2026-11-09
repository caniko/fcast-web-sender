# FCast Web Sender vision

FCast Web Sender is a browser extension plus a local companion binary for
sending explicitly selected web media to FCast receivers. A user picks media in
a Chromium or Firefox tab and casts it to a receiver they have personally
trusted on their own network; discovery, trust, and protocol transport stay on
the device, and media bytes never travel through the native-messaging channel.

## Problem

Casting from a browser usually means either routing traffic through a cloud
relay or handing URLs, cookies, and credentials to an opaque sender. Receivers
on a local network are also discovered and connected without a durable notion of
"which receiver do I trust", and a careless implementation becomes an open
proxy for private addresses.

## Principles

- Trust before session: a receiver must be explicitly trusted by its SPKI
  fingerprint before a connection can be established.
- The native-messaging channel carries bounded JSON control envelopes only;
  media bytes are not pushed through it.
- Media fetched by the companion is validated for scheme, credentials,
  redirects, DNS rebinding, and private address targets.
- Credential forwarding (cookies, Authorization) requires an explicit,
  short-lived lease bound to one trusted receiver and one HTTPS origin.
- Fallbacks are explicit and ordered, and each is observable in diagnostics:
  user action, detector, bridge, trust gate, direct FCast, FCompanion, element
  capture, tab mirroring. A fallback never becomes permission to bypass trust or
  media safety checks.
- Browser-independent core first: the extension logic lives in shared packages,
  and the Chromium and Firefox adapters are thin surfaces over it.
- The bridge schema in `schemas/` is canonical; generated TypeScript and Rust
  representations are checked in and refreshed through `just generate`.

## Scope

Browser-independent TypeScript packages (extension core, content, UI, bridge
types, per-browser adapters) and Rust crates for the companion, discovery,
receiver trust, FCast client, FCompanion leases, media fetching, manifest
rewriting, diagnostics, and a mock receiver; installers; schemas; and design
documentation under `docs/`.

## Non-goals

DRM, encrypted media keys, and circumvention are intentionally unsupported.
There is no cloud relay. The companion does not take over fetching media that a
trusted receiver can fetch directly, and the loopback FCompanion HTTP server is
a bounded fixture rather than a general-purpose proxy.

## Direction

The unreleased work has tightened the security boundary rather than widening
the feature list: failure-atomic fingerprint trust, pre-side-effect session
validation, receiver- and origin-bound credential leases, stricter bridge and
mDNS handling, and bounded session-scoped relays for authenticated HLS and DASH
streams.

## Place in the fleet

Canix packages the released companion binary and Chromium extension
(`root/packages/fcast_web_sender`), and its Home Manager module
`programs.fcast-web-sender` installs the extension package, native-messaging
manifest, and host registration on desktop hosts.
