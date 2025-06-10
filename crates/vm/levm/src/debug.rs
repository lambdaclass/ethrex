use crate::{constants::DEBUG_MEMORY_OFFSET, errors::InternalError};
use ethrex_common::U256;

#[derive(Default)]
pub struct DebugMode {
    pub enabled: bool,
    pub chunks_left: u8,
    pub print_buffer: Vec<u8>,
}

impl DebugMode {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }

    pub fn handle_debug(&mut self, offset: U256, value: U256) -> Result<(), InternalError> {
        if offset == DEBUG_MEMORY_OFFSET {
            // Get the amount of chunks to print. Each chunk will have one MSTORE associated with it.
            let chunks_to_print = value
                .try_into()
                .map_err(|_| InternalError::Custom("Debug Mode error".to_string()))?;

            self.chunks_left = self
                .chunks_left
                .checked_add(chunks_to_print)
                .ok_or(InternalError::Custom("Debug Mode error".to_string()))?;

            return Ok(());
        }

        if self.chunks_left > 0 {
            // Accumulate chunks in buffer until there are no more chunks left, then print.
            let to_print = value.to_big_endian();
            self.print_buffer.extend_from_slice(&to_print);

            self.chunks_left = self
                .chunks_left
                .checked_sub(1)
                .ok_or(InternalError::Custom("Debug Mode error".to_string()))?;

            // Print if this was the last chunk to read.
            if self.chunks_left == 0 {
                if let Ok(s) = std::str::from_utf8(&self.print_buffer) {
                    println!("PRINTED -> {}", s);
                } else {
                    // This shouldn't ever happen if the contract works fine but we are not going to return an internal error because of it...
                    println!("PRINTED FAILED -> {:?}", &self.print_buffer);
                }
                self.print_buffer.clear();
            }
        }
        Ok(())
    }
}
