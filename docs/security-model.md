# Security model

Receiver identity is pinned to the SHA-256 fingerprint of the certificate
SPKI. Discovery produces an untrusted receiver record. The user must approve
the fingerprint before connection; forgetting a receiver removes its
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
