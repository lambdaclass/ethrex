use bytes::Bytes;
use ethereum_types::{Bloom, H160, H256, U256};
use rkyv::{
    Archive, Archived, Deserialize, Serialize,
    rancor::{Fallible, Source},
    ser::{Allocator, Writer},
    vec::{ArchivedVec, VecResolver},
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
};

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(remote = Vec<Vec<u8>>)]
pub struct VecVecWrapper {
    #[rkyv(getter = vec_vec_to_vec)]
    vec_vec: Vec<Vec<u8>>,
}

fn vec_vec_to_vec(vec_vec: &[Vec<u8>]) -> Vec<Vec<u8>> {
    vec_vec.iter().map(|b| b.to_vec()).collect()
}

impl From<VecVecWrapper> for Vec<Vec<u8>> {
    fn from(value: VecVecWrapper) -> Self {
        value.vec_vec
    }
}

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(remote = Vec<Bytes>)]
pub struct VecBytesWrapper {
    #[rkyv(getter = vec_bytes_to_vec)]
    vec_bytes: Vec<Vec<u8>>,
}

fn vec_bytes_to_vec(vec_bytes: &[Bytes]) -> Vec<Vec<u8>> {
    vec_bytes.iter().map(|b| b.to_vec()).collect()
}

impl From<VecBytesWrapper> for Vec<Bytes> {
    fn from(value: VecBytesWrapper) -> Self {
        value
            .vec_bytes
            .into_iter()
            .map(|b| Bytes::copy_from_slice(&b))
            .collect()
    }
}

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(remote = U256)]
pub struct U256Wrapper([u64; 4]);

impl From<U256Wrapper> for U256 {
    fn from(value: U256Wrapper) -> Self {
        Self(value.0)
    }
}

#[derive(Archive, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[rkyv(remote = H160)]
pub struct H160Wrapper([u8; 20]);

impl From<H160Wrapper> for H160 {
    fn from(value: H160Wrapper) -> Self {
        Self(value.0)
    }
}

