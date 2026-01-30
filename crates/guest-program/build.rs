fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    #[cfg(all(not(clippy), feature = "risc0"))]
    {
        // Track RISC0 guest program source changes
        println!("cargo::rerun-if-changed=bin/risc0/src");
        println!("cargo::rerun-if-changed=bin/risc0/Cargo.toml");
        build_risc0_program();
    }

    #[cfg(all(not(clippy), feature = "sp1"))]
    {
        // Track SP1 guest program source changes
        println!("cargo::rerun-if-changed=bin/sp1/src");
        println!("cargo::rerun-if-changed=bin/sp1/Cargo.toml");
        build_sp1_program();
    }

    #[cfg(all(not(clippy), feature = "zisk"))]
    {
        // Track ZisK guest program source changes
        println!("cargo::rerun-if-changed=bin/zisk/src");
        println!("cargo::rerun-if-changed=bin/zisk/Cargo.toml");
        build_zisk_program();
    }

    #[cfg(all(not(clippy), feature = "openvm"))]
    {
        // Track OpenVM guest program source changes
        println!("cargo::rerun-if-changed=bin/openvm/src");
        println!("cargo::rerun-if-changed=bin/openvm/Cargo.toml");
        build_openvm_program();
    }
}

#[cfg(all(not(clippy), feature = "risc0"))]
fn build_risc0_program() {
    use hex;
    use risc0_build::{DockerOptionsBuilder, GuestOptionsBuilder, embed_methods_with_options};

    let features = if cfg!(feature = "l2") {
        vec!["l2".to_string()]
    } else {
        vec![]
    };

    let guest_options = if option_env!("PROVER_REPRODUCIBLE_BUILD").is_some() {
        let docker_options = DockerOptionsBuilder::default()
            .root_dir(format!("{}/../../../", env!("CARGO_MANIFEST_DIR")))
            .build()
            .unwrap();
        GuestOptionsBuilder::default()
            .features(features)
            .use_docker(docker_options)
            .build()
            .unwrap()
    } else {
        GuestOptionsBuilder::default()
            .features(features)
            .build()
            .unwrap()
    };

    let built_guests = embed_methods_with_options(std::collections::HashMap::from([(
        "ethrex-guest-risc0",
        guest_options,
    )]));
    let elf = built_guests[0].elf.clone();
    let image_id = built_guests[0].image_id;

    // this errs if the dir already exists, so we don't handle an error.
    let _ = std::fs::create_dir("./bin/risc0/out");

    std::fs::write("./bin/risc0/out/riscv32im-risc0-elf", &elf)
        .expect("could not write Risc0 elf to file");

    std::fs::write(
        "./bin/risc0/out/riscv32im-risc0-vk",
        format!("0x{}\n", hex::encode(image_id.as_bytes())),
    )
    .expect("could not write Risc0 vk to file");
}

#[cfg(all(not(clippy), feature = "sp1"))]
fn build_sp1_program() {
    use hex;
    use sp1_sdk::{HashableKey, ProverClient};

    let features = if cfg!(feature = "l2") {
        vec!["l2".to_string()]
    } else {
        vec![]
    };

    sp1_build::build_program_with_args(
        "./bin/sp1",
        sp1_build::BuildArgs {
            output_directory: Some("./bin/sp1/out".to_string()),
            elf_name: Some("riscv32im-succinct-zkvm-elf".to_string()),
            features,
            docker: option_env!("PROVER_REPRODUCIBLE_BUILD").is_some(),
            tag: "v5.0.8".to_string(),
            workspace_directory: Some(format!("{}/../../../", env!("CARGO_MANIFEST_DIR"))),
            ..Default::default()
        },
    );

    // Get verification key
    // ref: https://github.com/succinctlabs/sp1/blob/dev/crates/cli/src/commands/vkey.rs
    let elf = std::fs::read("./bin/sp1/out/riscv32im-succinct-zkvm-elf")
        .expect("could not read SP1 elf file");
    let prover = ProverClient::from_env();
    let (_, vk) = prover.setup(&elf);

    std::fs::write(
        "./bin/sp1/out/riscv32im-succinct-zkvm-vk-bn254",
        format!("{}\n", vk.vk.bytes32()),
    )
    .expect("could not write SP1 vk-bn254 to file");
    std::fs::write(
        "./bin/sp1/out/riscv32im-succinct-zkvm-vk-u32",
        format!("0x{}\n", hex::encode(vk.vk.hash_bytes())),
    )
    .expect("could not write SP1 vk-u32 to file");
}

