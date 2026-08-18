# Changelog

All notable changes to FCast Web Sender will be documented here.

## [Unreleased]

## [0.2.0] - 2026-08-17

### Changed

- Receiver trust now confirms the discovered fingerprint and persists
  failure-atomically.
- Session controls are validated before FCast side effects.
- Credential forwarding now uses short-lived receiver- and origin-bound leases.
- Bridge limits, upstream events, mDNS handling, and manifest parsing are
  stricter and typed.
- Store packaging includes icons, honest permission copy, and Linux/Windows
  companion installers. Tab capture is not part of this listing.

## [0.1.0] - 2026-07-28

### Added

- Privacy-preserving browser sender extensions for Chromium and Firefox.
- A statically linked FCast companion binary for Linux x86_64.
- Native-messaging installer templates for the supported desktop platforms.
