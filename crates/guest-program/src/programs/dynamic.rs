use std::collections::HashMap;
use std::path::Path;

use crate::traits::{GuestProgram, GuestProgramError, validate_elf_header};

/// A guest program that loads ELF binaries from the filesystem at runtime.
///
/// Unlike compile-time guest programs (e.g., [`EvmL2GuestProgram`]) that embed
/// ELF binaries via `include_bytes!()`, this implementation reads ELF files
/// from disk.  This allows adding or updating guest programs **without
/// recompiling** the prover binary.
///
/// # Construction
///
/// Use the builder API to create a `DynamicGuestProgram`:
///
/// ```no_run
/// use ethrex_guest_program::programs::dynamic::DynamicGuestProgram;
/// use ethrex_guest_program::traits::backends;
///
/// let program = DynamicGuestProgram::builder("my-program", 10)
///     .elf_from_file(backends::SP1, "/path/to/sp1/elf")
///     .unwrap()
///     .vk_from_file(backends::SP1, "/path/to/sp1/vk")
///     .unwrap()
///     .build();
/// ```
///
/// Or load from a standard directory layout:
///
/// ```no_run
/// use ethrex_guest_program::programs::dynamic::DynamicGuestProgram;
///
/// // Expects: <dir>/sp1/elf, <dir>/risc0/elf, etc.
/// let program = DynamicGuestProgram::from_dir("my-program", 10, "/path/to/elfs")
///     .unwrap();
/// ```
///
/// # ELF validation
///
/// By default, each ELF is validated on load (magic number, RISC-V class,
/// machine type).  Disable with [`DynamicGuestProgramBuilder::skip_validation`].
pub struct DynamicGuestProgram {
    id: String,
    type_id: u8,
    elfs: HashMap<String, Vec<u8>>,
    vks: HashMap<String, Vec<u8>>,
}

impl DynamicGuestProgram {
    /// Create a new builder for constructing a `DynamicGuestProgram`.
    pub fn builder(program_id: &str, program_type_id: u8) -> DynamicGuestProgramBuilder {
        DynamicGuestProgramBuilder {
            id: program_id.to_string(),
            type_id: program_type_id,
            elfs: HashMap::new(),
            vks: HashMap::new(),
            validate: true,
        }
    }

    /// Load ELF binaries from a standard directory layout.
    ///
    /// Scans `<dir>/<backend>/elf` for each known backend (`sp1`, `risc0`,
    /// `zisk`, `openvm`).  Missing directories are silently skipped.
    /// VK files are loaded from `<dir>/<backend>/vk` when present.
    ///
    /// Returns an error only if a found ELF file fails validation.
    pub fn from_dir(
        program_id: &str,
        program_type_id: u8,
        dir: impl AsRef<Path>,
    ) -> Result<Self, GuestProgramError> {
        use crate::traits::backends;

        let dir = dir.as_ref();
        let known_backends = [
            backends::SP1,
            backends::RISC0,
            backends::ZISK,
            backends::OPENVM,
        ];

        let mut builder = Self::builder(program_id, program_type_id);

        for backend in &known_backends {
            let elf_path = dir.join(backend).join("elf");
            if elf_path.is_file() {
                builder = builder.elf_from_file(backend, &elf_path)?;
            }

            let vk_path = dir.join(backend).join("vk");
            if vk_path.is_file() {
                builder = builder.vk_from_file(backend, &vk_path)?;
            }
        }

        Ok(builder.build())
    }

    /// Return the set of backends for which an ELF is loaded.
    pub fn loaded_backends(&self) -> Vec<&str> {
        self.elfs.keys().map(|s| s.as_str()).collect()
    }
}

/// Builder for [`DynamicGuestProgram`].
#[derive(Debug)]
pub struct DynamicGuestProgramBuilder {
    id: String,
    type_id: u8,
    elfs: HashMap<String, Vec<u8>>,
    vks: HashMap<String, Vec<u8>>,
    validate: bool,
}

impl DynamicGuestProgramBuilder {
    /// Load an ELF binary from a file for the given backend.
    pub fn elf_from_file(
        mut self,
        backend: &str,
        path: impl AsRef<Path>,
    ) -> Result<Self, GuestProgramError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| {
            GuestProgramError::Internal(format!("failed to read ELF from {}: {e}", path.display(),))
        })?;
        if self.validate {
            validate_elf_header(backend, &bytes)?;
        }
        self.elfs.insert(backend.to_string(), bytes);
        Ok(self)
    }

    /// Load an ELF binary from raw bytes for the given backend.
    pub fn elf_from_bytes(
        mut self,
        backend: &str,
        bytes: Vec<u8>,
    ) -> Result<Self, GuestProgramError> {
        if self.validate {
            validate_elf_header(backend, &bytes)?;
        }
        self.elfs.insert(backend.to_string(), bytes);
        Ok(self)
    }

    /// Load a verification key from a file for the given backend.
    pub fn vk_from_file(
        mut self,
        backend: &str,
        path: impl AsRef<Path>,
    ) -> Result<Self, GuestProgramError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| {
            GuestProgramError::Internal(format!("failed to read VK from {}: {e}", path.display(),))
        })?;
        self.vks.insert(backend.to_string(), bytes);
        Ok(self)
    }

    /// Load a verification key from raw bytes for the given backend.
    pub fn vk_from_bytes(mut self, backend: &str, bytes: Vec<u8>) -> Self {
        self.vks.insert(backend.to_string(), bytes);
        self
    }

    /// Disable ELF header validation on load.
    ///
    /// Useful when loading ELFs for backends with non-standard header formats.
    pub fn skip_validation(mut self) -> Self {
        self.validate = false;
        self
    }

    /// Build the [`DynamicGuestProgram`].
    pub fn build(self) -> DynamicGuestProgram {
        DynamicGuestProgram {
            id: self.id,
            type_id: self.type_id,
            elfs: self.elfs,
            vks: self.vks,
        }
    }
}

