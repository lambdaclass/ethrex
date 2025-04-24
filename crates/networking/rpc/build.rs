use std::error::Error;
use vergen::*;

fn main() -> Result<(), Box<dyn Error>> {
    let rustc = RustcBuilder::all_rustc()?;

    Emitter::default().add_instructions(&rustc)?.emit()?;
    Ok(())
}
