//! Build script for LEVM JIT stencil extraction.
//!
//! When the `jit` feature is enabled, this script:
//! 1. Compiles stencil source files to object files
//! 2. Extracts function bytes and relocations using the `object` crate
//! 3. Generates `src/jit/stencils/generated.rs` with the stencil data

fn main() {
    #[cfg(feature = "jit")]
    jit::build_stencils();
}

#[cfg(feature = "jit")]
mod jit {
    use object::{Object, ObjectSection, ObjectSymbol, RelocationTarget};
    use std::collections::HashMap;
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::Command;

    /// Relocation kind for stencils
    #[derive(Debug, Clone, Copy)]
    enum RelocKind {
        NextStencil,
        ExitJit,
        ImmediateValue,
    }

    /// A relocation found in the object file
    #[derive(Debug)]
    struct Relocation {
        offset: usize,
        kind: RelocKind,
        size: u8,
    }

    /// Extracted stencil data
    #[derive(Debug)]
    struct StencilData {
        name: String,
        bytes: Vec<u8>,
        relocations: Vec<Relocation>,
    }

    pub fn build_stencils() {
        println!("cargo:rerun-if-changed=src/jit/stencils/");

        let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");

        // Create a temporary crate for stencil compilation
        let stencil_crate_dir = PathBuf::from(&out_dir).join("stencil_crate");
        std::fs::create_dir_all(&stencil_crate_dir).expect("Failed to create stencil crate dir");

        // Write the stencil crate's Cargo.toml
        // Important: [workspace] is empty to exclude from parent workspace
        let cargo_toml = r#"
[package]
name = "levm_stencils"
version = "0.1.0"
edition = "2021"

[workspace]

[lib]
crate-type = ["staticlib"]

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
panic = "abort"
"#;
        std::fs::write(stencil_crate_dir.join("Cargo.toml"), cargo_toml)
            .expect("Failed to write Cargo.toml");

        // Create src directory
        let stencil_src_dir = stencil_crate_dir.join("src");
        std::fs::create_dir_all(&stencil_src_dir).expect("Failed to create src dir");

        // Copy stencil source files
        let source_dir = PathBuf::from(&manifest_dir).join("src/jit/stencils");
        copy_stencil_sources(&source_dir, &stencil_src_dir);

        // Write lib.rs that includes all stencils
        let lib_rs = r#"
#![no_std]
#![allow(unused_unsafe)]
#![allow(clippy::all)]
#![allow(unused_variables)]

mod context;
mod markers;
mod arithmetic;
mod stack;
mod control;

// Re-export all stencil functions
pub use arithmetic::*;
pub use stack::*;
pub use control::*;

// Panic handler for no_std
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
"#;
        std::fs::write(stencil_src_dir.join("lib.rs"), lib_rs).expect("Failed to write lib.rs");

        // Compile stencils
        let object_path = compile_stencils(&stencil_crate_dir, &out_dir);

        // Extract stencils from object file
        let stencils = extract_stencils(&object_path);

        // Generate Rust code
        let generated_path = PathBuf::from(&manifest_dir).join("src/jit/stencils/generated.rs");
        generate_rust_code(&stencils, &generated_path);

        println!("cargo:rerun-if-changed=build.rs");
    }

    fn copy_stencil_sources(source_dir: &PathBuf, dest_dir: &PathBuf) {
        let files = ["context.rs", "markers.rs", "arithmetic.rs", "stack.rs", "control.rs"];
        for file in &files {
            let src = source_dir.join(file);
            let dst = dest_dir.join(file);
            if src.exists() {
                std::fs::copy(&src, &dst)
                    .unwrap_or_else(|e| panic!("Failed to copy {}: {}", file, e));
            }
        }
    }

