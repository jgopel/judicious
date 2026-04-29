{
  description = "Minimal build environment for reasonable";

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
            "rust-src"
            "rust-analyzer"
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
          pkgs.cargo-hack
          pkgs.cargo-machete
          pkgs.cargo-nextest
          pkgs.cargo-udeps
        ];

        commonShellHook = ''
          if [ -t 0 ]; then
            echo "reasonable dev shell"
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
