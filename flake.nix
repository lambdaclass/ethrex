{
  description = "Minimal ethrex flake that pre-builds the release binary (named shells for mac/linux)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      lib = nixpkgs.lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forEachSystem = lib.genAttrs systems;
    in {
      devShells = forEachSystem (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };

          rustToolchain = pkgs.rust-bin.stable."1.90.0".default;

          targetTriple =
            if system == "x86_64-linux" then "x86_64-unknown-linux-gnu"
            else if system == "aarch64-linux" then "aarch64-unknown-linux-gnu"
            else if system == "aarch64-darwin" then "aarch64-apple-darwin"
            else if system == "x86_64-darwin" then "x86_64-apple-darwin"
            else system;
          targetVar = lib.replaceStrings ["-"] ["_"] targetTriple;

          solc = pkgs.stdenv.mkDerivation {
            pname = "solc";
            version = "0.8.29";
            src = pkgs.fetchurl {
              url = {
                "x86_64-linux"   = "https://github.com/ethereum/solidity/releases/download/v0.8.29/solc-static-linux";
                "aarch64-linux"  = "https://github.com/nikitastupin/solc/raw/refs/heads/main/linux/aarch64/solc-v0.8.29";
                "x86_64-darwin"  = "https://github.com/ethereum/solidity/releases/download/v0.8.29/solc-macos";
                "aarch64-darwin" = "https://github.com/ethereum/solidity/releases/download/v0.8.29/solc-macos";
              }.${system} or (throw "Unsupported system for solc 0.8.29");
              sha256 = {
                "x86_64-linux"   = "1q1pcsmfhnavbl08vwsi5fabifng6mxqlp0vddjifkf01nj1im0q";
                "aarch64-linux"  = "1wvdygj5p5743qg5ji6dmixwn08sbs74vdfr6bfc3x830wz48k5l";
                "x86_64-darwin"  = "1lb4x0yqrwjg9v0ks89d5ixinbnrnhbpvv4p34qr204cgk8vvyk6";
                "aarch64-darwin" = "1lb4x0yqrwjg9v0ks89d5ixinbnrnhbpvv4p34qr204cgk8vvyk6";
              }.${system} or (throw "Unsupported system for solc 0.8.29");
            };
            dontUnpack = true;
            installPhase = ''
              install -Dm755 $src $out/bin/solc
            '';
            meta = with lib; {
              description = "Solidity compiler";
              homepage = "https://docs.soliditylang.org";
              platforms = [ system ];
              license = licenses.gpl3Only;
            };
          };

          featuresMap = {
            "x86_64-linux"   = "sp1,risc0";
            "aarch64-linux"  = "sp1";
            "x86_64-darwin"  = "";
            "aarch64-darwin" = "";
          };

          rustFlagsMap = {
            "x86_64-linux" = "-C target-cpu=x86-64-v2";
          };
          rustFlags = rustFlagsMap.${system} or "";

          mkShell = { buildGpu ? false, shellFlavor }:
            let
              baseNativeDeps = [
                pkgs.pkg-config
                pkgs.openssl
                pkgs.protobuf
                pkgs.llvmPackages.libclang   # libclang .so/.dylib
                pkgs.clang                   # clang binary + resource-dir
                pkgs.cmake
                pkgs.zlib
                pkgs.curl
                pkgs.stdenv.cc               # wrapped toolchain
                solc
                pkgs.rustup
              ] ++ lib.optionals pkgs.stdenv.isLinux [
                pkgs.glibc.dev               # glibc headers (Linux only)
              ];

              # macOS frameworks + libc++
              darwinDeps =
                if pkgs.stdenv.isDarwin
                then (with pkgs.darwin.apple_sdk.frameworks; [ Security SystemConfiguration ])
                     ++ [ pkgs.libcxx.dev ]
                else [];

              cudaDeps = lib.optionals (buildGpu && system == "x86_64-linux") [
                pkgs.cudaPackages.cudatoolkit
                pkgs.cudaPackages.cuda_nvcc
              ];

              nativeDeps = baseNativeDeps ++ darwinDeps ++ cudaDeps;

              baseFeatures = featuresMap.${system} or "";
              baseFeaturesList = lib.filter (s: s != "") (lib.splitString "," baseFeatures);
              featuresList = baseFeaturesList ++ lib.optional buildGpu "gpu";
              featuresStr = lib.concatStringsSep "," featuresList;
              featureArg = lib.optionalString (featuresStr != "")
                " --features ${lib.escapeShellArg featuresStr}";
              featuresNote = if featuresStr != "" then featuresStr else "<none>";

              #FIXME: aarch64-linux installs risc0 even though featuresList does not make aarch64-linux compile with risc0
              needsRisc0 = lib.elem "risc0" featuresList;
              needsSp1 = lib.elem "sp1" featuresList;

              cIncArg =
                if pkgs.stdenv.isLinux then "-isystem ${pkgs.stdenv.cc.libc.dev}/include" else "";
              cxxIncArg =
                if pkgs.stdenv.isDarwin then "-isystem ${pkgs.libcxx.dev}/include/c++/v1" else "";
            in
            pkgs.mkShell {
              packages = nativeDeps ++ [ rustToolchain ];

              shellHook = lib.concatStringsSep "" [
                ''
                  export SHELL_FLAVOR=${shellFlavor}
                  export PATH=$PWD/target/release:$PATH
                ''
                (lib.optionalString (rustFlags != "") ''
                  export RUSTFLAGS=${lib.escapeShellArg rustFlags}
                '')
                (lib.optionalString buildGpu ''
                  export NVCC_PREPEND_FLAGS=-arch=sm_70
                '')
                ''
                  export CC="${pkgs.stdenv.cc}/bin/cc"
                  export CXX="${pkgs.stdenv.cc}/bin/c++"

                  unset CPATH CPLUS_INCLUDE_PATH CFLAGS CXXFLAGS CPPFLAGS
                  unset NIX_CFLAGS_COMPILE NIX_CXXFLAGS_COMPILE NIX_CFLAGS_COMPILE_FOR_BUILD

                  export OPENSSL_NO_VENDOR=1
                  unset OPENSSL_DIR OPENSSL_LIB_DIR OPENSSL_INCLUDE_DIR

                  SYSROOT=""
                  if [ "$(uname -s)" = "Darwin" ]; then
                    SYSROOT="$(xcrun --show-sdk-path 2>/dev/null || true)"
                  else
                    SYSROOT="$(${pkgs.stdenv.cc}/bin/cc -print-sysroot || true)"
                  fi

                  GCC_INC_BASE=""
                  if [ "$(uname -s)" != "Darwin" ]; then
                    GCC_LIBGCC="$(${pkgs.stdenv.cc}/bin/cc -print-libgcc-file-name || true)"
                    if [ -n "$GCC_LIBGCC" ]; then
                      GCC_INC_BASE="$(dirname "$GCC_LIBGCC")"
                    fi
                  fi

                  BINDGEN_FLAGS="--target=${targetTriple}"
                  if [ -n "$SYSROOT" ]; then
                    BINDGEN_FLAGS="$BINDGEN_FLAGS --sysroot=$SYSROOT"
                  fi
                  ${lib.optionalString (cIncArg != "") ''
                    BINDGEN_FLAGS="$BINDGEN_FLAGS ${cIncArg}"
                  ''}
                  ${lib.optionalString (cxxIncArg != "") ''
                    BINDGEN_FLAGS="$BINDGEN_FLAGS ${cxxIncArg}"
                  ''}
                  ${lib.optionalString pkgs.stdenv.isLinux ''
                    if [ -n "$GCC_INC_BASE" ]; then
                      BINDGEN_FLAGS="$BINDGEN_FLAGS -isystem $GCC_INC_BASE/include -isystem $GCC_INC_BASE/include-fixed"
                    fi
                  ''}
                  if command -v clang >/dev/null 2>&1; then
                    BINDGEN_FLAGS="$BINDGEN_FLAGS -resource-dir $(clang -print-resource-dir)"
                  fi
                  export BINDGEN_EXTRA_CLANG_ARGS="$BINDGEN_FLAGS"

                  export CLANG_PATH="${pkgs.clang}/bin/clang"
                  export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
                  export LD_LIBRARY_PATH="${pkgs.llvmPackages.libclang.lib}/lib"''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
                  export DYLD_LIBRARY_PATH="${pkgs.llvmPackages.libclang.lib}/lib"''${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}

                  export CC_${targetVar}="$CC"
                  export CXX_${targetVar}="$CXX"

                  if [ "$(uname -s)" = "Darwin" ]; then
                    SYSROOT_TRIMMED="$SYSROOT"; SYSROOT_TRIMMED="''${SYSROOT_TRIMMED%/}"
                    FRAMEWORKS_DIR="''${SYSROOT_TRIMMED}/System/Library/Frameworks"
                    [ -d "$FRAMEWORKS_DIR" ] || FRAMEWORKS_DIR="/System/Library/Frameworks"

                    export LDFLAGS="-F$FRAMEWORKS_DIR ${LDFLAGS:-}"
                    export CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS="''${CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS:+$CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS }-C link-arg=-F$FRAMEWORKS_DIR"
                    export MACOSX_DEPLOYMENT_TARGET=11.0
                    echo "macOS frameworks: $FRAMEWORKS_DIR"
                  fi
                ''
                (lib.optionalString needsSp1 ''
                  SP1UP="$HOME/.sp1/bin/sp1up"
                  if [ ! -x "$SP1UP" ]; then
                    echo "Installing SP1 toolchain..."
                    curl -s https://sp1up.succinct.xyz | bash
                  fi
                  if [ -x "$SP1UP" ]; then
                    export PATH=$HOME/.sp1/bin:$PATH
                    "$SP1UP" --version 5.0.8
                  else
                    echo "Warning: failed to install sp1up" >&2
                  fi
                '')
                (lib.optionalString needsRisc0 ''
                  RZUP="$HOME/.risc0/bin/rzup"
                  if [ ! -x "$RZUP" ]; then
                    echo "Installing RISC Zero toolchain..."
                    curl -sSfL https://risczero.com/install | bash
                  fi
                  if [ -x "$RZUP" ]; then
                    export PATH=$HOME/.risc0/bin:$PATH
                    if [ ! -x "$HOME/.risc0/bin/cargo-risczero" ]; then
                      "$RZUP" install cargo-risczero 3.0.3
                    fi
                    if [ ! -x "$HOME/.risc0/bin/risc0-groth16" ]; then
                      "$RZUP" install risc0-groth16
                    fi
                    if [ ! -x "$HOME/.risc0/bin/cargo" ]; then
                      "$RZUP" install rust
                    fi
                  else
                    echo "Warning: failed to find rzup after installation; skipping RISC Zero setup" >&2
                  fi
                  export PATH=$HOME/.risc0/bin:$PATH
                '')
                ''
                  echo "Building ethrex (release, COMPILE_CONTRACTS=true, features: ${featuresNote})"
                  COMPILE_CONTRACTS=true cargo build --bin ethrex --release${featureArg}
                ''
              ];
            };

          linuxX86     = mkShell { shellFlavor = "linux-x86_64"; };
          linuxX86Gpu  = mkShell { buildGpu = true; shellFlavor = "linux-x86_64-gpu"; };
          linuxArm     = mkShell { shellFlavor = "linux-aarch64"; };
          macShellArm  = mkShell { shellFlavor = "aarch64-darwin"; };
          macShellX86  = mkShell { shellFlavor = "x86_64-darwin"; };
        in
        if system == "x86_64-linux" then {
          default        = linuxX86;
          linux-x86_64   = linuxX86;
          linux-x86_64-gpu = linuxX86Gpu;
        } else if system == "aarch64-linux" then {
          default        = linuxArm;
          linux-aarch64  = linuxArm;
        } else if system == "aarch64-darwin" then {
          default        = macShellArm;
          aarch64-darwin = macShellArm;
        } else if system == "x86_64-darwin" then {
          default        = macShellX86;
          x86_64-darwin  = macShellX86;
        } else {
          default = pkgs.mkShell { };
        }
      );

      packages = forEachSystem (system:
        let
          pkgs = import nixpkgs { inherit system; overlays = [ rust-overlay.overlays.default ]; };
          rustToolchain = pkgs.rust-bin.stable."1.90.0".default;
        in {
          ethrex-run = pkgs.writeShellApplication {
            name = "ethrex-run";
            runtimeInputs = [
              rustToolchain
              pkgs.cargo
              pkgs.pkg-config
              pkgs.openssl
              pkgs.protobuf
              pkgs.cmake
              pkgs.clang
              pkgs.llvmPackages.libclang
              pkgs.zlib
              pkgs.curl
              pkgs.stdenv.cc
            ] ++ lib.optionals pkgs.stdenv.isLinux [ pkgs.glibc.dev ];

            text = ''
              set -euo pipefail
              SRC=${self}
              WORK="$(mktemp -d)"
              trap 'rm -rf "$WORK"' EXIT

              # build outside the store so cargo can write artifacts
              cp -R "$SRC"/* "$WORK"
              chmod -R u+w "$WORK"
              cd "$WORK"

              export OPENSSL_NO_VENDOR=1
              export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
              export DYLD_LIBRARY_PATH="${pkgs.llvmPackages.libclang.lib}/lib"''${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}
              export LD_LIBRARY_PATH="${pkgs.llvmPackages.libclang.lib}/lib"''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}

              cargo run --release --bin ethrex -- "$@"
            '';
          };

          default = self.packages.${system}.ethrex-run;
        });

      apps = forEachSystem (system: {
        ethrex = {
          type = "app";
          program = "${self.packages.${system}.ethrex-run}/bin/ethrex-run";
        };
        default = self.apps.${system}.ethrex;
      });
    };
}

