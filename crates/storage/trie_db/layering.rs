use ethrex_common::H256;
use ethrex_trie::{Nibbles, TrieDB, TrieError};
use rustc_hash::FxHashMap;
use std::{collections::hash_map::Entry, mem, sync::Arc};

#[derive(Clone, Debug, Default)]
pub struct TrieLayerCache {
    /// Mapping from keys to entries (from all layers).
    data: FxHashMap<Vec<u8>, TrieLayerCacheEntry<Vec<u8>>>,
}

impl TrieLayerCache {
    /// Obtain the cached value from any layer given its key.
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.data.get(key).map(|entry| entry.value.as_slice())
    }

    /// Write a batch of items into the cache at the last layer.
    ///
    /// Items that were already present in that layer will be overwritten. Use
    /// [`TrieLayerCache::commit`] to advance the layers before putting items into the new layer.
    pub fn put_iter(&mut self, iter: impl IntoIterator<Item = (Vec<u8>, Vec<u8>)>) {
        for (key, value) in iter {
            match self.data.entry(key) {
                Entry::Occupied(entry) => {
                    let entry = entry.into_mut();

                    let prev_value = mem::replace(&mut entry.value, value);
                    if entry.layers & 1 == 0 {
                        entry.previous.push(prev_value);
                    }

                    entry.layers |= 1;
                }
                Entry::Vacant(entry) => {
                    entry.insert(TrieLayerCacheEntry {
                        value,
                        layers: 1u128,
                        previous: Vec::new(),
                    });
                }
            }
        }
    }

    /// Return an iterator to extract the elements of the 128th layer.
    ///
    /// If there are not yet 128 layers, it'll return an empty iterator.
    /// Dropping the iterator will not leave the cache in an inconsistent state. All remaining items
    /// will be dropped.
    pub fn commit(&mut self) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut items = Vec::new();
        self.data.retain(|key, entry| {
            entry.layers <<= 1;
            if entry.layers == 0 {
                items.push((
                    key.clone(),
                    if entry.previous.is_empty() {
                        mem::take(&mut entry.value)
                    } else {
                        entry.previous.remove(0)
                    },
                ));

                true
            } else {
                false
            }
        });

        items
    }
}

#[derive(Clone, Debug)]
struct TrieLayerCacheEntry<T> {
    value: T,
    layers: u128,
    previous: Vec<T>,
}

pub struct TrieWrapper {
    pub state_root: H256,
    pub inner: Arc<TrieLayerCache>,
    pub db: Box<dyn TrieDB>,
    pub prefix: Option<H256>,
}

pub fn apply_prefix(prefix: Option<H256>, path: Nibbles) -> Nibbles {
    // Apply a prefix with an invalid nibble (17) as a separator, to
    // differentiate between a state trie value and a storage trie root.
    match prefix {
        Some(prefix) => Nibbles::from_bytes(prefix.as_bytes())
            .append_new(17)
            .concat(&path),
        None => path,
    }
}

impl TrieDB for TrieWrapper {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        let key = apply_prefix(self.prefix, key);
        self.db.flatkeyvalue_computed(key)
    }
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let key = apply_prefix(self.prefix, key);
        if let Some(value) = self.inner.get(key.as_ref()) {
            return Ok(Some(value.to_vec()));
        }
        self.db.get(key)
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        // TODO: Get rid of this.
        unimplemented!("This function should not be called");
    }
}
