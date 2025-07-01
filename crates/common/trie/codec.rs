use crate::nibbles::Nibbles;
use crate::node::{ExtensionNode, LeafNode};
use crate::{Node, NodeHash};
use ethrex_common::H256;

pub fn minimal_encode(node: &Node, buffer: &mut Vec<u8>) {
    buffer.clear();

    match node {
        Node::Branch(branch) => {
            buffer.push(0x00);

            let mut buf_occupied = 0x0000_u16;
            let mut buf_hashed = 0x0000_u16;

            buffer.resize(5, 0); // Placeholder for occupied and hashed bits

            for (i, choice) in branch
                .choices
                .iter()
                .enumerate()
                .filter(|(_, choice)| choice.is_valid())
            {
                buf_occupied |= 1 << i;
                match choice.compute_hash() {
                    NodeHash::Hashed(hash) => {
                        buf_hashed |= 1 << i;
                        buffer.extend_from_slice(&hash.0);
                    }
                    NodeHash::Inline(value) => {
                        buffer.push(value.1);
                        buffer.extend_from_slice(&value.0[0..value.1 as usize]);
                    }
                }
            }

            buffer[1] = (buf_occupied >> 8) as u8;
            buffer[2] = (buf_occupied & 0xFF) as u8;
            buffer[3] = (buf_hashed >> 8) as u8;
            buffer[4] = (buf_hashed & 0xFF) as u8
        }
        Node::Extension(extension) => {
            buffer.push(0x01);

            // push the hash of the child node
            let child_hash = extension.child.compute_hash();
            match child_hash {
                NodeHash::Hashed(hash) => {
                    buffer.extend_from_slice(&hash.0);
                }
                NodeHash::Inline(_value) => {
                    unreachable!("Extension nodes should not have inline children");
                }
            }
            // push the prefix key in compact form
            extension.prefix.encode_compact_to_vec(buffer);
        }
        Node::Leaf(leaf) => {
            buffer.push(0x02);

            // ValueRLP is inserted with the first byte as the length
            let value_length = leaf.value.len() as u8;
            buffer.push(value_length);
            buffer.extend_from_slice(&leaf.value.as_ref());
            leaf.partial.encode_compact_to_vec(buffer);
        }
    }
}

pub fn minimal_decode(input: &[u8]) -> anyhow::Result<Node> {
    let first = input
        .first()
        .ok_or_else(|| anyhow::anyhow!("Input is empty"))?;
    match first {
        0x00 => {
            // @@@@@@@@@@@@@@@@@@
            todo!("Branch node decoding not implemented yet");
        }
        0x01 => {
            // Extension node
            if input.len() < 3 {
                anyhow::bail!(anyhow::anyhow!("Input too short for Extension node"));
            }

            // decode the child of the child node
            if input.len() < 34 {
                anyhow::bail!(anyhow::anyhow!(
                    "Input too short for Extension node child hash"
                ));
            }
            let hash = &input[1..33];
            let child_hash = NodeHash::Hashed(hash);

            let prefix = Nibbles::decode_compact(&input[33..])?;
            return Ok(Node::Extension(ExtensionNode {
                prefix,
                child: NodeRef::Hash(child_hash),
            }));
        }
        0x02 => {
            // Leaf node
            if input.len() < 3 {
                anyhow::bail!(anyhow::anyhow!("Input too short for Leaf node"));
            }
            let value_rlp_length = input[1];
            let value_rlp = input[2..(2 + value_rlp_length as usize)].to_vec();
            let partial = input[(2 + value_rlp_length as usize)..].to_vec();

            return Ok(Node::Leaf(LeafNode {
                partial: Nibbles::decode_compact(&partial),
                value: value_rlp,
            }));
        }
        _ => {
            anyhow::bail!(anyhow::anyhow!("Invalid node type: {}", first));
        }
    }
    todo!()
}
