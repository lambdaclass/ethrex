use ethereum_types::H256;
use rkyv::{
    Archive, Deserialize, Serialize,
    rancor::{Fallible, Source},
    ser::{Allocator, Writer},
    vec::{ArchivedVec, VecResolver},
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use smallvec::SmallVec;
use std::hash::{Hash, Hasher};

/// rkyv wrapper that archives `SmallVec<[u8; N]>` as `ArchivedVec<u8>`.
pub struct SmallVecAsVec;

impl<const N: usize> ArchiveWith<SmallVec<[u8; N]>> for SmallVecAsVec {
    type Archived = ArchivedVec<u8>;
    type Resolver = VecResolver;

    fn resolve_with(
        field: &SmallVec<[u8; N]>,
        resolver: Self::Resolver,
        out: rkyv::Place<Self::Archived>,
    ) {
        ArchivedVec::resolve_from_len(field.len(), resolver, out);
    }
}

impl<S, const N: usize> SerializeWith<SmallVec<[u8; N]>, S> for SmallVecAsVec
where
    S: Fallible + Allocator + Writer + ?Sized,
{
    fn serialize_with(
        field: &SmallVec<[u8; N]>,
        serializer: &mut S,
    ) -> Result<VecResolver, S::Error> {
        ArchivedVec::serialize_from_slice(field.as_slice(), serializer)
    }
}

impl<D, const N: usize> DeserializeWith<ArchivedVec<u8>, SmallVec<[u8; N]>, D> for SmallVecAsVec
where
    D: Fallible + ?Sized,
    D::Error: Source,
{
    fn deserialize_with(field: &ArchivedVec<u8>, _: &mut D) -> Result<SmallVec<[u8; N]>, D::Error> {
        Ok(SmallVec::from_slice(field.as_slice()))
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
