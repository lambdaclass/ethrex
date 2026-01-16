//! Core data structures for trie navigation and in-page storage.

mod nibble_path;
mod slotted_array;

#[cfg(test)]
mod tests;

pub use nibble_path::NibblePath;
pub use slotted_array::SlottedArray;
