{ pkgs ? import <nixpkgs> {} }:
let
  OVMF = pkgs.OVMF.override {
    projectDscPath = "OvmfPkg/IntelTdx/IntelTdxX64.dsc";
    metaPlatforms = builtins.filter (pkgs.lib.hasPrefix "x86_64-") pkgs.OVMF.meta.platforms;
  };
  qemu = (pkgs.qemu.overrideAttrs (oldAttrs: rec {
    srcs = [
      (pkgs.fetchgit {
        name = "qemu";
        url = "https://git.launchpad.net/~kobuk-team/ubuntu/+source/qemu";
        rev = "cee4831c72ad8e57e6a10626fbb74fd3236e88ff";
        hash = "sha256-T+2IvPJHTxcaX4KZpttfVdFD3HyMLcRs92SLevPvnaE=";
      })
      (pkgs.fetchFromGitHub {
        name = "qemu-intel";
        owner = "intel";
        repo = "qemu-tdx";
        rev = "tdx-upstream-snapshot-2025-05-20";
        hash = "sha256-CrfcR/pfPbpj1I8jAZDE0y+lGN59C+Wio6lCVtn/d1g=";
      })
    ];
    patches = [ ];
    nativeBuildInputs = with pkgs; oldAttrs.nativeBuildInputs ++ [ 
    python3Packages.distutils
        dtc
        pixman
        vde2
        lzo
        snappy
        libtasn1
        libslirp
        libcbor
    ];
    postUnpack = ''
      cp -R qemu-intel/pc-bios/* qemu/pc-bios
      cd qemu
    '';
    sourceRoot = ".";
    postPatch = ''
      for fname in $(cat debian/patches/series | sed '/^#/d')
        do
          patch -p1 < "debian/patches/$fname"
        done
    '';
  })).override {
    minimal = true;
    spiceSupport = true;
    enableBlobs = true;
    hostCpuTargets = [ "x86_64-softmmu" ];
  };
in
let
  script = pkgs.writeShellScriptBin "run-qemu" ''
   ${qemu}/bin/qemu-system-x86_64 -machine q35,kernel_irqchip=split,confidential-guest-support=tdx,hpet=off -smp 2 -m 2G \
        -accel kvm -cpu host -nographic -nodefaults \
        -bios ${OVMF.mergedFirmware} \
        -no-user-config \
        -serial mon:stdio \
        -netdev user,id=net0,net=192.168.76.0/24 -device e1000,netdev=net0 \
        -device ide-hd,bus=ide.0,drive=main,bootindex=0 -drive "if=none,media=disk,id=main,file.filename=$1,discard=unmap,detect-zeroes=unmap" \
        -object '{"qom-type":"tdx-guest","id":"tdx","quote-generation-socket":{"type": "vsock", "cid":"2","port":"4050"}}'
  '';
in 
pkgs.symlinkJoin {
  name = "run-qemu";
  paths = [ script ];
  buildInputs = [ pkgs.makeWrapper ];
  postBuild = "wrapProgram $out/bin/run-qemu --prefix PATH : $out/bin";
}
