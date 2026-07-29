{
  description = "Convert Markdown to Atlassian Document Format (ADF), with JSON Schema validation";

  # nixpkgs only. No flake-utils, no rust overlay: the crate tracks stable Rust
  # and nixpkgs carries a new enough toolchain for edition 2024, so extra
  # inputs would add supply chain and lockfile churn for nothing.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = {nixpkgs, ...}: let
    systems = ["x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"];
    forAllSystems = f:
      nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

    # Single source of truth for the version: the manifest.
    cargoToml = fromTOML (builtins.readFile ./Cargo.toml);
  in {
    packages = forAllSystems (pkgs: rec {
      adfc = pkgs.rustPlatform.buildRustPackage {
        pname = cargoToml.package.name;
        inherit (cargoToml.package) version;
        src = ./.;

        # Vendored from the committed lockfile, so a build resolves exactly the
        # dependency set that was tested rather than whatever is current.
        cargoLock.lockFile = ./Cargo.lock;

        # buildRustPackage runs `cargo test` by default, which is what proves
        # the emitted ADF still validates against the vendored schema. Left on
        # deliberately: a build that produces a binary emitting invalid ADF is
        # not a build worth shipping.

        meta = {
          inherit (cargoToml.package) description;
          homepage = cargoToml.package.repository;
          license = pkgs.lib.licenses.mit;
          mainProgram = cargoToml.package.name;
        };
      };
      default = adfc;
    });

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        # rustfmt and clippy are listed explicitly: neither ships with a bare
        # cargo, and the pre-commit hooks fail on their absence first.
        packages = with pkgs; [
          cargo
          rustc
          rustfmt
          clippy
          rust-analyzer
          cargo-audit # `just audit`, and the CI advisory check
          cargo-dist # builds the release archives the npm packages wrap
          just # task entry points; see the justfile
          nodejs # the npm bin shim and the packaging script
          # The packaging script shells out to unzip for the Windows archives
          # and tar for the rest; zip is what its tests build fixtures with.
          # Listed explicitly rather than relied on from stdenv.
          unzip
          zip
          jq # the test harness and the CLI's own usage shell out to it
          prek # runs .pre-commit-config.yaml
        ];

        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      };
    });

    # `nix fmt` formats this flake; matches the formatter used across these repos.
    formatter = forAllSystems (pkgs: pkgs.alejandra);
  };
}
