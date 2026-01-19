//! Grid-based Patricia trie for efficient state root computation.
//!
//! This implements Erigon's grid-based trie pipelining algorithm.
//! Instead of recursive tree traversal, it uses an iterative grid state machine
//! that processes keys in sorted order with fold/unfold operations.

use ethereum_types::H256;

use crate::{
    db::TrieDB,
    error::TrieError,
    nibbles::Nibbles,
    node_hash::NodeHash,
    EMPTY_TRIE_HASH,
};

use super::{
    bitmap::{AfterMap, TouchMap},
    cell::{Cell, ACCOUNT_KEY_NIBBLES, MAX_DEPTH, NIBBLE_COUNT},
};

/// Grid-based Patricia trie for computing state roots efficiently.
///
/// The grid is organized as 128 rows (depth levels) x 16 columns (nibble values).
/// Keys are processed in sorted order, using fold/unfold operations to navigate
/// the tree structure without recursion.
pub struct HexPatriciaGrid<DB: TrieDB> {
    /// Grid cells: cells[row][column] where row is depth, column is nibble
    cells: Box<[[Cell; NIBBLE_COUNT]; MAX_DEPTH]>,

    /// Root cell (depth 0, before first nibble)
    root: Cell,

    /// TouchMap per row - tracks which columns were accessed
    touch_maps: [TouchMap; MAX_DEPTH],

    /// AfterMap per row - tracks which columns have children after updates
    after_maps: [AfterMap; MAX_DEPTH],

    /// Whether a branch existed at each depth before modification
    branch_before: [bool; MAX_DEPTH],

    /// Current depth in the grid (active row count)
    current_depth: u8,

    /// Current key path (nibbles from root to current position)
    current_key: [u8; MAX_DEPTH],

    /// Length of current key path
    current_key_len: u8,

    /// Depth values at each active row
    depths: [u8; MAX_DEPTH],

    /// Tracks which parent column each row's data belongs to.
    /// This allows detecting when we switch to a different subtree and need to reset.
    /// None means the row has no data yet or was fully consumed.
    row_parent_col: [Option<u8>; MAX_DEPTH],

    /// Database for loading/storing trie nodes
    db: DB,

    /// Root hash computed after all updates
    root_hash: Option<H256>,

    /// Whether root was touched during processing
    root_touched: bool,

    /// Whether root is present (non-empty) after processing
    root_present: bool,
}

impl<DB: TrieDB> HexPatriciaGrid<DB> {
    /// Create a new empty grid with the given database backend.
    pub fn new(db: DB) -> Self {
        Self {
            cells: Box::new(std::array::from_fn(|_| {
                std::array::from_fn(|_| Cell::new())
            })),
            root: Cell::new(),
            touch_maps: [TouchMap::new(); MAX_DEPTH],
            after_maps: [AfterMap::new(); MAX_DEPTH],
            branch_before: [false; MAX_DEPTH],
            current_depth: 0,
            current_key: [0u8; MAX_DEPTH],
            current_key_len: 0,
            depths: [0u8; MAX_DEPTH],
            row_parent_col: [None; MAX_DEPTH],
            db,
            root_hash: None,
            root_touched: false,
            root_present: false,
        }
    }

    /// Reset the grid to its initial empty state.
    pub fn reset(&mut self) {
        self.root.reset();
        self.current_depth = 0;
        self.current_key_len = 0;
        self.root_hash = None;
        self.root_touched = false;
        self.root_present = false;

        for i in 0..MAX_DEPTH {
            self.touch_maps[i].reset();
            self.after_maps[i].reset();
            self.branch_before[i] = false;
            self.depths[i] = 0;
            self.row_parent_col[i] = None;
            for j in 0..NIBBLE_COUNT {
                self.cells[i][j].reset();
            }
        }
    }

    /// Get reference to the database.
    pub fn db(&self) -> &DB {
        &self.db
    }

