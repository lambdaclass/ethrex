use bytes::Bytes;
use c_kzg::{Blob, BYTES_PER_BLOB};

pub fn blob_from_bytes(bytes: Bytes) -> Result<Blob, c_kzg::Error> {
    // We set the first byte of every 32-bytes chunk to 0x00
    // so it's always under the field module.
    if bytes.len() > BYTES_PER_BLOB * 31 / 32 {
        return Err(c_kzg::Error::InvalidBytesLength(format!(
            "Bytes too long for a Blob ({})",
            bytes.len()
        )));
    }

    let mut buf = [0u8; BYTES_PER_BLOB];
    buf[..(bytes.len() * 32).div_ceil(31)].copy_from_slice(
        &bytes
            .chunks(31)
            .map(|x| [&[0x00], x].concat())
            .collect::<Vec<_>>()
            .concat(),
    );

    Blob::from_bytes(&buf)
}
