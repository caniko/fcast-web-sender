# FCast Web Sender

<!-- simit:badges:start -->

[![CI](https://img.shields.io/badge/CI-managed+extra-2088ff)](.github/workflows/ci.yaml) [![Nix](https://img.shields.io/badge/Nix-managed-5277c3)](flake.nix) [![crates.io](https://img.shields.io/badge/crates.io-ready-f46623)](https://crates.io/crates/fcast-bridge) [![artifacts](https://img.shields.io/badge/artifacts-configured-2ea44f)](.github/workflows/release.yml)

<!-- simit:badges:end -->

FCast Web Sender is a browser extension and local companion for sending
explicitly selected web media to FCast receivers. It keeps discovery,
receiver trust, and protocol transport local to the device. The browser
extension never sends media bytes through the native-messaging channel.

Install the extension, then install the Linux or Windows companion from
[GitHub Releases](https://github.com/caniko/fcast-web-sender/releases) with
`install.sh` or `install.ps1`. macOS is not included in 0.2.0. Privacy:
[docs/privacy.md](docs/privacy.md).

The workspace is deliberately split between browser-independent TypeScript
packages and Rust companion crates. Chromium and Firefox adapters are thin
surfaces over the same extension core.

## Development

The reproducible development environment is provided by Nix and rs-harbor:

```sh
nix develop
just check
```

Frontend checks use the Nix-pinned Node, pnpm, and esbuild tools:

```sh
nix shell nixpkgs#nodejs nixpkgs#pnpm nixpkgs#esbuild -c pnpm package
```

The bridge schema is canonical in `schemas/`. `just generate` produces the
checked-in TypeScript and Rust representations used by the bridge packages.

## Safety boundaries

- A receiver must be explicitly trusted by its SPKI fingerprint before a
  session can connect.
- Native-messaging frames have a bounded size and carry control messages only.
- Companion-fetched media is validated for scheme, credentials, redirects,
  DNS rebinding, and private address targets. Direct media fetching remains a
  responsibility of the explicitly trusted receiver.
- Cookie and Authorization forwarding requires an explicit, short-lived lease
  bound to one trusted receiver and HTTPS origin.
- DRM, encrypted media keys, and circumvention are intentionally unsupported.
- Discovery and media control are local; there is no cloud relay.