    /// Apply sorted updates and compute the new root hash.
    ///
    /// # CRITICAL: Keys must be sorted in ascending order!
    ///
    /// The algorithm processes keys by:
    /// 1. Folding back to common prefix when keys diverge
    /// 2. Unfolding to reach target depth
    /// 3. Applying the update at the leaf position
    ///
    /// # Arguments
    /// * `updates` - Iterator of (hashed_key, value) pairs in sorted order.
    ///               Empty value means deletion.
    ///
    /// # Returns
    /// The computed state root hash.
    pub fn apply_sorted_updates<I>(&mut self, updates: I) -> Result<H256, TrieError>
    where
        I: Iterator<Item = (H256, Vec<u8>)>,
    {
        let mut prev_key: Option<H256> = None;
        let debug = std::env::var("GRID_DEBUG").is_ok();

        for (key, value) in updates {
            // Verify sorted order in debug builds
            #[cfg(debug_assertions)]
            if let Some(ref pk) = prev_key {
                debug_assert!(pk < &key, "Keys must be sorted! {:?} >= {:?}", pk, key);
            }

            // Convert key to nibbles
            let key_nibbles = Self::key_to_nibbles(&key);

            if debug {
                eprintln!("\n=== Processing key {:?} ===", &key_nibbles[..4]);
                eprintln!("  current_depth={}, prev_key={:?}", self.current_depth,
                    prev_key.map(|pk| Self::key_to_nibbles(&pk)[..4].to_vec()));
            }

            // Calculate common prefix length with previous key
            let common_prefix_len = if let Some(ref pk) = prev_key {
                Self::common_prefix_length(&Self::key_to_nibbles(pk), &key_nibbles)
            } else {
                0
            };

            if debug {
                eprintln!("  common_prefix_len={}", common_prefix_len);
            }

            // Phase 1: FOLD - reduce depth to common prefix level
            while self.current_depth > common_prefix_len as u8 {
                if debug {
                    eprintln!("  FOLD from depth {}", self.current_depth);
                }
                self.fold()?;
            }

            // Phase 2: UNFOLD - expand to key's target depth
            let target_depth = ACCOUNT_KEY_NIBBLES as u8; // 64 nibbles for H256

            while self.current_depth < target_depth {
                let unfolding = self.need_unfolding(&key_nibbles);
                if unfolding == 0 {
                    break;
                }
                if debug {
                    eprintln!("  UNFOLD from depth {} (unfolding={})", self.current_depth, unfolding);
                }
                self.unfold(&key_nibbles, unfolding)?;
            }

            // Phase 3: UPDATE - apply the value change
            if value.is_empty() {
                self.delete_at_key(&key_nibbles)?;
            } else {
                if debug {
                    eprintln!("  UPDATE at depth {}", self.current_depth);
                }
                self.update_at_key(&key_nibbles, value)?;
            }

            prev_key = Some(key);
        }

        // Final fold back to root
        if debug {
            eprintln!("\n=== Final fold to root ===");
        }
        while self.current_depth > 0 {
            if debug {
                eprintln!("  FOLD from depth {}", self.current_depth);
            }
            self.fold()?;
        }

        // Compute and return root hash
        let root = self.compute_root_hash();
        if debug {
            eprintln!("  Root hash: {:?}", root);
        }
        Ok(root)
    }

    /// Convert H256 key to nibbles array.
    fn key_to_nibbles(key: &H256) -> [u8; ACCOUNT_KEY_NIBBLES] {
        let mut nibbles = [0u8; ACCOUNT_KEY_NIBBLES];
        for (i, byte) in key.as_bytes().iter().enumerate() {
            nibbles[i * 2] = byte >> 4;
            nibbles[i * 2 + 1] = byte & 0x0f;
        }
        nibbles
    }

    /// Calculate common prefix length between two nibble arrays.
    fn common_prefix_length(a: &[u8], b: &[u8]) -> usize {
        a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
    }

