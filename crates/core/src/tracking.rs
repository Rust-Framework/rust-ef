//! Change tracking — entity state management, snapshots, and detection.
//!
//! `ChangeTracker` is the single authoritative source for entity state, original
//! snapshots, and modified-property lists. `DbSet` holds only the typed entity
//! + `entry_id`; tracking state lives here, joined by `entry_id` during save.

use crate::entity::EntityState;
use crate::entity_snapshot::EntitySnapshot;
use std::any::TypeId;
use std::collections::HashMap;

/// Tracks changes to entities within a DbContext.
///
/// The single source of truth for `EntityState`, original `EntitySnapshot`,
/// `modified_properties`, and `is_upsert` across all tracked entities.
/// `DbSet<T>` holds the typed entity + `entry_id`; this tracker holds the
/// tracking state, joined by `entry_id` during `save_changes`.
#[derive(Debug)]
pub struct ChangeTracker {
    entries: HashMap<u64, TrackerEntry>,
    auto_detect_changes: bool,
    next_id: u64,
}

/// A public read-only view of a tracked entry.
#[derive(Debug, Clone)]
pub struct EntityEntry {
    pub entry_id: u64,
    pub type_id: TypeId,
    pub type_name: String,
    pub state: EntityState,
    pub modified_properties: Vec<String>,
}

/// Lightweight, type-erased view of a pending entity entry used to build
/// `SaveChangesContext` for interceptors.
#[derive(Debug, Clone)]
pub struct EntityEntryView {
    pub type_id: TypeId,
    pub type_name: String,
    pub state: EntityState,
}

/// Internal tracker entry — stores state, original snapshot, and modified
/// property names for a single tracked entity.
struct TrackerEntry {
    id: u64,
    type_id: TypeId,
    type_name: String,
    state: EntityState,
    original: Option<EntitySnapshot>,
    modified_properties: Vec<String>,
    is_upsert: bool,
}

impl std::fmt::Debug for TrackerEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackerEntry")
            .field("id", &self.id)
            .field("type_name", &self.type_name)
            .field("state", &self.state)
            .finish()
    }
}

