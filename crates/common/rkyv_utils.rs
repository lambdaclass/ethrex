use crate::types::AccountInfo;
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
#[rkyv(remote = Vec<Bytes>)]
pub struct BytesVecWrapper {
    #[rkyv(getter = bytes_vec_to_vec)]
    bytes_vec: Vec<Vec<u8>>,
}

fn bytes_vec_to_vec(bytes_vec: &[Bytes]) -> Vec<Vec<u8>> {
    bytes_vec.iter().map(|b| b.to_vec()).collect()
}

impl From<BytesVecWrapper> for Vec<Bytes> {
    fn from(value: BytesVecWrapper) -> Self {
        value.bytes_vec.into_iter().map(Bytes::from).collect()
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

pub struct AccountInfoWrapper;

pub struct AccountInfoWrapperResolver {
    len: usize,
    inner: VecResolver,
}

impl ArchiveWith<AccountInfo> for AccountInfoWrapper {
    type Archived = ArchivedVec<u8>;
    type Resolver = AccountInfoWrapperResolver;

    fn resolve_with(_: &AccountInfo, resolver: Self::Resolver, out: rkyv::Place<Self::Archived>) {
        ArchivedVec::resolve_from_len(resolver.len, resolver.inner, out);
    }
}

impl<S> SerializeWith<AccountInfo, S> for AccountInfoWrapper
where
    S: Fallible + Allocator + Writer + ?Sized,
{
    fn serialize_with(field: &AccountInfo, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        let mut encoded: Vec<u8> = Vec::new();
        // Encode code hash
        encoded.extend_from_slice(&field.code_hash.0);
        // Encode balance
        encoded.extend_from_slice(&field.balance.to_little_endian());
        // Encode nonce
        encoded.extend_from_slice(&field.nonce.to_le_bytes());

        Ok(AccountInfoWrapperResolver {
            len: encoded.len(),
            inner: ArchivedVec::serialize_from_slice(encoded.as_slice(), serializer)?,
        })
    }
}

impl<D> DeserializeWith<Archived<Vec<u8>>, AccountInfo, D> for AccountInfoWrapper
where
    D: Fallible<Error = rkyv::rancor::Error> + ?Sized,
{
    fn deserialize_with(field: &Archived<Vec<u8>>, _: &mut D) -> Result<AccountInfo, D::Error> {
        // Get code hash (first 32 bytes)
        let code_hash = H256::from_slice(&field[0..32]);

        // Get balance (next 32 bytes)
        let mut balance_bytes = [0u8; 32];
        balance_bytes.copy_from_slice(&field[32..64]);
        let balance = U256::from_little_endian(&balance_bytes);

        // Get nonce (next 8 bytes)
        let nonce = u64::from_le_bytes(field[64..72].try_into().map_err(rkyv::rancor::Error::new)?);

        Ok(AccountInfo {
            code_hash,
            balance,
            nonce,
        })
    }
}

pub struct EncodedTrieWrapper;

pub struct EncodedTrieWrapperResolver {
    len: usize,
    inner: VecResolver,
}

impl ArchiveWith<(Option<Vec<u8>>, Vec<Vec<u8>>)> for EncodedTrieWrapper {
    type Archived = ArchivedVec<u8>;
    type Resolver = EncodedTrieWrapperResolver;

    fn resolve_with(
        _: &(Option<Vec<u8>>, Vec<Vec<u8>>),
        resolver: Self::Resolver,
        out: rkyv::Place<Self::Archived>,
    ) {
        ArchivedVec::resolve_from_len(resolver.len, resolver.inner, out);
    }
}

impl<S> SerializeWith<(Option<Vec<u8>>, Vec<Vec<u8>>), S> for EncodedTrieWrapper
where
    S: Fallible + Allocator + Writer + ?Sized,
{
    fn serialize_with(
        field: &(Option<Vec<u8>>, Vec<Vec<u8>>),
        serializer: &mut S,
    ) -> Result<Self::Resolver, S::Error> {
        let mut encoded: Vec<u8> = Vec::new();

        // Encode whether Option<Vec<u8>> is Some or None
        let has_root = field.0.is_some() as u8;
        encoded.push(has_root);

        // If Some, encode the root node
        if let Some(root) = &field.0 {
            // Encode length of root
            encoded.extend_from_slice(&(root.len() as u64).to_le_bytes());
            // Encode root content
            encoded.extend_from_slice(root);
        }

        // Encode number of nodes in Vec<Vec<u8>>
        encoded.extend_from_slice(&(field.1.len() as u64).to_le_bytes());

        // Encode each node
        for node in &field.1 {
            // Encode length of node
            encoded.extend_from_slice(&(node.len() as u64).to_le_bytes());
            // Encode node content
            encoded.extend_from_slice(node);
        }

        Ok(EncodedTrieWrapperResolver {
            len: encoded.len(),
            inner: ArchivedVec::serialize_from_slice(encoded.as_slice(), serializer)?,
        })
    }
}

impl<D> DeserializeWith<Archived<Vec<u8>>, (Option<Vec<u8>>, Vec<Vec<u8>>), D>
    for EncodedTrieWrapper
where
    D: Fallible<Error = rkyv::rancor::Error> + ?Sized,
{
    fn deserialize_with(
        field: &Archived<Vec<u8>>,
        _: &mut D,
    ) -> Result<(Option<Vec<u8>>, Vec<Vec<u8>>), D::Error> {
        let mut offset = 0;

        // Read if root is Some or None
        let has_root = field[offset] != 0;
        offset += 1;

        // Read root if present
        let root = if has_root {
            // Read root length
            let root_len = u64::from_le_bytes(
                field[offset..offset + 8]
                    .try_into()
                    .map_err(rkyv::rancor::Error::new)?,
            ) as usize;
            offset += 8;

            // Read root content
            let root_content = field[offset..offset + root_len].to_vec();
            offset += root_len;

            Some(root_content)
        } else {
            None
        };

        // Read number of nodes
        let nodes_count = u64::from_le_bytes(
            field[offset..offset + 8]
                .try_into()
                .map_err(rkyv::rancor::Error::new)?,
        ) as usize;
        offset += 8;

        // Read each node
        let mut nodes = Vec::with_capacity(nodes_count);
        for _ in 0..nodes_count {
            // Read node length
            let node_len = u64::from_le_bytes(
                field[offset..offset + 8]
                    .try_into()
                    .map_err(rkyv::rancor::Error::new)?,
            ) as usize;
            offset += 8;

            // Read node content
            let node_content = field[offset..offset + node_len].to_vec();
            offset += node_len;

            nodes.push(node_content);
        }

        Ok((root, nodes))
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
