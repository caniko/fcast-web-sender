# Architecture

The project has two strict halves:

1. `packages/` contains browser-independent sender logic, content detection,
   UI, and thin Chromium/Firefox adapters.
2. `crates/` contains the local companion, discovery, trust, FCast transport,
   FCompanion lease helpers, and bounded media helpers.

The native-messaging channel carries JSON control envelopes only. Media bytes
are either sent directly by the receiver from a public media URL or exposed to
the pinned FCast SDK through its FCompanion provider using an ephemeral local
file lease. The standalone loopback HTTP server in `crates/fcompanion` is a
bounded fixture for range/expiry tests, not a general-purpose proxy. A request
follows this order:

`user action → detector → bridge → trust gate → direct FCast → FCompanion → element capture → tab mirroring`

Each fallback is explicit and observable in diagnostics; a fallback is not a
permission to bypass receiver trust or media safety checks.
