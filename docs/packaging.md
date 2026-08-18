# Packaging

`pnpm package` produces `dist/extensions/chromium` and
`dist/extensions/firefox`. The native companion is built as
`fcast-companion` by Cargo/Nix. Browser native-host manifests are kept under
`installers/` and are rendered with the companion's absolute path at install
time; the repository never embeds a developer machine path.

`nix build .#release-bundle` publishes the Linux musl companion (with
`install.sh`), a Windows zip (with `install.ps1`), and both unsigned extension
zips. macOS outputs remain gated on a realized Apple SDK.

A `0.2.0` tag publishes those artifacts to GitHub Releases. Chrome Web Store
and AMO submission is manual. Listing copy lives in `docs/store/listing.md`.
The privacy policy is `docs/privacy.md`.
