# sysctl kernel.unprivileged_userns_apparmor_policy=0
# sysctl kernel.apparmor_restrict_unprivileged_userns=0

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
        ./service.nix
      ];

      system.stateVersion = "25.11";

      boot = {
        loader.grub.enable = false;
        initrd.systemd.enable = true;
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

        # bind-mount the store
        "/nix/store" = {
          device = "/usr/nix/store";
          options = [ "bind" ];
        };
      };
      image.repart = {
        name = "ethrex-image";
        verityStore = {
          enable = true;
          ukiPath = "/EFI/BOOT/BOOT${pkgs.stdenv.hostPlatform.efiArch}.EFI";
        };
        partitions = {
          ${partitionIds.esp} = {
            # the UKI is injected into this partition by the verityStore module
            repartConfig = {
              Type = "esp";
              Format = "vfat";
              SizeMinBytes = if pkgs.stdenv.hostPlatform.isx86_64 then "64M" else "96M";
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
