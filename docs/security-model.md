# Security model

Receiver identity is pinned to the SHA-256 fingerprint of the certificate
SPKI. Discovery produces an untrusted receiver record. The user must approve
the displayed fingerprint before connection, and the approved value must equal
the current discovery record. Forgetting a receiver removes its
persistent entry. The trust file is written atomically with mode `0600`.

The direct FCast adapter uses the pinned upstream sender SDK at the exact
revision recorded in `Cargo.lock`. The SDK's FCast v4 path upgrades to TLS 1.3
and rejects insecure downgrade when a fingerprint is present.

Manifest and relay fetching accepts HTTP(S) URLs without embedded credentials,
refuses redirects, pins each resolved hostname to a checked public address for
the request, bounds response size, and rejects private/local targets by
default. HLS encrypted-key declarations and DASH content protection are
rejected. This project does not decrypt DRM, proxy key material, or attempt
circumvention.

Credentialed relay requests require an explicit browser permission and a
companion-issued lease. A lease contains only Cookie and Authorization headers,
is held in memory, expires within five minutes, is single-use, and is bound to
one trusted receiver fingerprint and exact HTTPS origin. Credentials are never
persisted or included in diagnostics. Authorization-only sites may need to
issue another media request after permission is granted so the extension can
observe the header.

For direct casting, the receiver resolves and fetches the URL. The companion's
DNS pinning and SSRF policy apply only to requests made by the companion; the
user-approved receiver is a separate trust boundary.