impl ChangeTracker {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            auto_detect_changes: true,
            next_id: 0,
        }
    }

    /// Begins tracking an entity with the given state and optional original
    /// snapshot. Returns the assigned `entry_id` for joining with `DbSet`.
    pub fn track(
        &mut self,
        type_id: TypeId,
        type_name: &str,
        state: EntityState,
        original: Option<EntitySnapshot>,
        is_upsert: bool,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.insert(
            id,
            TrackerEntry {
                id,
                type_id,
                type_name: type_name.to_string(),
                state,
                original,
                modified_properties: Vec::new(),
                is_upsert,
            },
        );
        id
    }

    /// Compares current property snapshots against stored originals.
    ///
    /// For each `Unchanged` entry whose `original` exists, if the current
    /// snapshot differs, transitions to `Modified` and records the changed
    /// field names in `modified_properties`.
    pub fn detect_changes(&mut self, current_snapshots: &[(u64, EntitySnapshot)]) {
        let snap_map: HashMap<u64, &EntitySnapshot> =
            current_snapshots.iter().map(|(id, s)| (*id, s)).collect();

        for entry in self.entries.values_mut() {
            if entry.state != EntityState::Unchanged {
                continue;
            }
            let Some(original) = &entry.original else {
                continue;
            };
            let Some(current) = snap_map.get(&entry.id) else {
                continue;
            };

            if **current == *original {
                entry.modified_properties.clear();
                continue;
            }

            let changed: Vec<String> = current
                .iter()
                .filter(|(k, v)| original.get(k) != Some(*v))
                .map(|(k, _)| k.to_string())
                .collect();

            if !changed.is_empty() {
                entry.state = EntityState::Modified;
                entry.modified_properties = changed;
            }
        }
    }

    // ── Query methods (read path for save phases) ──

    pub fn entry_state(&self, entry_id: u64) -> Option<EntityState> {
        self.entries.get(&entry_id).map(|e| e.state)
    }

    pub fn entry_original(&self, entry_id: u64) -> Option<&EntitySnapshot> {
        self.entries
            .get(&entry_id)
            .and_then(|e| e.original.as_ref())
    }

    pub fn entry_modified(&self, entry_id: u64) -> Option<&[String]> {
        self.entries
            .get(&entry_id)
            .map(|e| e.modified_properties.as_slice())
    }

    pub fn entry_is_upsert(&self, entry_id: u64) -> bool {
        self.entries.get(&entry_id).is_some_and(|e| e.is_upsert)
    }

    // ── Mutation methods ──

    pub fn set_state(&mut self, entry_id: u64, state: EntityState) {
        if let Some(entry) = self.entries.get_mut(&entry_id) {
            entry.state = state;
        }
    }

    pub fn set_modified(&mut self, entry_id: u64, modified: Vec<String>) {
        if let Some(entry) = self.entries.get_mut(&entry_id) {
            entry.modified_properties = modified;
        }
    }

    /// After successful SaveChanges:
    /// - Deleted entries are removed
    /// - Added/Modified → Unchanged (original refreshed to current snapshot)
    /// - `modified_properties` cleared
    ///
    /// `current_snapshots` provides the post-save entity snapshots (with
    /// backfilled auto-increment PKs) to become the new `original`.
    pub fn accept_all_changes(&mut self, current_snapshots: &[(u64, EntitySnapshot)]) {
        let snap_map: HashMap<u64, &EntitySnapshot> =
            current_snapshots.iter().map(|(id, s)| (*id, s)).collect();

        self.entries.retain(|_, e| e.state != EntityState::Deleted);

        for (id, entry) in &mut self.entries {
            if entry.state == EntityState::Added || entry.state == EntityState::Modified {
                entry.state = EntityState::Unchanged;
                entry.modified_properties.clear();
                if let Some(snap) = snap_map.get(id) {
                    entry.original = Some((*snap).clone());
                }
            }
        }
    }

    /// Reverts all pending changes:
    /// - Added entries are removed
    /// - Modified/Deleted → Unchanged (original stays as-is)
    /// - `modified_properties` cleared
    pub fn reject_all_changes(&mut self) {
        self.entries.retain(|_, e| e.state != EntityState::Added);
        for entry in self.entries.values_mut() {
            if entry.state == EntityState::Modified || entry.state == EntityState::Deleted {
                entry.state = EntityState::Unchanged;
                entry.modified_properties.clear();
            }
        }
    }

    /// Detaches a specific entry by ID.
    pub fn detach(&mut self, entry_id: u64) {
        self.entries.remove(&entry_id);
    }

    /// Removes all tracked entries of the given type. Used by `load_all` to
    /// clear stale tracking state before re-attaching fresh entities.
    pub fn clear_by_type(&mut self, type_id: TypeId) {
        self.entries.retain(|_, e| e.type_id != type_id);
    }

    // ── Aggregate queries ──

    pub fn has_changes(&self) -> bool {
        self.entries.values().any(|e| {
            matches!(
                e.state,
                EntityState::Added | EntityState::Modified | EntityState::Deleted
            )
        })
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn entries(&self) -> Vec<EntityEntry> {
        self.entries
            .values()
            .map(|e| EntityEntry {
                entry_id: e.id,
                type_id: e.type_id,
                type_name: e.type_name.clone(),
                state: e.state,
                modified_properties: e.modified_properties.clone(),
            })
            .collect()
    }

    pub fn count_by_state(&self, state: EntityState) -> usize {
        self.entries.values().filter(|e| e.state == state).count()
    }

    pub fn entries_by_state(&self, state: EntityState) -> Vec<EntityEntry> {
        self.entries
            .values()
            .filter(|e| e.state == state)
            .map(|e| EntityEntry {
                entry_id: e.id,
                type_id: e.type_id,
                type_name: e.type_name.clone(),
                state: e.state,
                modified_properties: e.modified_properties.clone(),
            })
            .collect()
    }

    /// Returns `(type_id, type_name, state)` views for all tracked entries,
    /// used to build `SaveChangesContext` for interceptors.
    pub fn entry_views(&self) -> Vec<EntityEntryView> {
        self.entries
            .values()
            .map(|e| EntityEntryView {
                type_id: e.type_id,
                type_name: e.type_name.clone(),
                state: e.state,
            })
            .collect()
    }

    pub fn is_auto_detect_changes_enabled(&self) -> bool {
        self.auto_detect_changes
    }

    pub fn set_auto_detect_changes(&mut self, enabled: bool) {
        self.auto_detect_changes = enabled;
    }
}

