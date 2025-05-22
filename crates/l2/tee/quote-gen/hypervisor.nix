{ pkgs ? import <nixpkgs> {} }:
let
  OVMF = pkgs.OVMF.override {
    projectDscPath = "OvmfPkg/IntelTdx/IntelTdxX64.dsc";
    metaPlatforms = builtins.filter (pkgs.lib.hasPrefix "x86_64-") pkgs.OVMF.meta.platforms;
  };
  qemu = (pkgs.qemu.overrideAttrs (oldAttrs: rec {
    srcs = [
      (pkgs.fetchFromGitHub {
        name = "qemu";
        owner = "intel";
        repo = "qemu-tdx";
        rev = "tdx-upstream-snapshot-2025-05-20";
        fetchSubmodules = true;
        hash = "sha256-qm0KasKH1afx0vyAeuZOsPNJvS5E3znTI/XP/pyQ64o=";
      })
      (pkgs.fetchFromGitLab {
        name = "keycodemapdb";
        owner = "qemu-project";
        repo = "keycodemapdb";
        rev = "f5772a62ec52591ff6870b7e8ef32482371f22c6";
        fetchSubmodules = true;
        hash = "sha256-EQrnBAXQhllbVCHpOsgREzYGncMUPEIoWFGnjo+hrH4=";
      })
      (pkgs.fetchFromGitLab {
        name = "berkeley-softfloat-3";
        owner = "qemu-project";
        repo = "berkeley-softfloat-3";
        rev = "b64af41c3276f97f0e181920400ee056b9c88037";
        fetchSubmodules = true;
        hash = "sha256-Yflpx+mjU8mD5biClNpdmon24EHg4aWBZszbOur5VEA=";
      })
      (pkgs.fetchFromGitLab {
        name = "berkeley-testfloat-3";
        owner = "qemu-project";
        repo = "berkeley-testfloat-3";
        rev = "e7af9751d9f9fd3b47911f51a5cfd08af256a9ab";
        fetchSubmodules = true;
        hash = "sha256-inQAeYlmuiRtZm37xK9ypBltCJ+ycyvIeIYZK8a+RYU=";
      })
    ];
    patches = [];
    nativeBuildInputs = oldAttrs.nativeBuildInputs ++ [ pkgs.python3Packages.distutils ];
    postUnpack = ''
      mv keycodemapdb qemu/subprojects
      mv berkeley-softfloat-3 qemu/subprojects
      cp qemu/subprojects/packagefiles/berkeley-softfloat-3/* qemu/subprojects/berkeley-softfloat-3
      mv berkeley-testfloat-3 qemu/subprojects
      cp qemu/subprojects/packagefiles/berkeley-testfloat-3/* qemu/subprojects/berkeley-testfloat-3
      cd qemu
    '';
    sourceRoot = ".";
  })).override {
    minimal = true;
    hostCpuTargets = [ "x86_64-softmmu" ];
  };
in
let
  script = pkgs.writeShellScriptBin "run-qemu" ''
   ${qemu}/bin/qemu-system-x86_64 -machine q35,kernel_irqchip=split,confidential-guest-support=tdx,hpet=off -smp 2 -m 2G \
        -accel kvm -cpu host -nographic -nodefaults \
        -bios ${OVMF.mergedFirmware} \
        -nic user,model=virtio-net-pci \
        -chardev stdio,mux=on,id=console,signal=off -device virtconsole,chardev=console -mon console \
        -drive file=$1,if=none,id=virtio-disk0 -device virtio-blk-pci,drive=virtio-disk0 \
        -object '{"qom-type":"tdx-guest","id":"tdx","quote-generation-socket":{"type": "vsock", "cid":"2","port":"4050"}}'
  '';
in 
pkgs.symlinkJoin {
  name = "run-qemu";
  paths = [ script ];
  buildInputs = [ pkgs.makeWrapper ];
  postBuild = "wrapProgram $out/bin/run-qemu --prefix PATH : $out/bin";
}
