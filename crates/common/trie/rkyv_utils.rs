use core::hash::{Hash, Hasher};
use ethereum_types::H256;
use rkyv::{Archive, Deserialize, Serialize};

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
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ArchivedH256Wrapper {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl Eq for ArchivedH256Wrapper {}

impl Hash for ArchivedH256Wrapper {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}
