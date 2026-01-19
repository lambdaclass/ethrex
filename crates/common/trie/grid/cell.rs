//! Cell structure for the grid-based trie.
//!
//! Each Cell represents a node at a specific position in the grid (row, column).
//! Row corresponds to depth in the nibble path (0-127 for 64-byte keys).
//! Column corresponds to the nibble value at that depth (0-15).

use crate::{nibbles::Nibbles, node_hash::NodeHash};

/// Maximum depth for the grid (64 nibbles for account + 64 for storage = 128)
pub const MAX_DEPTH: usize = 128;

/// Number of nibbles per row (hexadecimal)
pub const NIBBLE_COUNT: usize = 16;

/// Account key length in nibbles (32 bytes = 64 nibbles)
pub const ACCOUNT_KEY_NIBBLES: usize = 64;

/// A single cell in the grid trie.
///
/// Cells store information about a trie node at a specific depth and nibble position.
/// They can represent branch points, extension paths, or leaf values.
#[derive(Debug, Clone)]
pub struct Cell {
    /// Extension prefix - remaining nibbles after this cell's position.
    /// Empty for branch nodes, non-empty for extension/leaf nodes.
    pub extension: Nibbles,

    /// Computed hash of the subtrie rooted at this cell.
    /// None if not yet computed or if cell is empty.
    pub hash: Option<NodeHash>,

    /// Value stored at this cell (for leaf nodes).
    /// None for branch/extension nodes.
    pub value: Option<Vec<u8>>,

    /// Depth of this cell in the trie (0 = root level).
    pub depth: u8,

    /// Whether this cell has been modified and needs hash recomputation.
    pub dirty: bool,

    /// Whether this cell represents a deleted entry.
    pub deleted: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self::new()
    }
}

impl Cell {
    /// Create a new empty cell.
    pub fn new() -> Self {
        Self {
            extension: Nibbles::default(),
            hash: None,
            value: None,
            depth: 0,
            dirty: false,
            deleted: false,
        }
    }

    /// Create a cell with a specific depth.
    pub fn with_depth(depth: u8) -> Self {
        Self {
            extension: Nibbles::default(),
            hash: None,
            value: None,
            depth,
            dirty: false,
            deleted: false,
        }
    }

    /// Reset this cell to its initial empty state.
    pub fn reset(&mut self) {
        self.extension = Nibbles::default();
        self.hash = None;
        self.value = None;
        self.dirty = false;
        self.deleted = false;
    }

    /// Check if this cell is empty (no hash, no extension, no value).
    pub fn is_empty(&self) -> bool {
        self.hash.is_none() && self.extension.is_empty() && self.value.is_none() && !self.deleted
    }

    /// Check if this cell has a valid hash.
    pub fn has_hash(&self) -> bool {
        self.hash.as_ref().map_or(false, |h| h.is_valid())
    }

    /// Check if this cell is a leaf (has a value).
    pub fn is_leaf(&self) -> bool {
        self.value.is_some()
    }

    /// Set the value for this cell (making it a leaf).
    pub fn set_value(&mut self, value: Vec<u8>) {
        self.value = Some(value);
        self.dirty = true;
        self.deleted = false;
    }

    /// Mark this cell as deleted.
    pub fn mark_deleted(&mut self) {
        self.deleted = true;
        self.value = None;
        self.dirty = true;
    }

    /// Set the hash for this cell.
    pub fn set_hash(&mut self, hash: NodeHash) {
        self.hash = Some(hash);
    }

    /// Clear the hash (invalidate cached value).
    pub fn clear_hash(&mut self) {
        self.hash = None;
    }

    /// Set the extension nibbles for this cell.
    pub fn set_extension(&mut self, extension: Nibbles) {
        self.extension = extension;
    }

    /// Fill this cell from a parent cell during unfold operation.
    ///
    /// This copies relevant data from the parent cell when expanding
    /// the grid to a deeper level.
    ///
    /// # Arguments
    /// * `parent` - The parent cell to copy from
    /// * `consumed_nibbles` - Number of nibbles consumed during unfold
    pub fn fill_from_parent(&mut self, parent: &Cell, consumed_nibbles: usize) {
        // Copy extension minus consumed nibbles
        if parent.extension.len() > consumed_nibbles {
            self.extension = parent.extension.slice(consumed_nibbles, parent.extension.len());
        } else {
            self.extension = Nibbles::default();
        }

        // Copy hash (will be invalidated if children change)
        self.hash = parent.hash;

        // Value stays with the leaf node
        if parent.extension.len() == consumed_nibbles {
            self.value = parent.value.clone();
        }

        // Inherit dirty state
        self.dirty = parent.dirty;
        self.deleted = parent.deleted;
    }

    /// Fill parent cell from this cell during fold operation.
    ///
    /// This propagates data back to the parent when reducing
    /// the grid depth.
    ///
    /// # Arguments
    /// * `parent` - The parent cell to fill
    /// * `prefix_nibbles` - Nibbles to prepend to extension
    pub fn fill_to_parent(&self, parent: &mut Cell, prefix_nibbles: &Nibbles) {
        // Prepend nibbles to extension
        if prefix_nibbles.is_empty() {
            parent.extension = self.extension.clone();
        } else {
            parent.extension = prefix_nibbles.concat(&self.extension);
        }

        // Copy hash
        parent.hash = self.hash;

        // Copy value for leaf nodes
        parent.value = self.value.clone();

        // Propagate dirty state
        parent.dirty = self.dirty;
        parent.deleted = self.deleted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cell_new() {
        let cell = Cell::new();
        assert!(cell.is_empty());
        assert!(!cell.has_hash());
        assert!(!cell.is_leaf());
        assert!(!cell.dirty);
        assert!(!cell.deleted);
    }

    #[test]
    fn test_cell_with_depth() {
        let cell = Cell::with_depth(42);
        assert_eq!(cell.depth, 42);
        assert!(cell.is_empty());
    }

    #[test]
    fn test_cell_set_value() {
        let mut cell = Cell::new();
        cell.set_value(vec![1, 2, 3]);

        assert!(cell.is_leaf());
        assert!(cell.dirty);
        assert!(!cell.is_empty());
        assert_eq!(cell.value, Some(vec![1, 2, 3]));
    }

    #[test]
    fn test_cell_mark_deleted() {
        let mut cell = Cell::new();
        cell.set_value(vec![1, 2, 3]);

        cell.mark_deleted();

        assert!(cell.deleted);
        assert!(cell.dirty);
        assert!(cell.value.is_none());
        assert!(!cell.is_leaf());
    }

    #[test]
    fn test_cell_reset() {
        let mut cell = Cell::new();
        cell.set_value(vec![1, 2, 3]);
        cell.dirty = true;
        cell.depth = 10;

        cell.reset();

        assert!(cell.is_empty());
        assert!(!cell.dirty);
        // Note: depth is not reset by reset()
        assert_eq!(cell.depth, 10);
    }

    #[test]
    fn test_cell_fill_from_parent() {
        let mut parent = Cell::new();
        parent.extension = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        parent.hash = Some(NodeHash::default());
        parent.value = Some(vec![42]);
        parent.dirty = true;

        let mut child = Cell::new();
        child.fill_from_parent(&parent, 2);

        // Extension should be sliced
        assert_eq!(child.extension, Nibbles::from_hex(vec![3, 4, 5]));
        // Hash should be copied
        assert!(child.hash.is_some());
        // Value should not be copied (extension not fully consumed)
        assert!(child.value.is_none());
        // Dirty state should be inherited
        assert!(child.dirty);
    }
}
