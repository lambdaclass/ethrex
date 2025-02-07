#[cfg(feature = "libmdbx")]
pub mod libmdbx {
    use std::{path::PathBuf, sync::Arc};

    use libmdbx::{
        orm::{table_info, Database, Table},
        table,
    };

    table!(
        /// Test table.
        (TestNodes) Vec<u8> => Vec<u8>
    );

    /// Creates a new DB on a given path
    pub fn new_db_with_path<T: Table>(path: PathBuf) -> Arc<Database> {
        let tables = [table_info!(T)].into_iter().collect();
        Arc::new(Database::create(Some(path), &tables).expect("Failed creating db with path"))
    }

    /// Creates a new temporary DB
    pub fn new_db<T: Table>() -> Arc<Database> {
        let tables = [table_info!(T)].into_iter().collect();
        Arc::new(Database::create(None, &tables).expect("Failed to create temp DB"))
    }

    /// Opens a DB from a given path
    pub fn open_db<T: Table>(path: &str) -> Arc<Database> {
        let tables = [table_info!(T)].into_iter().collect();
        Arc::new(Database::open(path, &tables).expect("Failed to open DB"))
    }
}

#[macro_export]
/// Creates a trie node
/// All partial paths are expressed in nibbles and values in bytes
macro_rules! pmt_node {
    (
        @( $trie:expr )
        branch { $( $choice:expr => $child_type:ident { $( $child_tokens:tt )* } ),+ $(,)? }
        $( offset $offset:expr )?
    ) => {
        $crate::node::BranchNode::new({
            #[allow(unused_variables)]
            let offset = true $( ^ $offset )?;
            let mut choices = $crate::node::BranchNode::EMPTY_CHOICES;
            $(
                let child_node: Node = pmt_node! { @($trie)
                    $child_type { $( $child_tokens )* }
                    offset offset
                }.into();
                choices[$choice as usize] = child_node.insert_self(&mut $trie.state).unwrap();
            )*
            Box::new(choices)
        })
    };
    (
        @( $trie:expr )
        branch { $( $choice:expr => $child_type:ident { $( $child_tokens:tt )* } ),+ $(,)? }
        with_leaf { $path:expr => $value:expr }
        $( offset $offset:expr )?
    ) => {{
        $crate::node::BranchNode::new_with_value({
            #[allow(unused_variables)]
            let offset = true $( ^ $offset )?;
            let mut choices = $crate::node::BranchNode::EMPTY_CHOICES;
            $(
                choices[$choice as usize] = $crate::node::Node::from(
                    pmt_node! { @($trie)
                        $child_type { $( $child_tokens )* }
                        offset offset
                    }).insert_self(&mut $trie.state).unwrap();
            )*
            Box::new(choices)
        }, $value)
    }};

    (
        @( $trie:expr )
        extension { $prefix:expr , $child_type:ident { $( $child_tokens:tt )* } }
        $( offset $offset:expr )?
    ) => {{
        #[allow(unused_variables)]
        let prefix = $crate::nibbles::Nibbles::from_hex($prefix.to_vec());

        $crate::node::ExtensionNode::new(
            prefix.clone(),
            {
                let child_node = $crate::node::Node::from(pmt_node! { @($trie)
                    $child_type { $( $child_tokens )* }
                });
                child_node.insert_self(&mut $trie.state).unwrap()
            }
        )
    }};

    (
        @( $trie:expr)
        leaf { $path:expr => $value:expr }
        $( offset $offset:expr )?
    ) => {
        {
            $crate::node::LeafNode::new(Nibbles::from_hex($path), $value)
        }
    };
}
