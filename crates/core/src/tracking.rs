//! Change tracking �?entity state management, snapshots, and detection.
//!
//! Implements EFCore's change-tracking semantics:
//!   - Entity states: Detached | Added | Unchanged | Modified | Deleted
//!   - Property-level snapshots taken at tracking time
//!   - `detect_changes()` compares current values against snapshots
//!   - `has_changes()` quickly checks for any pending mutations
//!   - `accept_all_changes()` resets states after successful SaveChanges

use crate::entity::EntityState;
use std::collections::HashMap;

/// Tracks changes to entities within a DbContext.
#[derive(Debug)]
pub struct ChangeTracker {
    entries: Vec<TrackerEntry>,
    auto_detect_changes: bool,
    /// Counter for generating stable entry IDs.
    next_id: u64,
}

/// A public read-only view of a tracked entry.
#[derive(Debug, Clone)]
pub struct EntityEntry {
    pub entry_id: u64,
    pub type_id: std::any::TypeId,
    pub type_name: String,
    pub state: EntityState,
    /// Property names that have been modified (populated after detect_changes).
    pub modified_properties: Vec<String>,
}

/// Lightweight, type-erased view of a pending entity entry used to build
/// `SaveChangesContext` from `DbSet.entries` (the real save data source).
///
/// Unlike `EntityEntry`, this carries no `entry_id` / `modified_properties`
/// (which only have meaning inside `ChangeTracker`). It exists so that
/// interceptors receive a snapshot consistent with what `save_changes()`
/// will actually commit, instead of the legacy (empty) `change_tracker`.
#[derive(Debug, Clone)]
pub struct EntityEntryView {
    pub type_id: std::any::TypeId,
    pub type_name: String,
    pub state: EntityState,
}

/// Internal tracker entry storing the entity, its state, and original snapshots.
struct TrackerEntry {
    id: u64,
    type_id: std::any::TypeId,
    type_name: String,
    state: EntityState,
    /// Original property values captured when the entity was first tracked.
    snapshot: HashMap<String, PropertySnapshot>,
}

/// A stored snapshot of a single property.
#[derive(Debug, Clone)]
struct PropertySnapshot {
    /// Serialized string representation of the value at tracking time.
    serialized: String,
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
            entries: Vec::new(),
            auto_detect_changes: true,
            next_id: 0,
        }
    }

    /// Takes a snapshot of the entity's current properties and begins tracking.
    ///
    /// The `snapshotter` closure should return a map of property-name �?string
    /// serialization of the property's current value.
    pub fn track_entity_with_snapshot(
        &mut self,
        type_id: std::any::TypeId,
        type_name: &str,
        state: EntityState,
        snapshot: HashMap<String, String>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        self.entries.push(TrackerEntry {
            id,
            type_id,
            type_name: type_name.to_string(),
            state,
            snapshot: snapshot
                .into_iter()
                .map(|(k, v)| (k, PropertySnapshot { serialized: v }))
                .collect(),
        });

        id
    }

    /// Begins tracking an entity without taking a snapshot (used for Added entities
    /// that have no pre-existing state).
    pub fn track_entity(
        &mut self,
        type_id: std::any::TypeId,
        type_name: &str,
        state: EntityState,
    ) -> u64 {
        self.track_entity_with_snapshot(type_id, type_name, state, HashMap::new())
    }

    /// Compares current property values (provided by the caller) against the
    /// stored snapshots. Any property whose value differs is marked as modified,
    /// and the entity transitions to `EntityState::Modified`.
    pub fn detect_changes_with_properties(
        &mut self,
        current_properties: &[(u64, HashMap<String, String>)],
    ) {
        // Build a lookup of current values by entry ID
        let current_map: HashMap<u64, &HashMap<String, String>> = current_properties
            .iter()
            .map(|(id, props)| (*id, props))
            .collect();

        for entry in &mut self.entries {
            // Only check Unchanged entities
            if entry.state != EntityState::Unchanged {
                continue;
            }

            if let Some(current) = current_map.get(&entry.id) {
                for (prop_name, snapshot) in &entry.snapshot {
                    let changed = match current.get(prop_name) {
                        Some(current_val) => current_val != &snapshot.serialized,
                        None => true, // Property removed = change
                    };

                    if changed {
                        entry.state = EntityState::Modified;
                        break; // One changed property is enough
                    }
                }
            }
        }
    }

    /// Marks the property snapshot for the given entry as updated.
    /// Used after a successful SaveChanges to update the "original" values.
    pub fn update_snapshot(&mut self, entry_id: u64, properties: HashMap<String, String>) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == entry_id) {
            entry.snapshot = properties
                .into_iter()
                .map(|(k, v)| (k, PropertySnapshot { serialized: v }))
                .collect();
        }
    }

    /// Returns whether any tracked entity has changes pending.
    pub fn has_changes(&self) -> bool {
        self.entries.iter().any(|e| {
            matches!(
                e.state,
                EntityState::Added | EntityState::Modified | EntityState::Deleted
            )
        })
    }

    /// Clears all tracked entities.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Returns an iterator over tracked entry views.
    pub fn entries(&self) -> Vec<EntityEntry> {
        self.entries
            .iter()
            .map(|e| EntityEntry {
                entry_id: e.id,
                type_id: e.type_id,
                type_name: e.type_name.clone(),
                state: e.state,
                modified_properties: Vec::new(),
            })
            .collect()
    }

    /// Returns count of entities in a given state.
    pub fn count_by_state(&self, state: EntityState) -> usize {
        self.entries.iter().filter(|e| e.state == state).count()
    }

    /// Returns entities grouped by their state.
    pub fn entries_by_state(&self, state: EntityState) -> Vec<EntityEntry> {
        self.entries()
            .into_iter()
            .filter(|e| e.state == state)
            .collect()
    }

    /// After successful SaveChanges:
    /// - Added �?Unchanged (save snapshot)
    /// - Modified �?Unchanged (save current as new snapshot)
    /// - Deleted �?Detached (removed from tracker)
    pub fn accept_all_changes(&mut self) {
        self.entries.retain(|e| e.state != EntityState::Deleted);
        for entry in &mut self.entries {
            if entry.state == EntityState::Added || entry.state == EntityState::Modified {
                entry.state = EntityState::Unchanged;
            }
        }
    }

    /// Reverts all pending changes (detached state reverted).
    pub fn reject_all_changes(&mut self) {
        self.entries.retain(|e| e.state != EntityState::Added);
        for entry in &mut self.entries {
            if entry.state == EntityState::Modified || entry.state == EntityState::Deleted {
                entry.state = EntityState::Unchanged;
            }
        }
    }

    /// Detaches a specific entry by ID.
    pub fn detach(&mut self, entry_id: u64) {
        self.entries.retain(|e| e.id != entry_id);
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

// ---------------------------------------------------------------------------
// TrackedEntity �?generic container for tracked entities
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct TrackedEntity<T> {
    pub entity: T,
    pub entry_id: u64,
    pub state: EntityState,
}

impl<T> TrackedEntity<T> {
    pub fn new(entity: T, entry_id: u64, state: EntityState) -> Self {
        Self {
            entity,
            entry_id,
            state,
        }
    }
}
