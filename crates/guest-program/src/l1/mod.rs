mod input;
mod output;
mod program;

pub use input::ProgramInput;
#[cfg(feature = "eip-8025")]
pub use input::{
    CanonicalChainConfig, CanonicalExecutionWitness, CanonicalStatelessInput, DecodedEip8025,
    EIP8025_VERSION_CANONICAL, EIP8025_VERSION_LEGACY, STATELESS_INPUT_SCHEMA_ID,
    STATELESS_INPUT_SCHEMA_ID_SIZE, decode_eip8025, encode_eip8025,
};
#[cfg(feature = "eip-8025")]
pub use input::{ProgramInputDecodeError, ProgramInputEncodeError};
pub use output::ProgramOutput;
#[cfg(feature = "eip-8025")]
pub use program::execute_canonical_stateless_input;
pub use program::execution_program;
