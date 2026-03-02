use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set by cargo");
    let gevm_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../gevm")
        .canonicalize()
        .expect("gevm directory not found");

    let archive_path = format!("{out_dir}/libgevm.a");

    let status = Command::new("go")
        .args(["build", "-buildmode=c-archive", "-o", &archive_path, "./capi/"])
        .current_dir(&gevm_dir)
        .env("CGO_ENABLED", "1")
        // Remove any user-set GOROOT that may point to the wrong location;
        // the go binary uses its compiled-in default when GOROOT is unset.
        .env_remove("GOROOT")
        .status()
        .expect("go build failed - is Go installed?");

    assert!(status.success(), "gevm build failed");

    println!("cargo:rustc-link-search=native={out_dir}");
    println!("cargo:rustc-link-lib=static=gevm");
    println!("cargo:rustc-link-lib=pthread");
    println!("cargo:rustc-link-lib=m");
    // macOS needs:
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=framework=CoreFoundation");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}/capi/", gevm_dir.display());
}