    fn compile_stencils(crate_dir: &PathBuf, out_dir: &str) -> PathBuf {
        // Build the stencil crate with frame pointers disabled
        // This prevents the compiler from generating function prologues that push frames,
        // allowing stencils to chain via tail jumps without stack frame buildup.
        let status = Command::new("cargo")
            .current_dir(crate_dir)
            .env("RUSTFLAGS", "-C force-frame-pointers=no -C opt-level=3")
            .args([
                "build",
                "--release",
                "--target-dir",
                &format!("{}/stencil_target", out_dir),
            ])
            .status()
            .expect("Failed to run cargo build for stencils");

        if !status.success() {
            panic!("Stencil compilation failed");
        }

        // Find the compiled object/archive file
        // For staticlib, cargo produces a .a file
        let target_dir = PathBuf::from(out_dir).join("stencil_target/release");

        // Look for the staticlib
        let lib_name = if cfg!(target_os = "macos") {
            "liblevm_stencils.a"
        } else {
            "liblevm_stencils.a"
        };

        let lib_path = target_dir.join(lib_name);
        if !lib_path.exists() {
            // Try to find any .a file
            for entry in std::fs::read_dir(&target_dir).expect("Failed to read target dir") {
                let entry = entry.expect("Failed to read entry");
                let path = entry.path();
                if path.extension().map(|e| e == "a").unwrap_or(false) {
                    return path;
                }
            }
            panic!(
                "Could not find compiled stencil library at {:?}. Target dir contents: {:?}",
                lib_path,
                std::fs::read_dir(&target_dir)
                    .map(|entries| entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .collect::<Vec<_>>())
                    .unwrap_or_default()
            );
        }

        lib_path
    }

    fn extract_stencils(archive_path: &PathBuf) -> Vec<StencilData> {
        let data = std::fs::read(archive_path).expect("Failed to read archive");

        // Archives contain object files - we need to extract them
        let archive = object::read::archive::ArchiveFile::parse(&*data)
            .expect("Failed to parse archive");

        let mut stencils = Vec::new();

        for member in archive.members() {
            let member = member.expect("Failed to read archive member");
            let member_data = member.data(&*data).expect("Failed to get member data");

            // Try to parse as object file
            if let Ok(obj) = object::File::parse(member_data) {
                let mut extracted = extract_from_object(&obj);
                stencils.append(&mut extracted);
            }
        }

        if stencils.is_empty() {
            println!("cargo:warning=No stencils found in archive. This may be expected on first build.");
        }

        stencils
    }

