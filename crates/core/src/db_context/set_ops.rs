//! Type-erased set operations — bridges `DbContext::save_changes()` and
//! concrete `DbSet<T>` instances via `Box<dyn ErasedSetOps>`.
//!
//! All tracking state lives in `ChangeTracker`; `DbSet` holds only typed
//! entities + `entry_id`. These methods join the two by `entry_id`.

use crate::cascade::{
    resolve_delete_behavior, CascadeDeleteAction, CascadeDeleteDirective, DrainedChild,
};
use crate::db_set::DbSet;
use crate::entity::{
    EntityState, IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter,
};
use crate::entity_snapshot::EntitySnapshot;
use crate::error::{EFError, EFResult};
use crate::metadata::{EntityTypeMeta, NavigationKind};
use crate::provider::{IAsyncConnection, IDatabaseProvider};
use crate::relations::DeleteBehavior;
use crate::tracking::ChangeTracker;
use std::any::{Any, TypeId};

use super::save_phases::{
    delete_deleted_phase, insert_added_phase, update_modified_phase, upsert_added_phase,
};

#[async_trait::async_trait]
pub(crate) trait ErasedSetOps: Send + Sync {
    /// Detects changes by comparing current entity snapshots against the
    /// tracker's stored originals, transitioning `Unchanged` → `Modified`.
    fn detect_changes(&self, raw_set: &mut (dyn Any + Send + Sync), tracker: &mut ChangeTracker);

