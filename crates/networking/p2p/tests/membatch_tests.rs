//! Tests for membatch commit_node behavior
//!
//! These tests document the current brittle behavior of the recursive commit_node
//! implementation in state_healing.rs and storage_healing.rs.
//!
//! Issues tested:
//! - Panic on missing parent (line 396-398 in state_healing.rs)
//! - Count underflow (line 400 in state_healing.rs)
//! - Stack overflow on deep tries (recursive call line 402-408)
//! - Memory leak with orphaned nodes (membatch grows unbounded)
//!
//! These will be fixed in Phase 2 by replacing recursive implementation with
//! iterative queue-based version.

use std::collections::HashMap;
use ethrex_trie::{Node, Nibbles, node::{ExtensionNode, NodeRef}};
use ethrex_common::H256;

// Mirror the MembatchEntryValue structure from state_healing.rs
#[derive(Debug)]
struct MembatchEntryValue {
    node: Node,
    children_not_in_storage_count: u64,
    parent_path: Nibbles,
}

// Copy of the recursive commit_node implementation to test
fn commit_node(
    node: Node,
    path: &Nibbles,
    parent_path: &Nibbles,
    membatch: &mut HashMap<Nibbles, MembatchEntryValue>,
    nodes_to_write: &mut Vec<(Nibbles, Node)>,
) {
    nodes_to_write.push((path.clone(), node));

    if parent_path == path {
        return; // Case where we're saving the root
    }

    let mut membatch_entry = membatch.remove(parent_path).unwrap_or_else(|| {
        panic!("The parent should exist. Parent: {parent_path:?}, path: {path:?}")
    });

    membatch_entry.children_not_in_storage_count -= 1;
    if membatch_entry.children_not_in_storage_count == 0 {
        commit_node(
            membatch_entry.node,
            parent_path,
            &membatch_entry.parent_path,
            membatch,
            nodes_to_write,
        );
    } else {
        membatch.insert(parent_path.clone(), membatch_entry);
    }
}

#[test]
#[should_panic(expected = "The parent should exist")]
fn test_commit_node_missing_parent_panics() {
    // Test 1: Document panic behavior when parent is missing
    //
    // Setup: Create a node with a parent path that doesn't exist in membatch
    // Expected: Panic with "The parent should exist" message
    //
    // This demonstrates the brittle behavior that will be fixed in Phase 2
    // by returning Result<(), CommitError::MissingParent> instead of panicking

    let mut membatch = HashMap::new();
    let mut nodes_to_write = Vec::new();

    // Create a child node with parent path [1, 2, 3]
    let parent_path = Nibbles::from_bytes(&[0x12, 0x30]);
    let child_path = Nibbles::from_bytes(&[0x12, 0x34]);
    let child_node = Node::Extension(ExtensionNode::new(
        Nibbles::from_bytes(&[0x34]),
        NodeRef::Hash(H256::zero().into()), // Dummy hash
    ));

    // Intentionally DON'T add parent to membatch - this will cause panic
    // membatch.insert(parent_path.clone(), ...);  // <-- missing!

    // This should panic
    commit_node(
        child_node,
        &child_path,
        &parent_path,
        &mut membatch,
        &mut nodes_to_write,
    );
}

#[test]
#[should_panic(expected = "attempt to subtract with overflow")]
fn test_commit_node_count_underflow() {
    // Test 2: Document count underflow behavior
    //
    // Setup: Manually create entry with children_not_in_storage_count = 0,
    //        then try to commit a child (which decrements count)
    // Expected: Panic on arithmetic underflow (u64 - 1 when u64 = 0)
    //
    // This demonstrates the missing checked arithmetic that will be fixed
    // in Phase 2 using checked_sub().ok_or(CommitError::CountUnderflow)

    let mut membatch = HashMap::new();
    let mut nodes_to_write = Vec::new();

    let parent_path = Nibbles::from_bytes(&[0x12, 0x30]);
    let child_path = Nibbles::from_bytes(&[0x12, 0x34]);

    let parent_node = Node::Extension(ExtensionNode::new(
        Nibbles::from_bytes(&[0x30]),
        NodeRef::Hash(H256::zero().into()),
    ));

    // Create parent with count already at 0
    membatch.insert(
        parent_path.clone(),
        MembatchEntryValue {
            node: parent_node,
            children_not_in_storage_count: 0, // Already 0!
            parent_path: parent_path.clone(), // Root
        },
    );

    let child_node = Node::Extension(ExtensionNode::new(
        Nibbles::from_bytes(&[0x34]),
        NodeRef::Hash(H256::zero().into()),
    ));

    // This should panic on underflow: 0 - 1
    commit_node(
        child_node,
        &child_path,
        &parent_path,
        &mut membatch,
        &mut nodes_to_write,
    );
}