    /// Update the cell at the given key with a new value.
    fn update_at_key(&mut self, key_nibbles: &[u8], value: Vec<u8>) -> Result<(), TrieError> {
        if self.current_depth == 0 {
            // Updating root - this means we're inserting into an empty trie
            // or updating the only entry. The extension should be the FULL key + leaf terminator.
            self.root_touched = true;
            self.root_present = true;
            self.root.set_value(value);
            // Set extension to the full key path with leaf terminator (16)
            let mut ext = key_nibbles.to_vec();
            ext.push(16); // Leaf terminator
            self.root.extension = Nibbles::from_hex(ext);
        } else {
            let row = (self.current_depth - 1) as usize;
            let col = key_nibbles[self.current_depth as usize - 1] as usize;

            self.touch_maps[row].set(col as u8);
            self.after_maps[row].set(col as u8);

            // Set value and extension (remaining nibbles from current position + leaf terminator)
            self.cells[row][col].set_value(value);
            // Extension is the remaining nibbles after current depth with leaf terminator
            let mut remaining = key_nibbles[self.current_depth as usize..].to_vec();
            remaining.push(16); // Leaf terminator
            self.cells[row][col].extension = Nibbles::from_hex(remaining);
        }
        Ok(())
    }

    /// Delete the cell at the given key.
    fn delete_at_key(&mut self, key_nibbles: &[u8]) -> Result<(), TrieError> {
        if self.current_depth == 0 {
            // Deleting root
            self.root_touched = true;
            self.root_present = false;
            self.root.mark_deleted();
        } else {
            let row = (self.current_depth - 1) as usize;
            let col = key_nibbles[self.current_depth as usize - 1] as usize;

            self.touch_maps[row].set(col as u8);
            self.after_maps[row].clear(col as u8);
            self.cells[row][col].mark_deleted();
        }
        Ok(())
    }

    /// Compute the final root hash.
    fn compute_root_hash(&self) -> H256 {
        use crate::node::{ExtensionNode, LeafNode};

        if !self.root_present {
            return *EMPTY_TRIE_HASH;
        }

        // If root has value, compute leaf hash
        if let Some(ref value) = self.root.value {
            let leaf = LeafNode::new(self.root.extension.clone(), value.clone());
            return leaf.compute_hash().finalize();
        }

        // If root has hash, optionally wrap in extension
        if let Some(hash) = self.root.hash {
            if self.root.extension.is_empty() {
                return hash.finalize();
            } else {
                let child_ref = crate::NodeRef::Hash(hash);
                let ext = ExtensionNode::new(self.root.extension.clone(), child_ref);
                return ext.compute_hash().finalize();
            }
        }

        *EMPTY_TRIE_HASH
    }

    /// Get the hash of the subtrie at a specific nibble position.
    ///
    /// This is used by ConcurrentPatriciaGrid to extract the child hash
    /// after processing a partition where all keys share the same first nibble.
    ///
    /// Returns Some(hash) if there's a valid subtrie at that position, None otherwise.
    pub fn get_child_hash_at_nibble(&self, nibble: u8) -> Result<Option<H256>, TrieError> {
        use crate::node::{ExtensionNode, LeafNode};

        if !self.root_present {
            return Ok(None);
        }

        // Check if root has an extension that starts with the expected nibble
        if !self.root.extension.is_empty() {
            let first_nibble = self.root.extension.as_ref()[0];
            if first_nibble != nibble {
                // Extension doesn't start with expected nibble - shouldn't happen in concurrent mode
                return Ok(None);
            }

            // Remove the first nibble from extension to get the remaining path
            let remaining_ext = if self.root.extension.len() > 1 {
                self.root.extension.slice(1, self.root.extension.len())
            } else {
                Nibbles::default()
            };

            // Compute the hash of the subtrie (value or hash with remaining extension)
            if let Some(ref value) = self.root.value {
                // It's a leaf - remaining extension + leaf terminator + value
                let leaf = LeafNode::new(remaining_ext, value.clone());
                return Ok(Some(leaf.compute_hash().finalize()));
            }

            if let Some(hash) = self.root.hash {
                if remaining_ext.is_empty() {
                    // Just the hash
                    return Ok(Some(hash.finalize()));
                } else {
                    // Extension node with remaining path
                    let child_ref = crate::NodeRef::Hash(hash);
                    let ext = ExtensionNode::new(remaining_ext, child_ref);
                    return Ok(Some(ext.compute_hash().finalize()));
                }
            }
        }

        // Root is a branch node (hash without extension)
        // This shouldn't happen when all keys share the same first nibble
        if let Some(hash) = self.root.hash {
            return Ok(Some(hash.finalize()));
        }

        Ok(None)
    }

