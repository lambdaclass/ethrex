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
      sha256 = "sha256-KUm16pHj+cRedf8vxs/Hd2YWxpOrWZ7UOrwhILdSJBU=";
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

  solc_0_8_29 = pkgs.stdenv.mkDerivation {
    name = "solc-0.8.29";
    src = pkgs.fetchurl {
      url = "https://github.com/ethereum/solidity/releases/download/v0.8.29/solc-static-linux";
      sha256 = "sha256-GNQYpA3ATRdlaxtcins1z7q4lCtR840AXVtZ6KpmN+A=";
    };
    phases = [ "installPhase" ];
    installPhase = ''
      mkdir -p $out/bin
      cp $src $out/bin/solc
      chmod +x $out/bin/solc
    '';
  };

  openzeppelin = pkgs.fetchgit {
    url = "https://github.com/OpenZeppelin/openzeppelin-contracts-upgradeable.git";
    rev = "v5.3.0";
    sha256 = "sha256-DliCWCetMsVtU7oNBsbk9shiC6MymX1X+qejkNNWpRE";
    fetchSubmodules = true;
  };

  sp1_contracts = pkgs.fetchgit {
    url = "https://github.com/succinctlabs/sp1-contracts.git";
    rev = "v4.0.0";
    sha256 = "sha256-QqgIISFoN1bnc2Bz0T6MXiP6hOg7J5+tEDFmOgRh6qM=";
    fetchSubmodules = true;
  };

  contracts = pkgs.runCommand "contracts" {} ''
    mkdir -p $out/lib/openzeppelin-contracts-upgradeable
    cp -r ${openzeppelin}/* $out/lib/openzeppelin-contracts-upgradeable

    mkdir -p $out/lib/sp1-contracts
    cp -r ${sp1_contracts}/* $out/lib/sp1-contracts
  '';

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
        "spawned-concurrency-0.1.0" = "sha256-63xBuGAlrHvIf8hboScUY4LZronPZJZzmfJBdAbUKTU=";
        "aligned-sdk-0.1.0" = "sha256-Az97VtggdN4gsYds3myezNJ+mNeSaIDbF0Pq5kq2M3M=";
        "lambdaworks-crypto-0.12.0" = "sha256-4vgW/O85zVLhhFrcZUwcPjavy/rRWB8LGTabAkPNrDw=";
      };
    };

    buildInputs = [ pkgs.openssl ];
    nativeBuildInputs = [
      pkgs.pkg-config
      rustPlatform.cargoSetupHook
      solc_0_8_29
    ];

    env = {
      OPENSSL_NO_VENDOR = 1;
      CONTRACTS_PATH = "${contracts}/lib";
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
