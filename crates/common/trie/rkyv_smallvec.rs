use rkyv::{
    Place,
    vec::{ArchivedVec, VecResolver},
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use smallvec::SmallVec;

/// rkyv wrapper that serializes SmallVec as ArchivedVec.
pub struct SmallVecAsVec;

impl<const N: usize> ArchiveWith<SmallVec<[u8; N]>> for SmallVecAsVec {
    type Archived = ArchivedVec<u8>;
    type Resolver = VecResolver;

    fn resolve_with(
        field: &SmallVec<[u8; N]>,
        resolver: Self::Resolver,
        out: Place<Self::Archived>,
    ) {
        ArchivedVec::resolve_from_slice(field.as_slice(), resolver, out);
    }
}

impl<S, const N: usize> SerializeWith<SmallVec<[u8; N]>, S> for SmallVecAsVec
where
    S: rkyv::ser::Allocator + rkyv::ser::Writer + rkyv::rancor::Fallible + ?Sized,
    S::Error: rkyv::rancor::Source,
{
    fn serialize_with(
        field: &SmallVec<[u8; N]>,
        serializer: &mut S,
    ) -> Result<Self::Resolver, S::Error> {
        ArchivedVec::serialize_from_slice(field.as_slice(), serializer)
    }
}

impl<D, const N: usize> DeserializeWith<ArchivedVec<u8>, SmallVec<[u8; N]>, D> for SmallVecAsVec
where
    D: rkyv::rancor::Fallible + ?Sized,
{
    fn deserialize_with(
        field: &ArchivedVec<u8>,
        _deserializer: &mut D,
    ) -> Result<SmallVec<[u8; N]>, D::Error> {
        Ok(SmallVec::from_slice(field.as_slice()))
    }
}