    /// Check how many nibbles we need to unfold to reach the key.
    /// Returns 0 if no unfolding needed.
    fn need_unfolding(&self, key_nibbles: &[u8]) -> u8 {
        let cell = if self.current_depth == 0 {
            &self.root
        } else {
            let row = (self.current_depth - 1) as usize;
            let col = key_nibbles[self.current_depth as usize - 1] as usize;
            &self.cells[row][col]
        };

        // If cell is empty, no unfolding needed
        if cell.is_empty() && !cell.has_hash() {
            return 0;
        }

        // If cell has no extension, unfold one level
        if cell.extension.is_empty() {
            if cell.has_hash() {
                return 1;
            }
            return 0;
        }

        // Calculate how much of the extension matches the key
        let depth = self.current_depth as usize;
        let ext_len = cell.extension.len();

        // Find common prefix between extension and remaining key
        let mut common = 0;
        for i in 0..ext_len {
            if depth + i >= key_nibbles.len() {
                break;
            }
            let ext_nibble = cell.extension.as_ref().get(i).copied().unwrap_or(0);
            if ext_nibble != key_nibbles[depth + i] {
                break;
            }
            common += 1;
        }

        // Unfold at least up to divergence point + 1
        (common + 1).min(ext_len) as u8
    }

    /// Fold operation: reduce depth by computing hashes and propagating upward.
    pub(crate) fn fold(&mut self) -> Result<(), TrieError> {
        if self.current_depth == 0 {
            return Ok(());
        }

        let row = (self.current_depth - 1) as usize;
        let debug = std::env::var("GRID_DEBUG").is_ok();

        // Determine what kind of fold based on after_map
        let after_count = self.after_maps[row].count();

        if debug {
            eprintln!("    fold row={} after_count={} after_bits={:016b}",
                row, after_count, self.after_maps[row].0);
        }

        match after_count {
            0 => {
                // Empty row - propagate deletion upward
                if debug { eprintln!("    -> fold_delete"); }
                self.fold_delete(row)?;
            }
            1 => {
                // Single child - create extension/leaf and propagate
                let child_col = self.after_maps[row].first_set().unwrap();
                if debug {
                    let cell = &self.cells[row][child_col as usize];
                    eprintln!("    -> fold_propagate col={} has_value={} has_hash={} ext_len={}",
                        child_col, cell.value.is_some(), cell.hash.is_some(), cell.extension.len());
                }
                self.fold_propagate(row)?;
            }
            _ => {
                // Multiple children - create branch node
                if debug { eprintln!("    -> fold_branch"); }
                self.fold_branch(row)?;
            }
        }

        // Mark all rows below as needing reset, since we're folding past them
        // This ensures stale data from earlier processing doesn't persist
        for r in (row + 1)..MAX_DEPTH {
            self.row_parent_col[r] = None;
        }

        self.current_depth -= 1;
        if self.current_key_len > 0 {
            self.current_key_len -= 1;
        }

        Ok(())
    }

    /// Handle fold when row is empty (deletion case).
    fn fold_delete(&mut self, row: usize) -> Result<(), TrieError> {
        if self.touch_maps[row].count() > 0 {
            // Propagate deletion to parent
            if row == 0 {
                self.root_touched = true;
                self.root_present = false;
            } else {
                let parent_col = self.current_key[row - 1] as usize;
                self.touch_maps[row - 1].set(parent_col as u8);
                self.after_maps[row - 1].clear(parent_col as u8);
            }
        }
        Ok(())
    }

