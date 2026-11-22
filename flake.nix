# SPDX-License-Identifier: AGPL-3.0-or-later
{
  description = "unemupc — IBM PC/AT (80286) LLE emulator devShell (Rust + Backlog.md + tooling)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    # Local-first task tracker. Provides the `backlog` CLI.
    # Do NOT add `.inputs.nixpkgs.follows` here — the bun2nix build wants its pinned nixpkgs.
    backlog-md.url = "github:MrLesk/Backlog.md";
    # The Rust toolchain: the latest stable release as upstream binaries, so
    # moving to a new Rust never rebuilds the rest of nixpkgs.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      backlog-md,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        # Keep [workspace.package] rust-version in Cargo.toml equal to this.
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
            "clippy"
            "rustfmt"
          ];
        };
      in
      {
        devShells.firmware = pkgs.mkShell {
          # SeaBIOS owns its freestanding flags and has no host hardening runtime.
          # Legacy host Kconfig also needs translated printf formats without fatal warnings.
          hardeningDisable = [ "all" ];
          packages = [
            pkgs.pkgsCross.i686-embedded.buildPackages.gcc
            pkgs.pkgsCross.i686-embedded.buildPackages.binutils
            pkgs.python3
            pkgs.gnumake
            pkgs.git
            pkgs.coreutils
            pkgs.gnutar
            pkgs.gnugrep
            pkgs.gawk
          ];
          CROSS_PREFIX = pkgs.pkgsCross.i686-embedded.stdenv.cc.targetPrefix;
          # Kconfig runs on the host; firmware compilation uses the cross GCC.
          HOSTCC = "${pkgs.stdenv.cc}/bin/cc";
        };
        devShells.default = pkgs.mkShell {
          packages = [
            backlog-md.packages.${system}.default # `backlog` — local-first task tracker
            pkgs.pre-commit
            pkgs.git
            pkgs.python3 # source-include guard and fixture validation

            # --- Rust toolchain (CPU core = x86jit, all crates are Rust) ---
            rustToolchain
            pkgs.cargo-deny
            # Runs every test in its own process and test binaries side by
            # side; scripts/affected-tests.sh uses it when present.
            pkgs.cargo-nextest

            # --- guest test-ROM build (hand-assembled real-mode / long-mode programs) ---
            pkgs.nasm
            pkgs.binutils # ld, objcopy for flat binaries

            # --- FreeDOS boot-floppy build (vendor/freedos/fetch.sh) ---
            pkgs.mtools # mformat/mcopy/mdir — lay down the FAT12 filesystem
            pkgs.dosfstools # mkfs.fat — FAT12 formatting

            # --- fixture downloads and archive extraction (vendor/*/fetch.sh) ---
            # Pinned instead of taken from the host, so every fixture script
            # runs the same tools on macOS and Linux. Only curl's `bin` output:
            # mkShell would otherwise add its `dev` output, whose headers and
            # pkg-config files reach the C compiles inside Rust builds.
            pkgs.curl.bin
            pkgs.unzip # Info-ZIP UnZip 6.00

            # --- optional differential oracles (final-state / serial compare) ---
            pkgs.qemu

            # Fetching the SingleStepTests / ProcessorTests corpus (real-mode oracle)
            pkgs.jq
          ];
          # Downloads trust the pinned Mozilla roots, not the host's bundle: an
          # old container's bundle lacks roots that current mirrors chain to.
          # Nixpkgs' OpenSSL (curl, git) reads NIX_SSL_CERT_FILE before
          # SSL_CERT_FILE; the curl program itself prefers CURL_CA_BUNDLE to a
          # host's SSL_CERT_FILE. cacert stays out of `packages`, whose setup
          # hook would export SSL_CERT_FILE: the registry client's native-roots
          # loader would then read that file instead of the macOS keychain.
          NIX_SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
          CURL_CA_BUNDLE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
        };
      }
    );
}
