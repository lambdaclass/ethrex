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
                choices[$choice as usize] = child_node.into();
            )*
            choices
        })
    };


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
                child_node.into()
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
