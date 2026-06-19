//! Query cache / Identity Map — entity-level caching by primary key.
//!
//! Implements the Identity Map pattern: ensures each entity with a given
//! primary key is loaded only once per DbContext session, preventing
//! duplicate identity issues.

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// Identity Map cache that stores entities by their primary key values.
///
/// Uses `(TypeId, String)` as the cache key where the string is a
/// serialized form of the primary key (e.g., "42" for single key,
/// "42|hello" for composite key).
#[derive(Default)]
pub struct DbCache {
    entries: HashMap<(TypeId, String), Box<dyn Any + Send + Sync>>,
}

impl DbCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Stores an entity in the cache identified by its type and serialized key.
    pub fn store<T: Send + Sync + 'static>(
        &mut self,
        type_id: TypeId,
        key: impl Into<String>,
        entity: T,
    ) {
        self.entries.insert((type_id, key.into()), Box::new(entity));
    }

    /// Retrieves an entity from the cache.
    pub fn get<T: 'static>(&self, type_id: TypeId, key: &str) -> Option<&T> {
        self.entries
            .get(&(type_id, key.to_string()))
            .and_then(|boxed| boxed.downcast_ref::<T>())
    }

    /// Removes an entity from the cache by key.
    pub fn remove(&mut self, type_id: TypeId, key: &str) {
        self.entries.remove(&(type_id, key.to_string()));
    }

    /// Clears all cached entries for a specific type.
    pub fn clear_type(&mut self, type_id: TypeId) {
        self.entries.retain(|(tid, _), _| *tid != type_id);
    }

    /// Clears all cached entries.
    pub fn clear_all(&mut self) {
        self.entries.clear();
    }

    /// Serializes primary key values into a cache key string.
    /// Usage: `DbCache::serialize_key(&[("col1", "v1"), ("col2", "v2")])` -> `"v1|v2"`
    pub fn serialize_key(key_values: &[&str]) -> String {
        key_values.join("|")
    }

    /// Returns the number of cached entities.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Invalidates cache entries that match a type and any of the given keys.
    pub fn invalidate_type_keys<I: IntoIterator<Item = String>>(
        &mut self,
        type_id: TypeId,
        keys: I,
    ) {
        for key in keys {
            self.remove(type_id, &key);
        }
    }
}

impl std::fmt::Debug for DbCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbCache")
            .field("entries", &self.entries.len())
            .finish()
    }
}

/// Helper to compute a cache key from HashMap of DbValue key-value pairs.
pub fn compute_cache_key(key_values: &HashMap<String, crate::provider::DbValue>) -> String {
    let mut parts: Vec<String> = key_values
        .iter()
        .map(|(_, v)| format!("{}", v))
        .collect();
    parts.sort(); // Deterministic ordering for composite keys
    parts.join("|")
}