impl Default for ChangeTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::DbValue;

    #[test]
    fn track_assigns_sequential_ids() {
        let mut tracker = ChangeTracker::new();
        let id1 = tracker.track(TypeId::of::<u32>(), "u32", EntityState::Added, None, false);
        let id2 = tracker.track(TypeId::of::<u32>(), "u32", EntityState::Added, None, false);
        assert_eq!(id1, 0);
        assert_eq!(id2, 1);
        assert_eq!(tracker.count_by_state(EntityState::Added), 2);
    }

    #[test]
    fn entry_queries_return_tracked_data() {
        let mut tracker = ChangeTracker::new();
        let snap = EntitySnapshot::new(vec![("id", DbValue::I32(42))]);
        let id = tracker.track(
            TypeId::of::<u32>(),
            "u32",
            EntityState::Unchanged,
            Some(snap),
            false,
        );
        assert_eq!(tracker.entry_state(id), Some(EntityState::Unchanged));
        assert!(tracker.entry_original(id).is_some());
        assert_eq!(tracker.entry_modified(id), Some(&[][..]));
        assert!(!tracker.entry_is_upsert(id));
    }

    #[test]
    fn detect_changes_marks_modified() {
        let mut tracker = ChangeTracker::new();
        let original = EntitySnapshot::new(vec![
            ("id", DbValue::I32(1)),
            ("name", DbValue::String("old".into())),
        ]);
        let id = tracker.track(
            TypeId::of::<u32>(),
            "u32",
            EntityState::Unchanged,
            Some(original),
            false,
        );
        let current = EntitySnapshot::new(vec![
            ("id", DbValue::I32(1)),
            ("name", DbValue::String("new".into())),
        ]);
        tracker.detect_changes(&[(id, current)]);
        assert_eq!(tracker.entry_state(id), Some(EntityState::Modified));
        let modified = tracker.entry_modified(id).unwrap();
        assert_eq!(modified, &["name".to_string()]);
    }

    #[test]
    fn detect_changes_noop_when_unchanged() {
        let mut tracker = ChangeTracker::new();
        let snap = EntitySnapshot::new(vec![("id", DbValue::I32(1))]);
        let id = tracker.track(
            TypeId::of::<u32>(),
            "u32",
            EntityState::Unchanged,
            Some(snap.clone()),
            false,
        );
        tracker.detect_changes(&[(id, snap)]);
        assert_eq!(tracker.entry_state(id), Some(EntityState::Unchanged));
    }

    #[test]
    fn accept_all_changes_transitions_and_updates_original() {
        let mut tracker = ChangeTracker::new();
        let id = tracker.track(TypeId::of::<u32>(), "u32", EntityState::Added, None, false);
        let current = EntitySnapshot::new(vec![("id", DbValue::I32(10))]);
        tracker.accept_all_changes(&[(id, current)]);
        assert_eq!(tracker.entry_state(id), Some(EntityState::Unchanged));
        assert!(tracker.entry_original(id).is_some());
    }

    #[test]
    fn accept_all_changes_removes_deleted() {
        let mut tracker = ChangeTracker::new();
        let id = tracker.track(
            TypeId::of::<u32>(),
            "u32",
            EntityState::Deleted,
            None,
            false,
        );
        tracker.accept_all_changes(&[]);
        assert_eq!(tracker.entry_state(id), None);
    }

    #[test]
    fn reject_all_changes_reverts_modified() {
        let mut tracker = ChangeTracker::new();
        let id = tracker.track(
            TypeId::of::<u32>(),
            "u32",
            EntityState::Modified,
            Some(EntitySnapshot::new(vec![("id", DbValue::I32(1))])),
            false,
        );
        tracker.reject_all_changes();
        assert_eq!(tracker.entry_state(id), Some(EntityState::Unchanged));
    }

    #[test]
    fn reject_all_changes_removes_added() {
        let mut tracker = ChangeTracker::new();
        let id = tracker.track(TypeId::of::<u32>(), "u32", EntityState::Added, None, false);
        tracker.reject_all_changes();
        assert_eq!(tracker.entry_state(id), None);
    }
}
