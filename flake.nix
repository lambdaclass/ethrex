{
  description = "Ethrex Nix";

  inputs = {
    nixpkgs.url     = "github:junjihashimoto/nixpkgs/60fe186313677e8dbf04f917a2b4c3d984be4efd"; #TODO: Change this to upstream nixpkgs when allowBuiltinFetchGit bug is solved (https://discourse.nixos.org/t/cannot-build-package-of-rust-application-ln-failed-to-create-symbolic-link-permission-denied/57587)
    flake-utils.url = "github:numtide/flake-utils";
    crane.url       = "github:ipetkov/crane";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  nixConfig = {
    extra-substituters = ["https://ethrex.cachix.org"];
    extra-trusted-public-keys = ["ethrex.cachix.org-1:ejp9KQpR1FBI2onstMQ34yogDm4OgU2ru6lIwPvuCVs="];
  };

  outputs = { self, nixpkgs, flake-utils, crane, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        toolchain = pkgs.rust-bin.stable."1.89.0".default;

        rustPlat = pkgs.makeRustPlatform {
          cargo = toolchain;
          rustc = toolchain;
        };

        version = "5.0.0";

        src = pkgs.fetchFromGitHub {
          owner  = "lambdaclass";
          repo   = "ethrex";
          rev    = "v${version}";
          sha256 = "sha256-ajPLu5j/sjxLEFN1F+C/f12cbDenzUapBPfStem0Tj4=";
        };

        ethrex = rustPlat.buildRustPackage {
          pname   = "ethrex";
          inherit version src;

          cargoLock = {
            lockFile = "${src}/Cargo.lock";
            allowBuiltinFetchGit = true;
          };

          cargoBuildFlags   = [ ];
          nativeBuildInputs = [ pkgs.pkg-config pkgs.rustPlatform.bindgenHook pkgs.clang pkgs.llvmPackages.libclang ];
          buildInputs       = [ pkgs.openssl ];

          doCheck = false;

          meta = with pkgs.lib; {
            description = "Ethereum execution client";
            homepage    = "https://ethrex.xyz/";
            license     = licenses.asl20;
            platforms   = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
          };
        };
      in
      {
        packages.default = ethrex;

        devShells.default = pkgs.mkShell {
          packages = [
            ethrex
            toolchain
            pkgs.pkg-config
            pkgs.openssl
          ];
          shellHook = ''
            echo "Entered Ethrex shell (${system})"
            export PS1="(ethrex-${system}) \\u@\\h:\\w\\$ "
          '';
        };

        apps.default = {
          type = "app";
          program = "${ethrex}/bin/ethrex";
        };
      }
    );
}
