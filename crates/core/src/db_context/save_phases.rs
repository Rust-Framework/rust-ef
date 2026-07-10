//! Save pipeline phase functions, cascade drain helpers, and result type.
//!
//! - `save_one_set` + 4 phase functions: per-DbSet DML execution
//! - `drain_cascade_adds` / `drain_cascade_deletes`: extracted from
//!   `save_changes` to keep `save_pipeline.rs` under 500 lines
//! - `SaveChangesResult`: save outcome summary

use crate::change_executor::ChangeExecutor;
use crate::db_set::DbSet;
use crate::entity::{EntityState, IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues};
use crate::error::{EFError, EFResult};
use crate::metadata::EntityTypeMeta;
use crate::provider::{DbValue, IAsyncConnection, IDatabaseProvider};
use std::any::TypeId;
use std::collections::HashMap;

use crate::cascade::{CascadeDeleteDirective, DrainedChild, FixupLink};

// ---------------------------------------------------------------------------
// Cascade drain helpers (extracted from save_changes)
// ---------------------------------------------------------------------------

impl super::DbContext {
    /// Iteratively drains HasMany/M2M children from Added principals.
    /// Drained children are added to their target DbSet as Added (if new) or
    /// attached as Unchanged (if existing). Repeats until no new children are
    /// extracted (handles arbitrary depth).
    pub(super) fn drain_cascade_adds(
        &mut self,
        type_ids: &[TypeId],
        configured_metas: &HashMap<TypeId, EntityTypeMeta>,
    ) -> EFResult<Vec<FixupLink>> {
        let mut fixup_links: Vec<FixupLink> = Vec::new();
        loop {
            let mut all_drained: Vec<DrainedChild> = Vec::new();
            for type_id in type_ids {
                let Some(saver) = self.savers.get(type_id) else {
                    continue;
                };
                let Some(set) = self.sets.get_mut(type_id) else {
                    continue;
                };
                let meta = configured_metas
                    .get(type_id)
                    .or_else(|| self.entity_metas.get(type_id))
                    .ok_or_else(|| {
                        EFError::configuration(format!(
                            "entity metadata not found for {:?}",
                            type_id
                        ))
                    })?;
                all_drained.extend(saver.drain_cascade_children(set.as_mut(), meta));
            }
            if all_drained.is_empty() {
                break;
            }
            for child in all_drained {
                let child_saver = self.savers.get(&child.child_type_id).ok_or_else(|| {
                    EFError::configuration(format!(
                        "Cannot cascade-save child type {:?}: no DbSet registered. \
                         Call ctx.set::<ChildType>() before save_changes.",
                        child.child_type_id
                    ))
                })?;
                let child_set = self.sets.get_mut(&child.child_type_id).ok_or_else(|| {
                    EFError::configuration(format!(
                        "DbSet not found for registered saver type {:?}",
                        child.child_type_id
                    ))
                })?;
                if let Some(child_idx) =
                    child_saver.add_cascade_child(child_set.as_mut(), child.child)
                {
                    if let Some(link) = fixup_links.iter_mut().find(|l| {
                        l.parent_type_id == child.parent_type_id
                            && l.parent_entry_idx == child.parent_entry_idx
                            && l.child_type_id == child.child_type_id
                            && l.through_table == child.through_table
                    }) {
                        link.child_entry_indices.push(child_idx);
                    } else {
                        fixup_links.push(FixupLink {
                            parent_type_id: child.parent_type_id,
                            parent_entry_idx: child.parent_entry_idx,
                            child_type_id: child.child_type_id,
                            child_entry_indices: vec![child_idx],
                            fk_target_type_id: child.fk_target_type_id,
                            through_table: child.through_table,
                            through_parent_fk_col: child.through_parent_fk_col,
                            through_child_fk_col: child.through_child_fk_col,
                        });
                    }
                }
            }
        }
        Ok(fixup_links)
    }

    /// Iteratively drains HasMany children from Deleted principals and collects
    /// direct DELETE/SET NULL directives for untracked dependents.
    pub(super) fn drain_cascade_deletes(
        &mut self,
        type_ids: &[TypeId],
        configured_metas: &HashMap<TypeId, EntityTypeMeta>,
        processed: &mut std::collections::HashSet<(TypeId, usize)>,
    ) -> EFResult<Vec<CascadeDeleteDirective>> {
        let mut delete_directives: Vec<CascadeDeleteDirective> = Vec::new();
        loop {
            let mut all_drained_deleted: Vec<DrainedChild> = Vec::new();
            for type_id in type_ids {
                let Some(saver) = self.savers.get(type_id) else {
                    continue;
                };
                let Some(set) = self.sets.get_mut(type_id) else {
                    continue;
                };
                let meta = configured_metas
                    .get(type_id)
                    .or_else(|| self.entity_metas.get(type_id))
                    .ok_or_else(|| {
                        EFError::configuration(format!(
                            "entity metadata not found for {:?}",
                            type_id
                        ))
                    })?;
                let (drained, directives) =
                    saver.drain_cascade_deleted_children(set.as_mut(), meta, processed);
                all_drained_deleted.extend(drained);
                delete_directives.extend(directives);
            }
            if all_drained_deleted.is_empty() {
                break;
            }
            for child in all_drained_deleted {
                let child_saver = self.savers.get(&child.child_type_id).ok_or_else(|| {
                    EFError::configuration(format!(
                        "Cannot cascade-delete child type {:?}: no DbSet registered. \
                         Call ctx.set::<ChildType>() before save_changes.",
                        child.child_type_id
                    ))
                })?;
                let child_set = self.sets.get_mut(&child.child_type_id).ok_or_else(|| {
                    EFError::configuration(format!(
                        "DbSet not found for registered saver type {:?}",
                        child.child_type_id
                    ))
                })?;
                child_saver.add_cascade_deleted_child(child_set.as_mut(), child.child);
            }
        }
        Ok(delete_directives)
    }
}

