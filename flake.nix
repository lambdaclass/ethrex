{
  description = "ethrex prebuilt shells + nix run support";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";

  outputs = { self, nixpkgs }:
  let
    lib = nixpkgs.lib;
    systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
    forEachSystem = lib.genAttrs systems;

    version = "3.0.0";

    urls = {
      "x86_64-linux" = {
        plain = "https://github.com/lambdaclass/ethrex/releases/download/v${version}/ethrex-linux_x86_64";
        gpu   = "https://github.com/lambdaclass/ethrex/releases/download/v${version}/ethrex-linux_x86_64-gpu";
      };
      "aarch64-linux" = {
        plain = "https://github.com/lambdaclass/ethrex/releases/download/v${version}/ethrex-linux_aarch64";
        gpu   = "https://github.com/lambdaclass/ethrex/releases/download/v${version}/ethrex-linux_aarch64-gpu";
      };
      "aarch64-darwin" = {
        plain = "https://github.com/lambdaclass/ethrex/releases/download/v${version}/ethrex-macos_aarch64";
      };
    };

    # Fill these with: nix store prefetch-file --json <url> | jq -r .hash
    hashes = {
      "x86_64-linux" = {
        plain = "sha256-Vg6jwBj5jrSAsb7nn04G/HEKhQyX8sICjmmPMpkIHTI=";
        gpu   = "sha256-DL4VedcJ7PjSRLHap+eHenMj2TfpIZ6rg97fWMsvrLU=";
      };
      "aarch64-linux" = {
        plain = "sha256-8ML5R6EvvYbnfxCRG5mqgNqY/vkl1iEOdoEHKF4BaX8=";
        gpu   = "sha256-c4tvdfrUsLappODY5qXWtwzRMTe+pJcR14NaFvTZpqA=";
      };
      "aarch64-darwin" = {
        plain = "sha256-dMoRtiJOCLV2B4coLrSLkYdEkTKsvbJGzmv049UpBhY=";
      };
    };

    mkEthrex = { pkgs, url, sha256 }:
      pkgs.stdenvNoCC.mkDerivation {
        pname = "ethrex";
        inherit version;
        src = pkgs.fetchurl { inherit url sha256; };
        dontUnpack = true;
        installPhase = ''
          install -Dm755 "$src" "$out/bin/ethrex"
        '';
        meta = with pkgs.lib; {
          description = "Ethereum execution client (prebuilt binary)";
          homepage    = "https://ethrex.xyz/";
          license     = licenses.asl20;
          mainProgram = "ethrex";
          platforms   = platforms.unix;
        };
      };
  in {
    devShells = forEachSystem (system:
      let pkgs = import nixpkgs { inherit system; };
          plain = mkEthrex { inherit pkgs; url = urls.${system}.plain; sha256 = hashes.${system}.plain; };
          haveGpu = urls.${system} ? gpu;
          gpuPkg = if haveGpu then mkEthrex { inherit pkgs; url = urls.${system}.gpu; sha256 = hashes.${system}.gpu; } else null;
          shellFor = pkg: pkgs.mkShell { packages = [ pkg ]; shellHook = ''echo "ethrex: $(command -v ethrex)"; ethrex --version || true''; };
      in if system == "x86_64-linux" then {
           default = shellFor plain; linux-x86_64 = shellFor plain; linux-x86_64-gpu = shellFor gpuPkg;
         } else if system == "aarch64-linux" then {
           default = shellFor plain; linux-aarch64 = shellFor plain; linux-aarch64-gpu = shellFor gpuPkg;
         } else {
           default = shellFor plain; macos-aarch64 = shellFor plain;
         }
    );

    packages = forEachSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        plain = mkEthrex { inherit pkgs; url = urls.${system}.plain; sha256 = hashes.${system}.plain; };
        haveGpu = urls.${system} ? gpu;
      in
        { ethrex = plain;
          default = plain;
        } // lib.optionalAttrs haveGpu {
          ethrex-gpu = mkEthrex { inherit pkgs; url = urls.${system}.gpu; sha256 = hashes.${system}.gpu; };
        }
    );

    apps = forEachSystem (system: {
      ethrex = {
        type = "app";
        program = "${self.packages.${system}.ethrex}/bin/ethrex";
      };
      default = self.apps.${system}.ethrex;
    });
  };
}