impl PartialEq for ArchivedH160Wrapper {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for ArchivedH160Wrapper {}

impl Hash for ArchivedH160Wrapper {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

#[derive(
    Archive, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[rkyv(remote = H256)]
pub struct H256Wrapper([u8; 32]);

impl From<H256Wrapper> for H256 {
    fn from(value: H256Wrapper) -> Self {
        Self(value.0)
    }
}

impl PartialEq for ArchivedH256Wrapper {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl PartialOrd for ArchivedH256Wrapper {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ArchivedH256Wrapper {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl Eq for ArchivedH256Wrapper {}

impl Hash for ArchivedH256Wrapper {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(remote = Bytes)]
pub struct BytesWrapper {
    #[rkyv(getter = bytes_to_vec)]
    bytes: Vec<u8>,
}

fn bytes_to_vec(bytes: &Bytes) -> Vec<u8> {
    bytes.to_vec()
}

impl From<BytesWrapper> for Bytes {
    fn from(value: BytesWrapper) -> Self {
        Self::copy_from_slice(&value.bytes)
    }
}

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(remote = Bloom)]
pub struct BloomWrapper {
    #[rkyv(getter = bloom_to_bytes)]
    bloom_bytes: [u8; 256],
}

fn bloom_to_bytes(bloom: &Bloom) -> [u8; 256] {
    bloom.0
}

impl From<BloomWrapper> for Bloom {
    fn from(value: BloomWrapper) -> Self {
        Self::from_slice(&value.bloom_bytes)
    }
}

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(remote = Option<H256>)]
pub enum OptionH256Wrapper {
    Some(#[rkyv(with = H256Wrapper)] H256),
    None,
}

impl From<OptionH256Wrapper> for Option<H256> {
    fn from(value: OptionH256Wrapper) -> Self {
        if let OptionH256Wrapper::Some(x) = value {
            Some(x)
        } else {
            None
        }
    }
}

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(remote = Option<HashMap<H160, Vec<Vec<u8>>>>)]
pub enum OptionStorageWrapper {
    Some(
        #[rkyv(with = rkyv::with::MapKV<H160Wrapper, rkyv::with::AsBox>)]
        HashMap<H160, Vec<Vec<u8>>>,
    ),
    None,
}

impl From<OptionStorageWrapper> for Option<HashMap<H160, Vec<Vec<u8>>>> {
    fn from(value: OptionStorageWrapper) -> Self {
        if let OptionStorageWrapper::Some(x) = value {
            Some(x)
        } else {
            None
        }
    }
}
pub struct AccessListItemWrapper;

pub struct AccessListItemWrapperResolver {
    len: usize,
    inner: VecResolver,
}

impl ArchiveWith<(H160, Vec<H256>)> for AccessListItemWrapper {
    type Archived = ArchivedVec<u8>;
    type Resolver = AccessListItemWrapperResolver;
    fn resolve_with(
        _: &(H160, Vec<H256>),
        resolver: Self::Resolver,
        out: rkyv::Place<Self::Archived>,
    ) {
        ArchivedVec::resolve_from_len(resolver.len, resolver.inner, out);
    }
}

impl<S> SerializeWith<(H160, Vec<H256>), S> for AccessListItemWrapper
where
    S: Fallible + Allocator + Writer + ?Sized,
{
    fn serialize_with(
        field: &(H160, Vec<H256>),
        serializer: &mut S,
    ) -> Result<Self::Resolver, S::Error> {
        let mut encoded: Vec<u8> = Vec::new();
        // Encode Address
        encoded.extend_from_slice(&field.0.0);
        // Encode length of access list keys
        encoded.extend_from_slice(&(field.1.len() as u64).to_le_bytes());
        for slot in field.1.iter() {
            // Encode access list key
            encoded.extend_from_slice(&slot.0);
        }

        Ok(AccessListItemWrapperResolver {
            len: encoded.len(),
            inner: ArchivedVec::serialize_from_slice(encoded.as_slice(), serializer)?,
        })
    }
}

impl<D> DeserializeWith<Archived<Vec<u8>>, (H160, Vec<H256>), D> for AccessListItemWrapper
where
    D: Fallible<Error = rkyv::rancor::Error> + ?Sized,
{
    fn deserialize_with(
        field: &Archived<Vec<u8>>,
        _: &mut D,
    ) -> Result<(H160, Vec<H256>), D::Error> {
        let address = H160::from_slice(&field[0..20]);

        let access_list_length =
            u64::from_le_bytes(field[20..28].try_into().map_err(rkyv::rancor::Error::new)?)
                as usize;

        let mut access_list_keys = Vec::with_capacity(access_list_length);
        let mut start = 28_usize;
        let mut end = start + 32_usize; // 60
        for _ in 0..access_list_length {
            access_list_keys.push(H256::from_slice(&field[start..end]));
            start = end;
            end = start + 32_usize;
        }
        Ok((address, access_list_keys))
    }
}

#[cfg(test)]
mod test {
    use ethereum_types::{H160, H256};
    use rkyv::{Archive, Deserialize, Serialize, rancor::Error};

    use crate::types::AccessListItem;

    #[test]
    fn serialize_deserialize_acess_list() {
        #[derive(Deserialize, Serialize, Archive, PartialEq, Debug)]
        struct AccessListStruct {
            #[rkyv(with = crate::rkyv_utils::AccessListItemWrapper)]
            list: AccessListItem,
        }

        let address = H160::random();
        let key_list = (0..10).map(|_| H256::random()).collect::<Vec<_>>();
        let access_list = AccessListStruct {
            list: (address, key_list),
        };
        let bytes = rkyv::to_bytes::<Error>(&access_list).unwrap();
        let deserialized = rkyv::from_bytes::<AccessListStruct, Error>(bytes.as_slice()).unwrap();
        assert_eq!(access_list, deserialized)
    }
}
