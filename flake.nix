{
  description = "A very basic flake";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    fenix.url = "github:nix-community/fenix";
  };

  outputs = { self, nixpkgs, fenix }: let
    pkgs = nixpkgs.legacyPackages.x86_64-linux;
    selfpkgs = nixpkgs.legacyPackages.x86_64-linux;
    fenixPkgs = fenix.packages.x86_64-linux;
    rustToolchain = with fenixPkgs; combine [
      stable.cargo
      stable.rustc
      stable.rust-src
      stable.rust-std
      stable.clippy
      stable.rustfmt
      rust-analyzer
    ];
  in {

    packages.x86_64-linux.hello = pkgs.hello;

    packages.x86_64-linux.default = selfpkgs.hello;

    packages.x86_64-linux.playpen = pkgs.rustPlatform.buildRustPackage {
      name = "playpen";
      src = ./.;
      cargoLock = {
        lockFile = ./Cargo.lock;
      };
    };

    devShells.x86_64-linux.default = pkgs.mkShell {
      RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
      buildInputs = [
        rustToolchain
        # fenixPkgs.rust-analyzer
      ];
    };

  };
}
