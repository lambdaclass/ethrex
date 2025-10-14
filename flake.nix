{
  description = "Ethrex Nix";

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
          mainProgram = "ethrex";
          description = "Ethereum execution client (prebuilt binary)";
          homepage = "https://ethrex.xyz/";
          license = licenses.asl20;
        };
      };

  in {
    devShells = forEachSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        haveGpu = urls.${system} ? gpu;

        ethrex-plain = mkEthrex {
          inherit pkgs;
          url = urls.${system}.plain;
          sha256 = hashes.${system}.plain;
        };

        ethrex-gpu = lib.optional haveGpu (mkEthrex {
          inherit pkgs;
          url = urls.${system}.gpu;
          sha256 = hashes.${system}.gpu;
        });

        shellFor = pkg: pkgs.mkShell {
          packages = [ pkg ];
          shellHook = ''
            echo "ethrex available at: $(command -v ethrex)"
            ethrex --version || true
          '';
        };
      in
      if system == "x86_64-linux" then {
        default           = shellFor ethrex-plain;
        linux-x86_64      = shellFor ethrex-plain;
        linux-x86_64-gpu  = shellFor (builtins.head ethrex-gpu);
      } else if system == "aarch64-linux" then {
        default           = shellFor ethrex-plain;
        linux-aarch64     = shellFor ethrex-plain;
        linux-aarch64-gpu = shellFor (builtins.head ethrex-gpu);
      } else if system == "aarch64-darwin" then {
        default           = shellFor ethrex-plain;
        macos-aarch64     = shellFor ethrex-plain;
      } else {
        default = pkgs.mkShell { };
      }
    );
  };
}