// ---------------------------------------------------------------------------
// Per-DbSet phase functions
// ---------------------------------------------------------------------------

#[allow(clippy::type_complexity)]
pub async fn save_one_set<E>(
    conn: &mut dyn IAsyncConnection,
    provider: &dyn IDatabaseProvider,
    db_set: &mut DbSet<E>,
    meta: &EntityTypeMeta,
) -> EFResult<(usize, usize, usize)>
where
    E: IEntityType + IEntitySnapshot + IGetKeyValues + IFromRow,
{
    let query_filter = db_set.query_filter().cloned();
    let ac = insert_added_phase(conn, provider, db_set, meta).await?;
    let ac_upsert = upsert_added_phase(conn, provider, db_set, meta).await?;
    let uc = update_modified_phase(conn, provider, db_set, meta, query_filter.as_ref()).await?;
    let dc = delete_deleted_phase(conn, provider, db_set, meta, query_filter.as_ref()).await?;
    Ok((ac + ac_upsert, uc, dc))
}

/// Phase 1a: INSERT Added (non-upsert) entities, then backfill generated PKs.
pub async fn insert_added_phase<E>(
    conn: &mut dyn IAsyncConnection,
    provider: &dyn IDatabaseProvider,
    db_set: &mut DbSet<E>,
    meta: &EntityTypeMeta,
) -> EFResult<usize>
where
    E: IEntityType + IEntitySnapshot + IGetKeyValues,
{
    let added: Vec<(&E, &EntityTypeMeta)> = db_set
        .tracked_by_state(EntityState::Added)
        .into_iter()
        .filter(|(_, _, _, is_upsert)| !*is_upsert)
        .map(|(e, _, _, _)| (e, meta))
        .collect();
    if added.is_empty() {
        return Ok(0);
    }
    let added_count = added.len();
    let mut generated_keys: Vec<i64> = vec![0; added_count];
    let inserted = ChangeExecutor::execute_inserts(conn, provider, &added, |idx, key| {
        if idx < generated_keys.len() {
            generated_keys[idx] = key;
        }
    })
    .await?;
    db_set.backfill_added_keys(&generated_keys);
    Ok(inserted)
}

/// Phase 1b: UPSERT Added entities (is_upsert = true).
pub async fn upsert_added_phase<E>(
    conn: &mut dyn IAsyncConnection,
    provider: &dyn IDatabaseProvider,
    db_set: &mut DbSet<E>,
    meta: &EntityTypeMeta,
) -> EFResult<usize>
where
    E: IEntityType + IEntitySnapshot + IGetKeyValues,
{
    let upserts: Vec<(&E, &EntityTypeMeta)> = db_set
        .tracked_by_state(EntityState::Added)
        .into_iter()
        .filter(|(_, _, _, is_upsert)| *is_upsert)
        .map(|(e, _, _, _)| (e, meta))
        .collect();
    if upserts.is_empty() {
        return Ok(0);
    }
    ChangeExecutor::execute_upserts(conn, provider, &upserts).await
}

/// Phase 2: UPDATE Modified entities (partial update via modified_properties).
#[allow(clippy::type_complexity)]
pub async fn update_modified_phase<E>(
    conn: &mut dyn IAsyncConnection,
    provider: &dyn IDatabaseProvider,
    db_set: &mut DbSet<E>,
    meta: &EntityTypeMeta,
    query_filter: Option<&crate::query::BoolExpr>,
) -> EFResult<usize>
where
    E: IEntityType + IEntitySnapshot + IGetKeyValues,
{
    let modified: Vec<(
        &E,
        &EntityTypeMeta,
        Option<&HashMap<String, DbValue>>,
        &[String],
    )> = db_set
        .tracked_by_state(EntityState::Modified)
        .into_iter()
        .map(|(e, orig, mods, _)| (e, meta, orig, mods))
        .collect();
    if modified.is_empty() {
        return Ok(0);
    }
    ChangeExecutor::execute_updates(conn, provider, &modified, query_filter).await
}

/// Phase 3: DELETE Deleted entities.
#[allow(clippy::type_complexity)]
pub async fn delete_deleted_phase<E>(
    conn: &mut dyn IAsyncConnection,
    provider: &dyn IDatabaseProvider,
    db_set: &mut DbSet<E>,
    meta: &EntityTypeMeta,
    query_filter: Option<&crate::query::BoolExpr>,
) -> EFResult<usize>
where
    E: IEntityType + IEntitySnapshot + IGetKeyValues,
{
    let deleted: Vec<(&E, &EntityTypeMeta, Option<&HashMap<String, DbValue>>)> = db_set
        .tracked_by_state(EntityState::Deleted)
        .into_iter()
        .map(|(e, orig, _, _)| (e, meta, orig))
        .collect();
    if deleted.is_empty() {
        return Ok(0);
    }
    ChangeExecutor::execute_deletes(conn, provider, &deleted, query_filter).await
}

// ---------------------------------------------------------------------------
// SaveChangesResult
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SaveChangesResult {
    pub added: usize,
    pub updated: usize,
    pub deleted: usize,
}
impl SaveChangesResult {
    pub fn total(&self) -> usize {
        self.added + self.updated + self.deleted
    }
}
impl std::fmt::Display for SaveChangesResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} entities modified ({} added, {} updated, {} deleted)",
            self.total(),
            self.added,
            self.updated,
            self.deleted
        )
    }
}