    /// Handle fold when single child remains (propagate/extension case).
    ///
    /// When there's only one child at a given level, we don't create a branch.
    /// Instead, we accumulate the path nibbles and propagate up.
    /// - If child has value: keep value and extend the path
    /// - If child has hash: keep hash and extend the path
    /// Only when reaching the root do we finalize the node structure.
    fn fold_propagate(&mut self, row: usize) -> Result<(), TrieError> {
        let child_col = self.after_maps[row].first_set().unwrap() as usize;

        // Clone child data to avoid borrow conflicts
        let child_hash = self.cells[row][child_col].hash;
        let child_extension = self.cells[row][child_col].extension.clone();
        let child_value = self.cells[row][child_col].value.clone();

        // Build the accumulated path: current nibble + any existing extension
        let mut accumulated_path = Nibbles::from_hex(vec![child_col as u8]);
        if !child_extension.is_empty() {
            accumulated_path = accumulated_path.concat(&child_extension);
        }

        // Propagate up: keep the value OR hash, and accumulate the path
        // Don't create final node structure yet - that happens at root
        if row == 0 {
            self.root_touched = true;
            self.root_present = child_value.is_some() || child_hash.is_some();
            self.root.hash = child_hash;
            self.root.extension = accumulated_path;
            self.root.value = child_value;
        } else {
            let parent_col = self.current_key[row - 1] as usize;
            self.touch_maps[row - 1].set(parent_col as u8);
            self.after_maps[row - 1].set(parent_col as u8);

            let parent = &mut self.cells[row - 1][parent_col];
            parent.hash = child_hash;
            parent.extension = accumulated_path;
            parent.value = child_value;
            parent.dirty = true;
        }

        Ok(())
    }

    /// Handle fold when multiple children remain (branch node case).
    fn fold_branch(&mut self, row: usize) -> Result<(), TrieError> {
        // Compute branch hash from children
        let branch_hash = self.compute_branch_hash(row)?;

        if row == 0 {
            self.root_touched = true;
            self.root_present = true;
            self.root.hash = Some(branch_hash);
            self.root.extension = Nibbles::default();
            self.root.value = None; // Clear value - this is now a branch, not a leaf
        } else {
            let parent_col = self.current_key[row - 1] as usize;
            self.touch_maps[row - 1].set(parent_col as u8);
            self.after_maps[row - 1].set(parent_col as u8);

            let parent = &mut self.cells[row - 1][parent_col];
            parent.hash = Some(branch_hash);
            parent.extension = Nibbles::default();
            parent.value = None; // Clear value - this is now a branch, not a leaf
            parent.dirty = true;
        }

        Ok(())
    }

    /// Compute the hash for a branch node at the given row.
    fn compute_branch_hash(&self, row: usize) -> Result<NodeHash, TrieError> {
        use crate::node::{BranchNode, ExtensionNode, LeafNode};
        use crate::NodeRef;

        let mut choices = BranchNode::EMPTY_CHOICES;

        for col in 0..NIBBLE_COUNT {
            if self.after_maps[row].is_set(col as u8) {
                let cell = &self.cells[row][col];

                // Compute the hash for this child cell
                let child_hash = if let Some(ref value) = cell.value {
                    // Cell has a value - it's a leaf node
                    let leaf = LeafNode::new(cell.extension.clone(), value.clone());
                    leaf.compute_hash()
                } else if let Some(hash) = cell.hash {
                    // Cell has an existing hash
                    if cell.extension.is_empty() {
                        hash
                    } else {
                        // Has extension - wrap in extension node
                        let child_ref = NodeRef::Hash(hash);
                        let ext = ExtensionNode::new(cell.extension.clone(), child_ref);
                        ext.compute_hash()
                    }
                } else {
                    continue; // No valid data for this cell
                };

                choices[col] = NodeRef::Hash(child_hash);
            }
        }

        let branch = BranchNode::new(choices);
        Ok(branch.compute_hash())
    }

