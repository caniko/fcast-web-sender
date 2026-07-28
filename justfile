default:
    just check

generate:
    node tools/generate-bridge.mjs

schema-check:
    node tools/validate-schemas.mjs

rust-fmt:
    cargo fmt --all -- --check

rust-check:
    cargo check --workspace --all-targets

rust-test:
    cargo test --workspace

rust-clippy:
    cargo clippy --workspace --all-targets -- -D warnings

ts-check:
    nix shell nixpkgs#nodejs nixpkgs#pnpm nixpkgs#esbuild -c pnpm check

check: schema-check generate rust-fmt rust-check rust-test rust-clippy ts-check
