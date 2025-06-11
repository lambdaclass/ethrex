use crate::errors::InternalError;
use ethrex_common::U256;

/// Special constant for debugging. 0xFEDEBEBECAFEDECEBADA
/// It has to match with the constant set in the Solidity contract for this purpose.
pub const MAGIC_PRINT_OFFSET: U256 = U256([0xBEBECAFEDECEBADA, 0xFEDE, 0, 0]);

#[derive(Default)]
pub struct DebugMode {
    pub enabled: bool,
    /// Accumulates chunks of data to print in one byte array.
    pub print_buffer: Vec<u8>,
    /// When enabled, store what's read into the buffer. Print the whole buffer when disabling this.
    pub print_mode: bool,
}

impl DebugMode {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }

    /// Returns true if the call resulted in a debug operation. False otherwise.
    pub fn handle_debug(&mut self, offset: U256, value: U256) -> Result<bool, InternalError> {
        if !self.enabled {
            return Ok(false);
        }

        if offset == MAGIC_PRINT_OFFSET {
            if !self.print_mode {
                self.print_mode = true;
            } else {
                if let Ok(s) = std::str::from_utf8(&self.print_buffer) {
                    println!("PRINTED -> {}", s);
                } else {
                    // Theoretically this shouldn't happen but I'll leave this JIC.
                    println!("PRINTED (failed) -> {:?}", self.print_buffer);
                }
                self.print_buffer.clear();
                self.print_mode = false;
            }

            return Ok(true);
        }

        if self.print_mode {
            // Accumulate chunks in buffer until there are no more chunks left, then print.
            let to_print = value.to_big_endian();
            self.print_buffer.extend_from_slice(&to_print);

            return Ok(true);
        }

        Ok(false)
    }
}