impl GuestProgram for DynamicGuestProgram {
    fn program_id(&self) -> &str {
        &self.id
    }

    fn elf(&self, backend: &str) -> Option<&[u8]> {
        self.elfs.get(backend).map(|v| v.as_slice())
    }

    fn vk_bytes(&self, backend: &str) -> Option<Vec<u8>> {
        self.vks.get(backend).cloned()
    }

    fn program_type_id(&self) -> u8 {
        self.type_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::backends;
    use std::io::Write;

    /// Build a minimal valid RISC-V ELF header.
    fn make_elf(class: u8, machine: u16) -> Vec<u8> {
        let mut buf = vec![0u8; 20];
        buf[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        buf[4] = class;
        buf[18..20].copy_from_slice(&machine.to_le_bytes());
        buf
    }

    const ELFCLASS32: u8 = 1;
    const EM_RISCV: u16 = 243;

    #[test]
    fn builder_from_bytes() {
        let elf = make_elf(ELFCLASS32, EM_RISCV);
        let prog = DynamicGuestProgram::builder("test-prog", 42)
            .elf_from_bytes(backends::SP1, elf.clone())
            .unwrap()
            .build();

        assert_eq!(prog.program_id(), "test-prog");
        assert_eq!(prog.program_type_id(), 42);
        assert_eq!(prog.elf(backends::SP1), Some(elf.as_slice()));
        assert!(prog.elf(backends::RISC0).is_none());
    }

    #[test]
    fn builder_validates_elf_by_default() {
        let bad_elf = vec![0u8; 20]; // no magic
        let result = DynamicGuestProgram::builder("test", 1).elf_from_bytes(backends::SP1, bad_elf);
        assert!(result.is_err());
    }

    #[test]
    fn builder_skip_validation() {
        let bad_elf = vec![0u8; 20]; // no magic
        let prog = DynamicGuestProgram::builder("test", 1)
            .skip_validation()
            .elf_from_bytes(backends::SP1, bad_elf.clone())
            .unwrap()
            .build();

        assert_eq!(prog.elf(backends::SP1), Some(bad_elf.as_slice()));
    }

    #[test]
    fn builder_rejects_wrong_class() {
        // SP1 expects 32-bit; give it 64-bit (class = 2)
        let elf64 = make_elf(2, EM_RISCV);
        let result = DynamicGuestProgram::builder("test", 1).elf_from_bytes(backends::SP1, elf64);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(format!("{err}").contains("class mismatch"));
    }

    #[test]
    fn builder_from_file() {
        let elf = make_elf(ELFCLASS32, EM_RISCV);
        let dir = tempfile::tempdir().unwrap();
        let elf_path = dir.path().join("test.elf");
        let mut f = std::fs::File::create(&elf_path).unwrap();
        f.write_all(&elf).unwrap();

        let prog = DynamicGuestProgram::builder("file-prog", 5)
            .elf_from_file(backends::SP1, &elf_path)
            .unwrap()
            .build();

        assert_eq!(prog.elf(backends::SP1), Some(elf.as_slice()));
    }

    #[test]
    fn builder_from_file_missing_returns_error() {
        let result = DynamicGuestProgram::builder("test", 1)
            .elf_from_file(backends::SP1, "/nonexistent/path/elf");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(format!("{err}").contains("failed to read ELF"));
    }

    #[test]
    fn vk_from_bytes() {
        let elf = make_elf(ELFCLASS32, EM_RISCV);
        let vk = vec![0xAA; 32];
        let prog = DynamicGuestProgram::builder("vk-test", 7)
            .elf_from_bytes(backends::SP1, elf)
            .unwrap()
            .vk_from_bytes(backends::SP1, vk.clone())
            .build();

        assert_eq!(prog.vk_bytes(backends::SP1), Some(vk));
        assert!(prog.vk_bytes(backends::RISC0).is_none());
    }

    #[test]
    fn vk_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let vk_path = dir.path().join("test.vk");
        let vk_data = vec![0xBB; 64];
        std::fs::write(&vk_path, &vk_data).unwrap();

        let prog = DynamicGuestProgram::builder("vk-file", 8)
            .vk_from_file(backends::SP1, &vk_path)
            .unwrap()
            .build();

        assert_eq!(prog.vk_bytes(backends::SP1), Some(vk_data));
    }

    #[test]
    fn from_dir_loads_available_backends() {
        let dir = tempfile::tempdir().unwrap();

        // Create sp1/elf
        let sp1_dir = dir.path().join("sp1");
        std::fs::create_dir(&sp1_dir).unwrap();
        let elf32 = make_elf(ELFCLASS32, EM_RISCV);
        std::fs::write(sp1_dir.join("elf"), &elf32).unwrap();
        std::fs::write(sp1_dir.join("vk"), vec![0xCC; 32]).unwrap();

        let prog = DynamicGuestProgram::from_dir("dir-prog", 10, dir.path()).unwrap();

        assert_eq!(prog.program_id(), "dir-prog");
        assert_eq!(prog.program_type_id(), 10);
        assert_eq!(prog.elf(backends::SP1), Some(elf32.as_slice()));
        assert_eq!(prog.vk_bytes(backends::SP1), Some(vec![0xCC; 32]));
        // No risc0 directory → None
        assert!(prog.elf(backends::RISC0).is_none());
    }

    #[test]
    fn from_dir_skips_missing_backends() {
        let dir = tempfile::tempdir().unwrap();
        // Empty directory — no backend subdirectories
        let prog = DynamicGuestProgram::from_dir("empty", 20, dir.path()).unwrap();
        assert!(prog.elf(backends::SP1).is_none());
        assert!(prog.elf(backends::RISC0).is_none());
        assert!(prog.loaded_backends().is_empty());
    }

    #[test]
    fn from_dir_validates_elf() {
        let dir = tempfile::tempdir().unwrap();
        let sp1_dir = dir.path().join("sp1");
        std::fs::create_dir(&sp1_dir).unwrap();
        // Write invalid ELF (no magic)
        std::fs::write(sp1_dir.join("elf"), vec![0u8; 20]).unwrap();

        let result = DynamicGuestProgram::from_dir("bad-elf", 1, dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn loaded_backends_returns_correct_set() {
        let elf = make_elf(ELFCLASS32, EM_RISCV);
        let prog = DynamicGuestProgram::builder("multi", 3)
            .elf_from_bytes(backends::SP1, elf.clone())
            .unwrap()
            .elf_from_bytes(backends::RISC0, elf)
            .unwrap()
            .build();

        let mut backends_list = prog.loaded_backends();
        backends_list.sort();
        assert_eq!(backends_list, vec!["risc0", "sp1"]);
    }

    #[test]
    fn multiple_backends_from_dir() {
        let dir = tempfile::tempdir().unwrap();

        // SP1 (32-bit)
        let sp1_dir = dir.path().join("sp1");
        std::fs::create_dir(&sp1_dir).unwrap();
        std::fs::write(sp1_dir.join("elf"), make_elf(ELFCLASS32, EM_RISCV)).unwrap();

        // ZisK (64-bit)
        let zisk_dir = dir.path().join("zisk");
        std::fs::create_dir(&zisk_dir).unwrap();
        std::fs::write(zisk_dir.join("elf"), make_elf(2, EM_RISCV)).unwrap(); // class 2 = 64-bit

        let prog = DynamicGuestProgram::from_dir("multi-dir", 15, dir.path()).unwrap();

        assert!(prog.elf(backends::SP1).is_some());
        assert!(prog.elf(backends::ZISK).is_some());
        assert!(prog.elf(backends::RISC0).is_none());
    }

    #[test]
    fn implements_guest_program_trait() {
        let elf = make_elf(ELFCLASS32, EM_RISCV);
        let prog = DynamicGuestProgram::builder("trait-test", 99)
            .elf_from_bytes(backends::SP1, elf)
            .unwrap()
            .build();

        // Verify trait methods work through a trait object.
        let trait_obj: &dyn GuestProgram = &prog;
        assert_eq!(trait_obj.program_id(), "trait-test");
        assert_eq!(trait_obj.program_type_id(), 99);
        assert!(trait_obj.elf(backends::SP1).is_some());

        // Default serialize_input and encode_output are pass-through.
        let data = b"test data";
        assert_eq!(trait_obj.serialize_input(data).unwrap(), data);
        assert_eq!(trait_obj.encode_output(data).unwrap(), data);
    }

    #[test]
    fn can_register_in_arc() {
        use std::sync::Arc;

        let elf = make_elf(ELFCLASS32, EM_RISCV);
        let prog = DynamicGuestProgram::builder("arc-test", 50)
            .elf_from_bytes(backends::SP1, elf)
            .unwrap()
            .build();

        let arc: Arc<dyn GuestProgram> = Arc::new(prog);
        assert_eq!(arc.program_id(), "arc-test");
        assert!(arc.elf(backends::SP1).is_some());
    }
}