    #[allow(elided_lifetimes_in_paths)]
    fn extract_from_object(obj: &object::File) -> Vec<StencilData> {
        let mut stencils = Vec::new();

        // Collect all stencil symbols with their addresses
        let mut stencil_symbols: Vec<(String, usize, usize)> = Vec::new(); // (name, section_idx, addr)
        let mut section_sizes: HashMap<usize, usize> = HashMap::new();

        // First pass: collect stencil symbols
        for symbol in obj.symbols() {
            if let Ok(name) = symbol.name() {
                if name.starts_with("stencil_") || name.starts_with("_stencil_") {
                    let clean_name = name.strip_prefix('_').unwrap_or(name);
                    if let Some(section_idx) = symbol.section_index() {
                        stencil_symbols.push((
                            clean_name.to_string(),
                            section_idx.0,
                            symbol.address() as usize,
                        ));
                    }
                }
            }
        }

        // Get section sizes
        for section in obj.sections() {
            section_sizes.insert(section.index().0, section.size() as usize);
        }

        // Sort by address within each section to calculate sizes
        stencil_symbols.sort_by_key(|(_, section, addr)| (*section, *addr));

        // Calculate sizes from address gaps
        let mut symbol_map: HashMap<String, (usize, usize, usize)> = HashMap::new();
        for i in 0..stencil_symbols.len() {
            let (name, section_idx, addr) = &stencil_symbols[i];

            // Find next symbol in same section, or use section end
            let size = if i + 1 < stencil_symbols.len() && stencil_symbols[i + 1].1 == *section_idx {
                stencil_symbols[i + 1].2 - addr
            } else {
                // Last symbol in section - use section size
                section_sizes.get(section_idx).unwrap_or(&0).saturating_sub(*addr)
            };

            symbol_map.insert(name.clone(), (*section_idx, *addr, size));
        }

        // Extract each stencil
        for (name, (section_idx, addr, size)) in &symbol_map {
            let section = match obj.section_by_index(object::SectionIndex(*section_idx)) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let section_data = match section.data() {
                Ok(d) => d,
                Err(_) => continue,
            };

            let section_addr = section.address() as usize;
            let offset_in_section = addr.saturating_sub(section_addr);

            if offset_in_section + size > section_data.len() {
                println!(
                    "cargo:warning=Stencil {} extends beyond section bounds",
                    name
                );
                continue;
            }

            let bytes = section_data[offset_in_section..offset_in_section + size].to_vec();

            // Find relocations for this symbol
            let mut relocations = Vec::new();
            for (reloc_offset, reloc) in section.relocations() {
                let reloc_offset = reloc_offset as usize;

                // Check if relocation is within this symbol's range
                if reloc_offset >= offset_in_section && reloc_offset < offset_in_section + size {
                    let reloc_offset_in_stencil = reloc_offset - offset_in_section;

                    // Determine relocation kind from target symbol name
                    let kind = match reloc.target() {
                        RelocationTarget::Symbol(sym_idx) => {
                            if let Ok(sym) = obj.symbol_by_index(sym_idx) {
                                if let Ok(sym_name) = sym.name() {
                                    let clean_name = sym_name.strip_prefix('_').unwrap_or(sym_name);
                                    match clean_name {
                                        "NEXT_STENCIL" => Some(RelocKind::NextStencil),
                                        "EXIT_JIT" => Some(RelocKind::ExitJit),
                                        "IMMEDIATE_VALUE" => Some(RelocKind::ImmediateValue),
                                        _ => None,
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };

                    if let Some(kind) = kind {
                        // Determine relocation size based on relocation kind
                        let size = match reloc.kind() {
                            object::RelocationKind::Relative => 4, // 32-bit PC-relative
                            object::RelocationKind::Absolute => 8, // 64-bit absolute
                            _ => 4, // Default to 4
                        };

                        relocations.push(Relocation {
                            offset: reloc_offset_in_stencil,
                            kind,
                            size,
                        });
                    }
                }
            }

            stencils.push(StencilData {
                name: name.clone(),
                bytes,
                relocations,
            });
        }

        stencils
    }

    fn generate_rust_code(stencils: &[StencilData], output_path: &PathBuf) {
        let mut output = String::new();

        output.push_str("// AUTO-GENERATED FILE - DO NOT EDIT\n");
        output.push_str("// Generated by build.rs from stencil source files\n\n");
        output.push_str("use super::{Stencil, Relocation, RelocKind};\n\n");

        for stencil in stencils {
            let const_name = stencil.name.to_uppercase();

            output.push_str(&format!("pub static {}: Stencil = Stencil {{\n", const_name));

            // Write bytes
            output.push_str("    bytes: &[\n        ");
            for (i, byte) in stencil.bytes.iter().enumerate() {
                output.push_str(&format!("0x{:02x}, ", byte));
                if (i + 1) % 16 == 0 && i + 1 < stencil.bytes.len() {
                    output.push_str("\n        ");
                }
            }
            output.push_str("\n    ],\n");

            // Write relocations
            output.push_str("    relocations: &[\n");
            for reloc in &stencil.relocations {
                let kind_str = match reloc.kind {
                    RelocKind::NextStencil => "RelocKind::NextStencil",
                    RelocKind::ExitJit => "RelocKind::ExitJit",
                    RelocKind::ImmediateValue => "RelocKind::ImmediateValue",
                };
                output.push_str(&format!(
                    "        Relocation {{ offset: {}, kind: {}, size: {} }},\n",
                    reloc.offset, kind_str, reloc.size
                ));
            }
            output.push_str("    ],\n");

            output.push_str("};\n\n");
        }

        // If no stencils were extracted, generate empty placeholders
        if stencils.is_empty() {
            output.push_str("// No stencils extracted - using empty placeholders\n\n");

            for name in &[
                "STENCIL_STOP",
                "STENCIL_ADD",
                "STENCIL_SUB",
                "STENCIL_MUL",
                "STENCIL_POP",
                "STENCIL_PUSH",
            ] {
                output.push_str(&format!(
                    "pub static {}: Stencil = Stencil {{ bytes: &[], relocations: &[] }};\n",
                    name
                ));
            }
        }

        // Create parent directory if needed
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent).expect("Failed to create output directory");
        }

        let mut file = File::create(output_path).expect("Failed to create generated.rs");
        file.write_all(output.as_bytes())
            .expect("Failed to write generated.rs");

        println!("cargo:rerun-if-changed={}", output_path.display());
    }
}