#[cfg(all(not(clippy), feature = "zisk"))]
fn build_zisk_program() {
    // cargo-zisk rom-setup fails with `Os { code: 2, kind: NotFound, message: "No such file or directory" }`
    // when building in a GitHub CI environment. This command is not required if we won't generate a proof
    // so we skip it under the `ci` feature flag.

    #[cfg(not(feature = "ci"))]
    fn zisk_command() -> std::process::Command {
        std::process::Command::new("cargo-zisk")
    }

    #[cfg(feature = "ci")]
    fn zisk_command() -> std::process::Command {
        let mut cmd = std::process::Command::new("cargo");
        cmd.args(["+zisk"]);
        cmd
    }

    let mut build_command = std::process::Command::new("cargo");
    #[cfg(not(feature = "ci"))]
    let mut setup_command = zisk_command();
    #[cfg(not(feature = "ci"))]
    let mut vkey_command = zisk_command();

    let features = if cfg!(feature = "l2") {
        vec!["l2".to_string()]
    } else {
        vec![]
    };

    build_command
        .env("RUSTC", rustc_path("zisk"))
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .args([
            "+zisk",
            "build",
            "--release",
            "--target",
            "riscv64ima-zisk-zkvm-elf",
        ])
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .current_dir("./bin/zisk");
    if !features.is_empty() {
        build_command.arg("--features").arg(features.join(","));
    }
    #[cfg(not(feature = "ci"))]
    {
        setup_command
            .env("RUSTC", rustc_path("zisk"))
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .args([
                "rom-setup",
                "-e",
                "./target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk",
            ])
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .current_dir("./bin/zisk");

        let proving_key = std::env::var("HOME")
            .map(|home| format!("{home}/.zisk/provingKey"))
            .unwrap_or_else(|_| ".zisk/provingKey".to_string());
        vkey_command
            .env("RUSTC", rustc_path("zisk"))
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .args([
                "rom-vkey",
                "-e",
                "./target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk",
                "-k",
                proving_key.as_str(),
                "-o",
                "../zisk/out/riscv64ima-zisk-vk",
            ])
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .current_dir("./bin/zisk");
    }

    println!("{build_command:?}");
    #[cfg(not(feature = "ci"))]
    println!("{setup_command:?}");
    #[cfg(not(feature = "ci"))]
    println!("{vkey_command:?}");

    println!("CWD = {}", std::env::current_dir().unwrap().display());

    let start = std::time::Instant::now();

    let build_status = build_command
        .status()
        .expect("Failed to execute zisk build command");

    #[cfg(not(feature = "ci"))]
    let setup_status = setup_command
        .status()
        .expect("Failed to execute zisk setup command");
    #[cfg(not(feature = "ci"))]
    let vkey_status = vkey_command
        .status()
        .expect("Failed to execute zisk rom-vkey command");

    let duration = start.elapsed();

    println!(
        "ZisK guest program built in {:.2?} seconds",
        duration.as_secs_f64()
    );

    if !build_status.success() {
        panic!("Failed to build guest program with zisk toolchain");
    }
    #[cfg(not(feature = "ci"))]
    if !setup_status.success() {
        panic!("Failed to setup compiled guest program with zisk toolchain");
    }
    #[cfg(not(feature = "ci"))]
    if !vkey_status.success() {
        panic!("Failed to generate ZisK verification key");
    }

    let _ = std::fs::create_dir("./bin/zisk/out");

    std::fs::copy(
        "./bin/zisk/target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk",
        "./bin/zisk/out/riscv64ima-zisk-elf",
    )
    .expect("could not copy Zisk elf to output directory");
}

#[cfg(all(not(clippy), feature = "openvm"))]
fn build_openvm_program() {
    use std::{
        fs,
        path::Path,
        process::{Command, Stdio},
    };

    let status = Command::new("cargo")
        .arg("openvm")
        .arg("build")
        .arg("--no-transpile")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .current_dir("./bin/openvm")
        .status()
        .expect("failed to execute cargo openvm build");

    if !status.success() {
        panic!("cargo openvm build failed with exit status: {}", status);
    }

    let elf_src =
        Path::new("./bin/openvm/target/riscv32im-risc0-zkvm-elf/release/ethrex-guest-openvm");
    let elf_dst = Path::new("./bin/openvm/out/riscv32im-openvm-elf");

    if let Some(parent) = elf_dst.parent() {
        fs::create_dir_all(parent).expect("failed to create destination dir");
    }

    fs::copy(&elf_src, &elf_dst).expect("failed to copy ethrex-guest-openvm");
}

#[cfg(all(not(clippy), feature = "zisk"))]
/// Returns the path to `rustc` executable of the given toolchain.
///
/// Taken from https://github.com/eth-act/ere/blob/master/crates/compile-utils/src/rust.rs#L166
pub fn rustc_path(toolchain: &str) -> std::path::PathBuf {
    let mut cmd = std::process::Command::new("rustc");
    let output = cmd
        .env("RUSTUP_TOOLCHAIN", toolchain)
        .args(["--print", "sysroot"])
        .output()
        .expect("Failed to execute rustc command");

    if !output.status.success() {
        panic!("Failed to get sysroot for toolchain {}", toolchain);
    }

    std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
        .join("bin")
        .join("rustc")
}
