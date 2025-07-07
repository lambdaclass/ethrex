let
  pkgs = import <nixpkgs> { };

  solc_0_8_29 = pkgs.stdenv.mkDerivation {
    name = "solc-0.8.29";
    src = pkgs.fetchurl {
      url = "https://github.com/ethereum/solidity/releases/download/v0.8.29/solc-static-linux";
      sha256 = "";
    };
    phases = [ "installPhase" ];
    installPhase = ''
      mkdir -p $out/bin
      cp $src $out/bin/solc
      chmod +x $out/bin/solc
    '';
  };
in
(pkgs.nixos [
  (
    {
      config,
      lib,
      pkgs,
      modulesPath,
      ...
    }:
    let
      inherit (config.image.repart.verityStore) partitionIds;
    in
    {
      imports = [
        "${modulesPath}/image/repart.nix"
        "${modulesPath}/profiles/minimal.nix"
        ./service.nix
      ];

      system.stateVersion = "25.11";
      environment.systemPackages = lib.mkOverride 99 [];
      environment.nativeBuildInputs = with pkgs; [
        git
        solc_0_8_29
      ];
      
      boot.kernelModules = [ "tdx_guest" "tsm" ];
      boot.initrd.availableKernelModules  = [ "dm_mod" "dm_verity" "erofs" "sd_mod" "ahci" ];
      boot.initrd.includeDefaultModules = false;
      nix.enable = false;
      boot = {
        loader.grub.enable = false;
        initrd.systemd.enable = true;
        kernelParams = [ "console=ttyS0" ];
      };
      system.image = {
        id = "ethrex";
        version = "0.1";
      };
      fileSystems = {
        "/" = {
          fsType = "tmpfs";
          options = [ "mode=0755" ];
        };
        "/nix/store" = {
          device = "/usr/nix/store";
          options = [ "bind" "ro" ];
        };
      };
      image.repart = {
        name = "ethrex-image";
        verityStore = {
          enable = true;
          ukiPath = "/EFI/BOOT/BOOTX64.EFI";
        };
        partitions = {
          ${partitionIds.esp} = {
            repartConfig = {
              Type = "esp";
              Format = "vfat";
              SizeMinBytes = "96M";
            };
          };
          ${partitionIds.store-verity}.repartConfig = {
            Minimize = "best";
          };
          ${partitionIds.store}.repartConfig = {
            Minimize = "best";
          };
        };
      };
    }
  )
]).finalImage
