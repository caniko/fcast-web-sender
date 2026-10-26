# Packaging

`pnpm package` produces `dist/extensions/chromium` and
`dist/extensions/firefox`. The native companion is built as
`fcast-companion` by Cargo/Nix. Browser native-host manifests are kept under
`installers/` and are rendered with the companion's absolute path at install
time; the repository never embeds a developer machine path.

The flake also exposes rs-harbor cross outputs for
`fcast-companion-aarch64-linux` and `fcast-companion-windows`. macOS outputs
remain gated on a realized Apple SDK rather than silently producing an
unusable artifact.

Branch CI uploads build artifacts for review. A `v*` tag runs the Simit
release-parity app, builds the companion through the pinned rs-harbor Nix
derivation, packages both extensions, and publishes a GitHub release
containing the Linux archive, checksums, and SBOMs. Release signing and
browser-store submission remain explicit follow-up work.

`homeManagerModules.default` installs the companion and registers the native
messaging host for Chromium, Brave and Firefox:

```nix
programs.fcast-web-sender.enable = true;
```

`package`, `chromiumExtension` and `firefoxExtension` default to this flake's
source builds; point them at release assets to avoid a local build. The
unpacked extensions land in `$XDG_DATA_HOME/fcast-web-sender/extensions`.