    /// Unfold operation: expand depth by loading from DB or deriving from parent.
    pub(crate) fn unfold(&mut self, key_nibbles: &[u8], _unfolding: u8) -> Result<(), TrieError> {
        let row = self.current_depth as usize;

        // Determine the parent column for this row.
        // For row 0, we use a sentinel value (255 means "root").
        // For row > 0, it's the nibble we followed to get here.
        let parent_col = if row == 0 {
            255u8 // Sentinel for root
        } else {
            key_nibbles[row - 1]
        };

        // Reset this row if we're switching to a different parent column.
        // This means we're in a different subtree and need fresh data.
        let needs_reset = self.row_parent_col[row] != Some(parent_col);

        if needs_reset {
            self.touch_maps[row].reset();
            self.after_maps[row].reset();
            self.branch_before[row] = false;
            self.row_parent_col[row] = Some(parent_col);
            for col in 0..NIBBLE_COUNT {
                self.cells[row][col].reset();
            }
        }

        // Extract parent cell info to avoid borrow conflicts
        let (parent_has_extension, parent_has_hash, parent_data) = if self.current_depth == 0 {
            let has_ext = !self.root.extension.is_empty();
            let has_hash = self.root.has_hash();
            let data = if has_ext {
                Some((
                    self.root.extension.clone(),
                    self.root.hash,
                    self.root.value.clone(),
                    self.root.dirty,
                    self.root.deleted,
                ))
            } else {
                None
            };
            (has_ext, has_hash, data)
        } else {
            let parent_row = (self.current_depth - 1) as usize;
            let parent_col = key_nibbles[self.current_depth as usize - 1] as usize;
            let parent = &self.cells[parent_row][parent_col];
            let has_ext = !parent.extension.is_empty();
            let has_hash = parent.has_hash();
            let data = if has_ext {
                Some((
                    parent.extension.clone(),
                    parent.hash,
                    parent.value.clone(),
                    parent.dirty,
                    parent.deleted,
                ))
            } else {
                None
            };
            (has_ext, has_hash, data)
        };

        // Determine target nibble from key
        let target_nibble = key_nibbles[self.current_depth as usize] as usize;

        // If parent has a hash but no extension, load branch from DB
        if !parent_has_extension && parent_has_hash {
            self.unfold_branch(row)?;
        } else if let Some((ext, hash, value, dirty, deleted)) = parent_data {
            // Parent has extension - we need to split it
            // The first nibble of the extension tells us which child gets the existing data

            if ext.is_empty() {
                // No extension, just propagate to target
                self.after_maps[row].set(target_nibble as u8);
            } else {
                let parent_first_nibble = ext.as_ref()[0] as usize;

                if parent_first_nibble == target_nibble {
                    // Keys share the same path - continue down
                    let child = &mut self.cells[row][target_nibble];

                    // Consume first nibble of extension
                    if ext.len() > 1 {
                        child.extension = ext.slice(1, ext.len());
                    } else {
                        child.extension = Nibbles::default();
                    }

                    child.hash = hash;
                    // Value stays with its extension - copy if extension is now empty
                    if ext.len() <= 1 {
                        child.value = value;
                    } else {
                        // Value stays attached to the remaining extension
                        child.value = value;
                    }
                    child.dirty = dirty;
                    child.deleted = deleted;

                    self.after_maps[row].set(target_nibble as u8);
                } else {
                    // Keys diverge! Put existing data at parent_first_nibble,
                    // target_nibble will be empty for new insertion
                    let existing_child = &mut self.cells[row][parent_first_nibble];

                    // Existing data goes here (minus first nibble consumed)
                    if ext.len() > 1 {
                        existing_child.extension = ext.slice(1, ext.len());
                    } else {
                        existing_child.extension = Nibbles::default();
                    }
                    existing_child.hash = hash;
                    existing_child.value = value;
                    existing_child.dirty = dirty;
                    existing_child.deleted = deleted;

                    self.after_maps[row].set(parent_first_nibble as u8);
                    // Note: target_nibble cell stays empty - will be filled by update_at_key
                }
            }
        }

        // Update current key
        self.current_key[self.current_depth as usize] = target_nibble as u8;
        self.current_key_len = self.current_depth + 1;
        self.depths[row] = self.current_depth + 1;
        self.current_depth += 1;

        Ok(())
    }

