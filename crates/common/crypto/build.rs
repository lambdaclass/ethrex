#[cfg(feature = "c-kzg")]
use c_kzg::KzgSettings;
use std::error::Error;

// This script downloads dependencies and compiles contracts to be embedded as constants in the deployer.

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo::rerun-if-changed=build.rs");

    #[cfg(feature = "c-kzg")]
    {
        use std::path::Path;

        let as_bytes = (c_kzg::ethereum_kzg_settings(8) as *const KzgSettings).cast::<u8>();
        let size = std::mem::size_of::<KzgSettings>();
        let slice = unsafe { std::slice::from_raw_parts(as_bytes, size) };
        let out_dir = std::env::var_os("OUT_DIR").unwrap();
        let out_dir = Path::new(&out_dir);
        std::fs::write(out_dir.join("kzg_settings.bin"), slice)?;
    }

    Ok(())
}
