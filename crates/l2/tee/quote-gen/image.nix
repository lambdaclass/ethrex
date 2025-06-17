let
  pkgs = import <nixpkgs> { };
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