    /// Load branch node children from database.
    fn unfold_branch(&mut self, row: usize) -> Result<(), TrieError> {
        // Build path for DB lookup
        let path = Nibbles::from_hex(self.current_key[..self.current_key_len as usize].to_vec());

        // Try to load from DB
        if let Some(encoded) = self.db.get(path)? {
            use crate::Node;
            use ethrex_rlp::decode::RLPDecode;

            let node = Node::decode(&encoded).map_err(TrieError::RLPDecode)?;

            match node {
                Node::Branch(branch) => {
                    self.branch_before[row] = true;
                    for (col, choice) in branch.choices.iter().enumerate() {
                        if choice.is_valid() {
                            self.cells[row][col].hash = Some(choice.compute_hash());
                            self.after_maps[row].set(col as u8);
                        }
                    }
                }
                Node::Extension(ext) => {
                    let first_nibble = ext.prefix.as_ref().first().copied().unwrap_or(0) as usize;
                    self.cells[row][first_nibble].hash = Some(ext.child.compute_hash());
                    self.cells[row][first_nibble].extension = ext.prefix.slice(1, ext.prefix.len());
                    self.after_maps[row].set(first_nibble as u8);
                }
                Node::Leaf(leaf) => {
                    let first_nibble = leaf.partial.as_ref().first().copied().unwrap_or(0) as usize;
                    self.cells[row][first_nibble].value = Some(leaf.value);
                    self.cells[row][first_nibble].extension =
                        leaf.partial.slice(1, leaf.partial.len());
                    self.after_maps[row].set(first_nibble as u8);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::InMemoryTrieDB;
    use std::sync::{Arc, Mutex};
    use std::collections::BTreeMap;

    fn create_test_grid() -> HexPatriciaGrid<InMemoryTrieDB> {
        let db = InMemoryTrieDB::new(Arc::new(Mutex::new(BTreeMap::new())));
        HexPatriciaGrid::new(db)
    }

    #[test]
    fn test_grid_new() {
        let grid = create_test_grid();
        assert_eq!(grid.current_depth, 0);
        assert!(!grid.root_touched);
        assert!(!grid.root_present);
    }

    #[test]
    fn test_grid_reset() {
        let mut grid = create_test_grid();
        grid.current_depth = 5;
        grid.root_touched = true;

        grid.reset();

        assert_eq!(grid.current_depth, 0);
        assert!(!grid.root_touched);
    }

    #[test]
    fn test_key_to_nibbles() {
        let key = H256::from_low_u64_be(0x1234);
        let nibbles = HexPatriciaGrid::<InMemoryTrieDB>::key_to_nibbles(&key);

        // 0x1234 is at the end of the 32-byte H256
        // Last 2 bytes are 0x12, 0x34
        // Which become nibbles: 1, 2, 3, 4 at positions 60, 61, 62, 63
        assert_eq!(nibbles[60], 1);
        assert_eq!(nibbles[61], 2);
        assert_eq!(nibbles[62], 3);
        assert_eq!(nibbles[63], 4);
    }

    #[test]
    fn test_common_prefix_length() {
        let a = [1, 2, 3, 4, 5];
        let b = [1, 2, 3, 9, 9];

        let len = HexPatriciaGrid::<InMemoryTrieDB>::common_prefix_length(&a, &b);
        assert_eq!(len, 3);
    }

    #[test]
    fn test_empty_trie_hash() {
        let mut grid = create_test_grid();
        let hash = grid.apply_sorted_updates(std::iter::empty()).unwrap();
        assert_eq!(hash, *EMPTY_TRIE_HASH);
    }
}