    /// Accepts all pending changes: Added/Modified → Unchanged (with refreshed
    /// originals), Deleted entries removed from both DbSet and tracker.
    fn accept_all_changes(
        &self,
        raw_set: &mut (dyn Any + Send + Sync + 'static),
        tracker: &mut ChangeTracker,
    );

    /// Collects type-erased views of all tracked entries, joined by `entry_id`.
    fn collect_entries(
        &self,
        raw_set: &(dyn Any + Send + Sync),
        tracker: &ChangeTracker,
    ) -> Vec<crate::tracking::EntityEntryView>;

    // ── Cascade pipeline methods ──

    /// Drains HasMany/ManyToMany children from all Added entries. Returns
    /// type-erased children with parent linkage info for FK fixup.
    fn drain_cascade_children(
        &self,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &ChangeTracker,
        meta: &EntityTypeMeta,
    ) -> Vec<DrainedChild>;

    /// Adds a cascade-drained child (type-erased) to this set as Added (or
    /// Unchanged if it already has a PK). Returns the new entry index.
    fn add_cascade_child(
        &self,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &mut ChangeTracker,
        child: Box<dyn Any + Send + Sync>,
    ) -> Option<usize>;

    /// Reads the first PK value (as i64) of the entry at `idx`.
    fn get_pk_at(&self, raw_set: &(dyn Any + Send + Sync), idx: usize) -> Option<i64>;

    /// Sets the FK field on the entry at `idx` pointing to `target_type`.
    fn set_fk_at(
        &self,
        raw_set: &mut (dyn Any + Send + Sync),
        idx: usize,
        target_type: TypeId,
        key: i64,
    );

    /// Phase 1a: INSERT Added (non-upsert), backfill PKs.
    async fn insert_added(
        &self,
        conn: &mut (dyn IAsyncConnection + Send),
        provider: &dyn IDatabaseProvider,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &ChangeTracker,
        meta: &EntityTypeMeta,
    ) -> EFResult<usize>;

    /// Phase 1b: UPSERT Added (is_upsert = true).
    async fn upsert_added(
        &self,
        conn: &mut (dyn IAsyncConnection + Send),
        provider: &dyn IDatabaseProvider,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &ChangeTracker,
        meta: &EntityTypeMeta,
    ) -> EFResult<usize>;

    /// Phase 2: UPDATE Modified.
    async fn update_modified(
        &self,
        conn: &mut (dyn IAsyncConnection + Send),
        provider: &dyn IDatabaseProvider,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &ChangeTracker,
        meta: &EntityTypeMeta,
    ) -> EFResult<usize>;

    /// Phase 3: DELETE Deleted.
    async fn delete_deleted(
        &self,
        conn: &mut (dyn IAsyncConnection + Send),
        provider: &dyn IDatabaseProvider,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &ChangeTracker,
        meta: &EntityTypeMeta,
    ) -> EFResult<usize>;

    /// Drains HasMany children from Deleted entries and collects direct DELETE
    /// directives for untracked dependents.
    fn drain_cascade_deleted_children(
        &self,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &ChangeTracker,
        meta: &EntityTypeMeta,
        processed: &mut std::collections::HashSet<(TypeId, usize)>,
    ) -> (Vec<DrainedChild>, Vec<CascadeDeleteDirective>);

    /// Adds a cascade-drained child to this set as Deleted. Returns the new
    /// entry index, or `None` if the type doesn't match or the child has no PK.
    fn add_cascade_deleted_child(
        &self,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &mut ChangeTracker,
        child: Box<dyn Any + Send + Sync>,
    ) -> Option<usize>;
}

pub(crate) struct SetOps<E> {
    _phantom: std::marker::PhantomData<E>,
}
impl<E> SetOps<E> {
    pub(crate) fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

#[async_trait::async_trait]
impl<E> ErasedSetOps for SetOps<E>
where
    E: IEntityType
        + IEntitySnapshot
        + IGetKeyValues
        + IFromRow
        + INavigationSetter
        + Send
        + Sync
        + 'static,
{
    fn detect_changes(&self, raw_set: &mut (dyn Any + Send + Sync), tracker: &mut ChangeTracker) {
        let Some(db_set) = raw_set.downcast_mut::<DbSet<E>>() else {
            return;
        };
        let snapshots: Vec<(u64, EntitySnapshot)> = db_set
            .entries
            .iter()
            .map(|e| (e.entry_id, e.entity.snapshot()))
            .collect();
        tracker.detect_changes(&snapshots);
    }

    fn accept_all_changes(
        &self,
        raw_set: &mut (dyn Any + Send + Sync + 'static),
        tracker: &mut ChangeTracker,
    ) {
        let Some(db_set) = raw_set.downcast_mut::<DbSet<E>>() else {
            return;
        };
        // Collect Deleted entry_ids before mutating the DbSet.
        let deleted_ids: Vec<u64> = db_set
            .entries
            .iter()
            .filter(|e| tracker.entry_state(e.entry_id) == Some(EntityState::Deleted))
            .map(|e| e.entry_id)
            .collect();
        db_set
            .entries
            .retain(|e| !deleted_ids.contains(&e.entry_id));
        // Refresh tracker originals from post-save entity snapshots.
        let snapshots: Vec<(u64, EntitySnapshot)> = db_set
            .entries
            .iter()
            .map(|e| (e.entry_id, e.entity.snapshot()))
            .collect();
        tracker.accept_all_changes(&snapshots);
    }

    fn collect_entries(
        &self,
        raw_set: &(dyn Any + Send + Sync),
        tracker: &ChangeTracker,
    ) -> Vec<crate::tracking::EntityEntryView> {
        let Some(db_set) = raw_set.downcast_ref::<DbSet<E>>() else {
            return Vec::new();
        };
        let type_name = E::entity_meta().type_name.to_string();
        db_set
            .entries
            .iter()
            .filter_map(|e| {
                let state = tracker.entry_state(e.entry_id)?;
                Some(crate::tracking::EntityEntryView {
                    type_id: TypeId::of::<E>(),
                    type_name: type_name.clone(),
                    state,
                })
            })
            .collect()
    }

    fn drain_cascade_children(
        &self,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &ChangeTracker,
        meta: &EntityTypeMeta,
    ) -> Vec<DrainedChild> {
        let Some(db_set) = raw_set.downcast_mut::<DbSet<E>>() else {
            return Vec::new();
        };
        let mut result = Vec::new();
        for (entry_idx, entry) in db_set.entries.iter_mut().enumerate() {
            if tracker.entry_state(entry.entry_id) != Some(EntityState::Added) {
                continue;
            }
            if tracker.entry_is_upsert(entry.entry_id) {
                continue;
            }
            for nav in &meta.navigations {
                if !matches!(
                    nav.kind,
                    NavigationKind::HasMany | NavigationKind::ManyToMany
                ) {
                    continue;
                }
                if let Some(items) = entry.entity.drain_has_many(nav.field_name.as_ref()) {
                    for item in items {
                        result.push(DrainedChild {
                            parent_type_id: TypeId::of::<E>(),
                            parent_entry_idx: entry_idx,
                            child: item,
                            child_type_id: nav.related_type_id,
                            fk_target_type_id: TypeId::of::<E>(),
                            through_table: nav.through_table.as_ref().map(|s| s.to_string()),
                            through_parent_fk_col: nav
                                .through_parent_fk
                                .as_ref()
                                .map(|s| s.to_string()),
                            through_child_fk_col: nav
                                .through_related_fk
                                .as_ref()
                                .map(|s| s.to_string()),
                        });
                    }
                }
            }
        }
        result
    }

    fn add_cascade_child(
        &self,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &mut ChangeTracker,
        child: Box<dyn Any + Send + Sync>,
    ) -> Option<usize> {
        let db_set = raw_set.downcast_mut::<DbSet<E>>()?;
        let child = child.downcast::<E>().ok()?;
        let pk: i64 = child
            .key_values()
            .iter()
            .next()
            .map(|(_, v)| v.clone())
            .and_then(|v| v.try_into().ok())
            .unwrap_or(0);
        let type_name = E::entity_meta().type_name.to_string();
        let (state, original) = if pk > 0 {
            (EntityState::Unchanged, Some(child.snapshot()))
        } else {
            (EntityState::Added, None)
        };
        let entry_id = tracker.track(TypeId::of::<E>(), &type_name, state, original, false);
        db_set.push_entry(*child, entry_id);
        Some(db_set.entries.len() - 1)
    }

    fn get_pk_at(&self, raw_set: &(dyn Any + Send + Sync), idx: usize) -> Option<i64> {
        let db_set = raw_set.downcast_ref::<DbSet<E>>()?;
        let entry = db_set.entries.get(idx)?;
        entry
            .entity
            .key_values()
            .iter()
            .next()
            .map(|(_, v)| v.clone())
            .and_then(|v| v.try_into().ok())
    }

    fn set_fk_at(
        &self,
        raw_set: &mut (dyn Any + Send + Sync),
        idx: usize,
        target_type: TypeId,
        key: i64,
    ) {
        if let Some(db_set) = raw_set.downcast_mut::<DbSet<E>>() {
            if let Some(entry) = db_set.entries.get_mut(idx) {
                entry.entity.set_foreign_key(target_type, key);
            }
        }
    }

    async fn insert_added(
        &self,
        conn: &mut (dyn IAsyncConnection + Send),
        provider: &dyn IDatabaseProvider,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &ChangeTracker,
        meta: &EntityTypeMeta,
    ) -> EFResult<usize> {
        let db_set = raw_set
            .downcast_mut::<DbSet<E>>()
            .ok_or_else(|| EFError::configuration("SetOps type mismatch in insert_added"))?;
        insert_added_phase(conn, provider, db_set, tracker, meta).await
    }

    async fn upsert_added(
        &self,
        conn: &mut (dyn IAsyncConnection + Send),
        provider: &dyn IDatabaseProvider,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &ChangeTracker,
        meta: &EntityTypeMeta,
    ) -> EFResult<usize> {
        let db_set = raw_set
            .downcast_mut::<DbSet<E>>()
            .ok_or_else(|| EFError::configuration("SetOps type mismatch in upsert_added"))?;
        upsert_added_phase(conn, provider, db_set, tracker, meta).await
    }

    async fn update_modified(
        &self,
        conn: &mut (dyn IAsyncConnection + Send),
        provider: &dyn IDatabaseProvider,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &ChangeTracker,
        meta: &EntityTypeMeta,
    ) -> EFResult<usize> {
        let db_set = raw_set
            .downcast_mut::<DbSet<E>>()
            .ok_or_else(|| EFError::configuration("SetOps type mismatch in update_modified"))?;
        let query_filter = db_set.query_filter().cloned();
        update_modified_phase(conn, provider, db_set, tracker, meta, query_filter.as_ref()).await
    }

    async fn delete_deleted(
        &self,
        conn: &mut (dyn IAsyncConnection + Send),
        provider: &dyn IDatabaseProvider,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &ChangeTracker,
        meta: &EntityTypeMeta,
    ) -> EFResult<usize> {
        let db_set = raw_set
            .downcast_mut::<DbSet<E>>()
            .ok_or_else(|| EFError::configuration("SetOps type mismatch in delete_deleted"))?;
        let query_filter = db_set.query_filter().cloned();
        delete_deleted_phase(conn, provider, db_set, tracker, meta, query_filter.as_ref()).await
    }

    fn drain_cascade_deleted_children(
        &self,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &ChangeTracker,
        meta: &EntityTypeMeta,
        processed: &mut std::collections::HashSet<(TypeId, usize)>,
    ) -> (Vec<DrainedChild>, Vec<CascadeDeleteDirective>) {
        let Some(db_set) = raw_set.downcast_mut::<DbSet<E>>() else {
            return (Vec::new(), Vec::new());
        };
        let mut drained_children = Vec::new();
        let mut directives = Vec::new();

        for (entry_idx, entry) in db_set.entries.iter_mut().enumerate() {
            if tracker.entry_state(entry.entry_id) != Some(EntityState::Deleted) {
                continue;
            }
            if !processed.insert((TypeId::of::<E>(), entry_idx)) {
                continue;
            }

            let principal_pk: i64 = entry
                .entity
                .key_values()
                .iter()
                .next()
                .map(|(_, v)| v.clone())
                .and_then(|v| v.try_into().ok())
                .unwrap_or(0);

            for nav in &meta.navigations {
                match nav.kind {
                    NavigationKind::ManyToMany => {
                        if let (Some(table), Some(fk_col), Some(pk)) = (
                            nav.through_table.as_ref(),
                            nav.through_parent_fk.as_ref(),
                            (principal_pk > 0).then_some(principal_pk),
                        ) {
                            directives.push(CascadeDeleteDirective {
                                table: table.to_string(),
                                fk_column: fk_col.to_string(),
                                principal_pk: pk,
                                action: CascadeDeleteAction::Delete,
                            });
                        }
                    }
                    NavigationKind::HasMany => {
                        let behavior = resolve_delete_behavior(nav);
                        match behavior {
                            DeleteBehavior::Cascade => {
                                if let Some(items) =
                                    entry.entity.drain_has_many(nav.field_name.as_ref())
                                {
                                    for item in items {
                                        drained_children.push(DrainedChild {
                                            parent_type_id: TypeId::of::<E>(),
                                            parent_entry_idx: entry_idx,
                                            child: item,
                                            child_type_id: nav.related_type_id,
                                            fk_target_type_id: TypeId::of::<E>(),
                                            through_table: None,
                                            through_parent_fk_col: None,
                                            through_child_fk_col: None,
                                        });
                                    }
                                }
                                if let (Some(table), Some(fk_col), Some(pk)) = (
                                    nav.related_table.as_ref(),
                                    nav.fk_column.as_ref(),
                                    (principal_pk > 0).then_some(principal_pk),
                                ) {
                                    directives.push(CascadeDeleteDirective {
                                        table: table.to_string(),
                                        fk_column: fk_col.to_string(),
                                        principal_pk: pk,
                                        action: CascadeDeleteAction::Delete,
                                    });
                                }
                            }
                            DeleteBehavior::SetNull => {
                                if let (Some(table), Some(fk_col), Some(pk)) = (
                                    nav.related_table.as_ref(),
                                    nav.fk_column.as_ref(),
                                    (principal_pk > 0).then_some(principal_pk),
                                ) {
                                    directives.push(CascadeDeleteDirective {
                                        table: table.to_string(),
                                        fk_column: fk_col.to_string(),
                                        principal_pk: pk,
                                        action: CascadeDeleteAction::SetNull,
                                    });
                                }
                            }
                            DeleteBehavior::Restrict | DeleteBehavior::NoAction => {}
                        }
                    }
                    NavigationKind::BelongsTo | NavigationKind::HasOne => {}
                }
            }
        }
        (drained_children, directives)
    }

    fn add_cascade_deleted_child(
        &self,
        raw_set: &mut (dyn Any + Send + Sync),
        tracker: &mut ChangeTracker,
        child: Box<dyn Any + Send + Sync>,
    ) -> Option<usize> {
        let db_set = raw_set.downcast_mut::<DbSet<E>>()?;
        let child = child.downcast::<E>().ok()?;
        let pk: i64 = child
            .key_values()
            .iter()
            .next()
            .map(|(_, v)| v.clone())
            .and_then(|v| v.try_into().ok())
            .unwrap_or(0);
        if pk == 0 {
            return None;
        }
        let original = child.snapshot();
        let type_name = E::entity_meta().type_name.to_string();
        let entry_id = tracker.track(
            TypeId::of::<E>(),
            &type_name,
            EntityState::Deleted,
            Some(original),
            false,
        );
        db_set.push_entry(*child, entry_id);
        Some(db_set.entries.len() - 1)
    }
}
