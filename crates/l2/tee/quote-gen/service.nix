let
  pkgs = import <nixpkgs> { };
in
let
  quoteGen = pkgs.rustPlatform.buildRustPackage rec {
    pname = "quote-gen";
    version = "0.1";

    src = pkgs.lib.cleanSource ./../../../../.;
    sourceRoot = "${src.name}/crates/l2/tee/quote-gen";

    cargoDeps = pkgs.rustPlatform.importCargoLock {
      lockFile = ./Cargo.lock;
      outputHashes = {
        "bls12_381-0.8.0" = "sha256-8/pXRA7hVAPeMKCZ+PRPfQfxqstw5Ob4MJNp85pv5WQ=";
      };
    };

    buildInputs = [ pkgs.openssl ];
    nativeBuildInputs = [
      pkgs.pkg-config
      pkgs.rustPlatform.cargoSetupHook
    ];
    env.OPENSSL_NO_VENDOR = 1;
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
