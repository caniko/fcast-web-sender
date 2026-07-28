{
  description = "Privacy-preserving browser sender for FCast receivers";

  nixConfig = {
    extra-substituters = ["https://attic.candee.baby/canix"];
    extra-trusted-public-keys = [
      "canix:lPzPzKrmYqW5Rxa5r0uQWvCqD3S5nx0h2eCy7XD5JM8="
      "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
    ];
  };

  inputs = {
    rs-harbor.url = "github:caniko/rs-harbor/0b5c6f3651f2d07f71c40f7880fa89d53420c2e0";
    nixpkgs.follows = "rs-harbor/nixpkgs";
    rust-overlay.follows = "rs-harbor/rust-overlay";
    crane.follows = "rs-harbor/crane";
    flake-utils.url = "github:numtide/flake-utils";

    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    rs-harbor,
    rust-overlay,
    crane,
    flake-utils,
    treefmt-nix,
    git-hooks,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [(import rust-overlay)];
      };
      lib = pkgs.lib;
      version = "0.1.0";
      pname = "fcast-companion";

      toolchain = rs-harbor.lib.mkToolchain {inherit pkgs;};
      inherit (toolchain) craneLib buildCache;
      rustToolchain = toolchain.rustToolchain;
      cross = rs-harbor.lib.mkCross {inherit pkgs system;};
      rustSrc = craneLib.cleanCargoSource ./.;
      commonArgs = {
        src = rustSrc;
        strictDeps = true;
        cargoExtraArgs = "--package fcast-companion --bin fcast-companion";
      };

      crossPackages = rs-harbor.lib.mkCrossPackages {
        inherit pkgs cross pname commonArgs buildCache;
        inherit craneLib;
        targets =
          ["native" "aarch64-linux" "windows" "darwin-x86_64" "darwin-aarch64"]
          ++ lib.optional (system == "x86_64-linux") "x86_64-linux-musl";
        targetArgs.windows = {
          nativeBuildInputs = [cross.mingwBinutils];
          doCheck = false;
        };
      };

      frontendSrc = lib.cleanSourceWith {
        src = ./.;
        filter = path: type: let
          name = baseNameOf path;
        in
          !builtins.elem name [".git" "dist" "node_modules" "target"];
      };

      frontendPackage = pkgs.stdenvNoCC.mkDerivation {
        pname = "fcast-web-sender-extensions";
        inherit version;
        src = frontendSrc;
        nativeBuildInputs = [
          pkgs.coreutils
          pkgs.findutils
          pkgs.nodejs
          pkgs.pnpm
          pkgs.pnpmConfigHook
          pkgs.zip
        ];
        pnpmDeps = pkgs.fetchPnpmDeps {
          pname = "fcast-web-sender-node-deps";
          inherit version;
          src = frontendSrc;
          fetcherVersion = 4;
          hash = lib.fakeHash;
        };
        buildPhase = "pnpm check && pnpm package";
        installPhase = ''
          set -euo pipefail
          mkdir -p "$out/extensions"
          for browser in chromium firefox; do
            extension="dist/extensions/$browser"
            find "$extension" -type f -exec touch --date='@1' {} +
            (cd "$extension" && zip -X -qr "$out/$browser.zip" .)
            cp -R "$extension" "$out/extensions/$browser"
          done
        '';
      };

      binaryRelease =
        if system == "x86_64-linux"
        then
          rs-harbor.lib.mkBinaryRelease {
            inherit pkgs pname version;
            artifacts.x86_64-linux-musl = {
              package = crossPackages.fcast-companion-x86_64-linux-musl;
              system = "x86_64-linux";
              rustTarget = "x86_64-unknown-linux-musl";
              binutils = pkgs.pkgsStatic.stdenv.cc.bintools;
              strip = "${pkgs.pkgsStatic.stdenv.cc.bintools}/bin/x86_64-unknown-linux-musl-strip";
              readelf = "${pkgs.pkgsStatic.stdenv.cc.bintools.bintools}/bin/x86_64-unknown-linux-musl-readelf";
              binaries = ["fcast-companion"];
            };
          }
        else null;

      binaryArchiveName = "${pname}-${version}-x86_64-linux-musl.tar.gz";
      releaseArtifacts =
        if binaryRelease == null
        then {}
        else {
          binary = rs-harbor.lib.mkReleaseArtifact {
            inherit pkgs pname version;
            name = binaryArchiveName;
            source = binaryRelease.archives.x86_64-linux-musl;
            sourcePath = binaryArchiveName;
            kind = "binary-archive";
            format = "tar.gz";
            system = "x86_64-linux";
            rustTarget = "x86_64-unknown-linux-musl";
            validation = "static-archive";
            consumable = true;
          };
          chromium = rs-harbor.lib.mkReleaseArtifact {
            inherit pkgs pname version;
            name = "fcast-web-sender-chromium-${version}.zip";
            source = frontendPackage;
            sourcePath = "chromium.zip";
            kind = "browser-extension";
            format = "zip";
            consumable = true;
          };
          firefox = rs-harbor.lib.mkReleaseArtifact {
            inherit pkgs pname version;
            name = "fcast-web-sender-firefox-${version}.zip";
            source = frontendPackage;
            sourcePath = "firefox.zip";
            kind = "browser-extension";
            format = "zip";
            consumable = true;
          };
        };

      treefmtEval = treefmt-nix.lib.evalModule pkgs (import ./nix/treefmt.nix);
      preCommitCheck = git-hooks.lib.${system}.run {
        src = ./.;
        hooks = import ./nix/pre-commit.nix {
          inherit pkgs;
          treefmtWrapper = treefmtEval.config.build.wrapper;
          inherit rustToolchain;
        };
      };

      releaseSmoke = pkgs.writeShellApplication {
        name = "fcast-release-smoke";
        runtimeInputs = with pkgs; [coreutils findutils gnutar gzip jq];
        text = ''
          set -euo pipefail
          version="''${1:?version is required}"
          release_dir="''${2:-release}"
          manifest="$(find "$release_dir" -maxdepth 1 -name '*-release-manifest.json' -print -quit)"
          test -n "$manifest"
          jq -e --arg version "$version" '.schemaVersion == 2 and .version == $version' "$manifest" >/dev/null
          archive="$(find "$release_dir" -maxdepth 1 -name 'fcast-companion-*.tar.gz' -print -quit)"
          test -n "$archive"
          stage="$(mktemp -d)"
          trap 'rm -rf "$stage"' EXIT
          tar -xzf "$archive" -C "$stage"
          test -x "$stage/bin/fcast-companion"
          "$stage/bin/fcast-companion" --version
        '';
      };
    in {
      packages =
        crossPackages
        // {
          default = crossPackages.${pname};
          fcast-web-sender = crossPackages.${pname};
          extensions = frontendPackage;
        }
        // lib.optionalAttrs (binaryRelease != null) {
          release-bundle = rs-harbor.lib.mkReleaseBundle {
            inherit pkgs pname version;
            artifacts = releaseArtifacts;
          };
          fcast-companion-x86_64-linux-musl = crossPackages.fcast-companion-x86_64-linux-musl;
        };

      apps.release-smoke = {
        type = "app";
        program = "${releaseSmoke}/bin/fcast-release-smoke";
      };

      checks = {
        default = crossPackages.${pname};
        formatting = treefmtEval.config.build.check self;
        fmt = craneLib.cargoFmt {src = rustSrc;};
        clippy = craneLib.cargoClippy (commonArgs
          // {
            cargoArtifacts = craneLib.buildDepsOnly commonArgs;
            cargoClippyExtraArgs = "--workspace --all-targets --all-features -- --deny warnings";
          });
        extensions = frontendPackage;
      };

      formatter = treefmtEval.config.build.wrapper;

      devShells = rs-harbor.lib.mkDevShells {
        inherit pkgs cross;
        inherit craneLib;
        packages = with pkgs;
          [
            cargo-deny
            cargo-sbom
            esbuild
            file
            jq
            nodejs
            pnpm
            rust-analyzer
            taplo
            unzip
            zip
          ]
          ++ preCommitCheck.enabledPackages;
        extraShellHook = preCommitCheck.shellHook;
      };
    });
}
