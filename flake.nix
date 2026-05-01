{
  description = "Build environment for judicious";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      flake-utils,
      ...
    }:
    let
      pythonVersionRaw = builtins.readFile ./.python-version;
      pythonVersion = builtins.replaceStrings [ "\n" "\r" " " ] [ "" "" "" ] pythonVersionRaw;
      pythonAttr = "python${builtins.replaceStrings [ "." ] [ "" ] pythonVersion}";
    in
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        python = pkgs.${pythonAttr};

        rustDev = (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml).override {
          extensions = [
            "clippy"
            "rustfmt"
          ];
        };

        rustNightly = pkgs.rust-bin.nightly.latest.default;

        cargo-nightly = pkgs.writeShellScriptBin "cargo-nightly" ''
          export RUSTC="${rustNightly}/bin/rustc"
          export RUSTDOC="${rustNightly}/bin/rustdoc"
          exec ${rustNightly}/bin/cargo "$@"
        '';

        coreBuildInputs = [
          # Script tooling
          pkgs.git
          pkgs.just

          # Python tooling
          python
          pkgs.uv
          pkgs.pre-commit

          # Rust tooling
          cargo-nightly
          pkgs.cargo-audit
          pkgs.cargo-hack
          pkgs.cargo-machete
          pkgs.cargo-nextest
          pkgs.cargo-shear
          pkgs.cargo-udeps
        ];

        commonShellHook = ''
          if [ -z "$GOCACHE" ]; then
            if [ -n "$XDG_CACHE_HOME" ]; then
              cache_root="$XDG_CACHE_HOME"
            elif [ -n "$TMPDIR" ]; then
              cache_root="$TMPDIR/judicious-cache-$(id -u)"
            else
              cache_root="/tmp/judicious-cache-$(id -u)"
            fi
            export GOCACHE="$cache_root/go-build"
          fi
          mkdir -p "$GOCACHE"

          if [ -t 0 ]; then
            echo "judicious dev shell"
            echo "Rust: $(rustc --version)"
            echo "Python: $(python3 --version)"
            echo "uv: $(uv --version)"
          fi
        '';

        commonEnv = {
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
            pkgs.stdenv.cc.cc.lib
            pkgs.openssl.out
            pkgs.zlib
          ];

          OPENSSL_DIR = "${pkgs.openssl.dev}";
          OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
        };
      in
      {
        devShells.default = pkgs.mkShell (
          commonEnv
          // {
            buildInputs = coreBuildInputs ++ [ rustDev pkgs.cargo-edit ];
            shellHook = commonShellHook;
          }
        );
      }
    );
}
