{ gitRev }:
assert (builtins.stringLength gitRev == 7)
  || throw "gitRev must be exactly 7 characters use (git rev-parse --short=7 HEAD)";

let
  pkgs = import <nixpkgs> { };
  fenix = pkgs.callPackage (pkgs.fetchFromGitHub {
    owner = "nix-community";
    repo = "fenix";
    rev = "95606d64662a730da5d3031ed798dd6315d35f33";
    hash = "sha256-aP4korVsN5Yy+PB9zjjm8Qbo3a69/m8vlFXS5mdVXtk=";
  }) { };
  toolchain = fenix.fromToolchainFile {
      file = ../../../../rust-toolchain.toml;
      sha256 = "sha256-Qxt8XAuaUR2OMdKbN4u8dBJOhSHxS+uS06Wl9+flVEk=";
  };
  rustPlatform = pkgs.makeRustPlatform {
    cargo = toolchain;
    rustc = toolchain;
  };
  gitignoreSrc = pkgs.fetchFromGitHub {
    owner = "hercules-ci";
    repo = "gitignore.nix";
    rev = "637db329424fd7e46cf4185293b9cc8c88c95394";
    sha256 = "sha256-HG2cCnktfHsKV0s4XW83gU3F57gaTljL9KNSuG6bnQs";
  };
  inherit (import gitignoreSrc { inherit (pkgs) lib; }) gitignoreSource;

in
let
  quoteGen = rustPlatform.buildRustPackage rec {
    pname = "quote-gen";
    version = "0.1";

    src = gitignoreSource ./../../../../.;
    sourceRoot = "${src.name}/crates/l2/tee/quote-gen";

    cargoDeps = rustPlatform.importCargoLock {
      lockFile = ./Cargo.lock;
      outputHashes = {
        "bls12_381-0.8.0" = "sha256-8/pXRA7hVAPeMKCZ+PRPfQfxqstw5Ob4MJNp85pv5WQ=";
        "aligned-sdk-0.1.0" = "sha256-Az97VtggdN4gsYds3myezNJ+mNeSaIDbF0Pq5kq2M3M=";
        "lambdaworks-crypto-0.12.0" = "sha256-4vgW/O85zVLhhFrcZUwcPjavy/rRWB8LGTabAkPNrDw=";
      };
    };

    buildInputs = [ pkgs.openssl ];
    nativeBuildInputs = [
      pkgs.pkg-config
      rustPlatform.cargoSetupHook
    ];

    env = {
      OPENSSL_NO_VENDOR = 1;
      VERGEN_GIT_SHA = gitRev;
    };
  };
in
{
  systemd.services.quote-gen = {
    description = "Ethrex TDX Quote Generator";
    wantedBy = [ "multi-user.target" ];

    serviceConfig = {
      ExecStart = "${quoteGen}/bin/quote-gen";
      StandardOutput = "journal+console";
    };
  };
}
