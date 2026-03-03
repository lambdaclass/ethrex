use std::env;

fn main() {
    let mut build = cc::Build::new();

    build
        .file("libmdbx/mdbx.c")
        .define("MDBX_BUILD_SHARED_LIBRARY", "0")
        .define("NDEBUG", None)
        // Allow cross-thread transaction use (needed for Send/Sync patterns)
        .define("MDBX_TXN_CHECKOWNER", "0")
        // Disable C++ API
        .define("MDBX_BUILD_CXX", "0");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "macos" | "ios" => {
            // macOS uses libSystem which includes pthreads
            println!("cargo:rustc-link-lib=System");
        }
        "linux" | "android" => {
            println!("cargo:rustc-link-lib=pthread");
        }
        "windows" => {
            build.define("_WIN32", None);
            println!("cargo:rustc-link-lib=ntdll");
        }
        _ => {}
    }

    build.warnings(false);
    build.compile("mdbx");
}
