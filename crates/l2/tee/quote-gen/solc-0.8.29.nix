{ pkgs }:
pkgs.stdenv.mkDerivation rec {
  pname = "solc";
  version = "0.8.29";

  src = pkgs.fetchurl {
    url = "https://github.com/ethereum/solidity/releases/download/v${version}/solc-static-linux";
    sha256 = "sha256-GNQYpA3ATRdlaxtcins1z7q4lCtR840AXVtZ6KpmN+A=";
  };

  dontUnpack = true;

  installPhase = ''
    install -Dm755 "$src" "$out/bin/solc"
  '';
}
