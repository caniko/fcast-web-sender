default:
    just check

generate:
    node tools/generate-bridge.mjs

schema-check:
    node tools/validate-schemas.mjs

rust-fmt:
    cargo fmt --all -- --check

rust-check:
    cargo check --workspace --all-targets --all-features

rust-test:
    cargo test --workspace --all-features

rust-clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

rust-doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
    cargo test --workspace --doc --all-features

security-check:
    cargo deny check advisories bans licenses sources
    cargo audit

ts-check:
    nix shell nixpkgs#nodejs nixpkgs#pnpm nixpkgs#esbuild -c pnpm check

check: schema-check generate rust-fmt rust-check rust-test rust-clippy rust-doc ts-check