#[test]
fn test_commit_node_orphaned_nodes_leak() {
    // Test 3: Document memory leak with orphaned nodes
    //
    // Setup: Create nodes where the parent never gets its count reduced to 0
    //        (simulates scenario where some children never arrive)
    // Expected: Membatch grows without bound as orphaned entries accumulate
    //
    // This demonstrates the lack of timeout/cleanup mechanism for stale entries.
    // Phase 2 won't directly fix this but the iterative version makes it easier
    // to add cleanup logic.

    let mut membatch = HashMap::new();
    let mut nodes_to_write = Vec::new();

    let parent_path = Nibbles::from_bytes(&[0x12]);
    let parent_node = Node::Extension(ExtensionNode::new(
        Nibbles::from_bytes(&[0x12]),
        NodeRef::Hash(H256::zero().into()),
    ));

    // Parent expects 10 children
    membatch.insert(
        parent_path.clone(),
        MembatchEntryValue {
            node: parent_node,
            children_not_in_storage_count: 10, // Expects 10 children
            parent_path: parent_path.clone(),   // Root
        },
    );

    // Only commit 3 children (simulating 7 missing/delayed children)
    for i in 0..3 {
        let child_path = Nibbles::from_bytes(&[0x12, 0x30 + i]);
        let child_node = Node::Extension(ExtensionNode::new(
            Nibbles::from_bytes(&[0x30 + i]),
            NodeRef::Hash(H256::zero().into()),
        ));

        commit_node(
            child_node,
            &child_path,
            &parent_path,
            &mut membatch,
            &mut nodes_to_write,
        );
    }

    // Verify parent is still in membatch (orphaned, waiting for 7 more children)
    assert_eq!(membatch.len(), 1);
    let entry = membatch.get(&parent_path).unwrap();
    assert_eq!(entry.children_not_in_storage_count, 7);

    // In a real scenario, if those 7 children never arrive, this entry
    // will remain in membatch indefinitely, causing a memory leak
    // The membatch could grow to gigabytes with enough orphaned entries
}

#[test]
#[ignore] // Disabled by default - causes stack overflow
fn test_commit_node_deep_tree_stack_overflow() {
    // Test 4: Document stack overflow on deep tree
    //
    // Setup: Create a chain of 10,000+ nodes where each is parent of the next
    //        When the leaf commits, it triggers 10,000 recursive calls
    // Expected: Stack overflow (default Rust stack is ~2MB per thread)
    //
    // This demonstrates why recursive implementation is dangerous.
    // Phase 2 will replace with iterative queue-based version that uses
    // heap memory instead of stack frames.
    //
    // NOTE: This test is #[ignore] because it crashes the test runner.
    // Run with: cargo test test_commit_node_deep_tree_stack_overflow -- --ignored

    let mut membatch = HashMap::new();
    let mut nodes_to_write = Vec::new();

    const DEPTH: usize = 10_000;

    // Build chain: root -> node1 -> node2 -> ... -> leaf
    // Each node has 1 child (count = 1)
    let root_path = Nibbles::from_bytes(&[0x00]);

    for depth in 0..DEPTH {
        let path = Nibbles::from_bytes(&vec![depth as u8]);
        let parent_path = if depth == 0 {
            root_path.clone()
        } else {
            Nibbles::from_bytes(&vec![(depth - 1) as u8])
        };

        let node = Node::Extension(ExtensionNode::new(
            Nibbles::from_bytes(&[depth as u8]),
            NodeRef::Hash(H256::from_low_u64_be(depth as u64).into()),
        ));

        membatch.insert(
            path.clone(),
            MembatchEntryValue {
                node,
                children_not_in_storage_count: 1, // Each expects 1 child
                parent_path,
            },
        );
    }

    // Now commit the leaf - this will trigger DEPTH recursive calls
    let leaf_path = Nibbles::from_bytes(&vec![DEPTH as u8]);
    let leaf_node = Node::Extension(ExtensionNode::new(
        Nibbles::from_bytes(&[DEPTH as u8]),
        NodeRef::Hash(H256::from_low_u64_be(DEPTH as u64).into()),
    ));

    let leaf_parent = Nibbles::from_bytes(&vec![(DEPTH - 1) as u8]);

    // This will cause stack overflow due to deep recursion
    // Each recursive call adds a stack frame, and with 10,000 frames
    // we exceed the default stack size (~2MB)
    commit_node(
        leaf_node,
        &leaf_path,
        &leaf_parent,
        &mut membatch,
        &mut nodes_to_write,
    );

    // If we reach here, the iterative fix has been applied
    // (or the stack size was increased, which is not the right fix)
    assert_eq!(nodes_to_write.len(), DEPTH + 1);
}
