use crate::{Node, NodeHash};

pub fn minimal_encode(node: &Node, buffer: &mut Vec<u8>) {
    buffer.clear();

    match node {
        Node::Branch(branch) => {
            buffer.push(0x00);

            let mut buf_occupied = 0x0000_u16;
            let mut buf_hashed = 0x0000_u16;

            buffer.resize(5, 0); // Placeholder for occupied and hashed bits

            for (i, choice) in branch.choices.iter().enumerate() {
                if choice.is_valid() {
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
        0x00 => {}
        0x01 => {}
        0x02 => {}
        _ => {
            anyhow::bail!(anyhow::anyhow!("Invalid node type: {}", first));
        }
    }
    todo!()
}
