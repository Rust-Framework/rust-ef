//! `DbContext::save_changes` — the unit-of-work commit pipeline.
//!
//! Orchestrates change detection, interceptor hooks, cascade drain,
//! topological DML execution, and transaction management.

use crate::cascade::{self, CascadeDeleteAction};
use crate::dependency_graph::DependencyGraph;
use crate::error::{EFError, EFResult};
use crate::interceptor::{SaveChangesContext, SaveChangesResultContext};
use crate::metadata::EntityTypeMeta;
use crate::provider::{DbValue, IAsyncConnection};
use crate::transaction::ITransaction;
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use super::save_phases::SaveChangesResult;

/// Transaction source for `save_changes`: either the ambient transaction
/// registered by `use_transaction` / `transaction`, or a self-managed
/// connection.
pub(super) enum TxnSource {
    /// Ambient transaction (`use_transaction` / `transaction`).
    Ambient(Box<dyn ITransaction>),
    /// Self-managed transaction (no explicit transaction around the save).
    Managed(Box<dyn IAsyncConnection>),
}

impl TxnSource {
    fn conn(&mut self) -> &mut dyn IAsyncConnection {
        match self {
            TxnSource::Ambient(t) => t.connection(),
            TxnSource::Managed(c) => c.as_mut(),
        }
    }

    /// Only context-scoped transactions get per-save savepoints. A legacy
    /// standalone handle (`begin_transaction`) does not cover queries or
    /// `save_changes`, so it must not affect save semantics.
    fn is_scoped(&self) -> bool {
        matches!(self, TxnSource::Ambient(t) if t.is_scoped())
    }
}

/// Savepoints are per-save unique; a process-wide counter keeps names distinct
/// across nested / concurrent sessions without allocating UUIDs.
static SAVEPOINT_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_savepoint_name() -> String {
    format!(
        "rust_ef_save_{}",
        SAVEPOINT_SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

impl super::DbContext {
    /// Builds the interceptor `SaveChangesContext` from the actual pending
    /// entries across all `DbSet`s (the real save data source), instead of
    /// the legacy `change_tracker` which is never populated by `DbSet::add`.
    /// This keeps interceptor snapshots consistent with what will be committed.
    fn build_save_context(&self) -> SaveChangesContext {
        let mut views: Vec<crate::tracking::EntityEntryView> = Vec::new();
        for (type_id, set) in &self.sets {
            if let Some(saver) = self.savers.get(type_id) {
                views.extend(saver.collect_entries(set.as_ref(), &self.change_tracker));
            }
        }
        SaveChangesContext::from_views(views)
    }

    /// Saves all pending changes across all DbSets.
    ///
    /// Detects changes, runs interceptors, executes INSERT/UPDATE/DELETE in a
    /// transaction, and clears tracked entries on success.
    pub async fn save_changes(&mut self) -> EFResult<SaveChangesResult> {
        let _save_guard = crate::observability::SaveChangesGuard::new();
        let type_ids: Vec<TypeId> = self.sets.keys().copied().collect();
        for type_id in &type_ids {
            let Some(set) = self.sets.get_mut(type_id) else {
                continue;
            };
            let Some(saver) = self.savers.get(type_id) else {
                continue;
            };
            saver.detect_changes(set.as_mut(), &mut self.change_tracker);
        }

        let configured_metas: HashMap<TypeId, EntityTypeMeta> = self
            .model_builder
            .build()
            .into_iter()
            .map(|m| (m.type_id, m))
            .collect();

        let save_ctx = self.build_save_context();
        self.interceptor_pipeline.on_saving(&save_ctx).await?;

        // No-op saves should not open a transaction (or a savepoint): EF Core
        // returns without touching the database when nothing is pending.
        if save_ctx.added_count() == 0
            && save_ctx.modified_count() == 0
            && save_ctx.deleted_count() == 0
        {
            let result_ctx = SaveChangesResultContext {
                added: 0,
                updated: 0,
                deleted: 0,
            };
            self.interceptor_pipeline
                .on_saved(&save_ctx, &result_ctx)
                .await?;
            return Ok(SaveChangesResult {
                added: 0,
                updated: 0,
                deleted: 0,
            });
        }

        // Note: unlike EF Core's `AutoTransactionBehavior.WhenNeeded`, a
        // self-managed save always opens a transaction. A single-statement
        // fast path would need a complete statement plan (cascades, FK fixups,
        // M2M join rows) before execution; an incorrect count would commit
        // partially, so it is intentionally not attempted here.
        let mut txn = match self.ambient_transaction.take() {
            Some(t) => TxnSource::Ambient(t),
            None => {
                let mut c = self.provider.get_connection().await?;
                c.begin_transaction().await?;
                TxnSource::Managed(c)
            }
        };

        // Context-scoped transactions get a savepoint so a failed save can be
        // rolled back without discarding work already done in the same
        // transaction (EF Core does the same around SaveChanges).
        let scoped = txn.is_scoped();
        let savepoint = if scoped {
            let name = next_savepoint_name();
            if let Err(e) = txn.conn().create_savepoint(&name).await {
                return self.fail_save(txn, &save_ctx, None, e).await;
            }
            Some(name)
        } else {
            None
        };

        let type_ids: Vec<TypeId> = self.sets.keys().copied().collect();

        // --- Cascade drain: Added + Deleted ---
        let fixup_links = match self.drain_cascade_adds(&type_ids, &configured_metas) {
            Ok(links) => links,
            Err(e) => {
                return self
                    .fail_save(txn, &save_ctx, savepoint.as_deref(), e)
                    .await
            }
        };
        let mut processed: std::collections::HashSet<(TypeId, usize)> =
            std::collections::HashSet::new();
        let delete_directives =
            match self.drain_cascade_deletes(&type_ids, &configured_metas, &mut processed) {
                Ok(directives) => directives,
                Err(e) => {
                    return self
                        .fail_save(txn, &save_ctx, savepoint.as_deref(), e)
                        .await
                }
            };

        let graph = DependencyGraph::build(&configured_metas);
        let insert_order = graph.topological_sort();
        let delete_order = graph.deletion_order();

        let mut total_added = 0usize;
        let mut total_updated = 0usize;
        let mut total_deleted = 0usize;

        // --- INSERT phase (topological order) + FK fixup ---
        for type_id in &insert_order {
            if !self.sets.contains_key(type_id) || !self.savers.contains_key(type_id) {
                continue;
            }
            let Some(saver) = self.savers.get(type_id) else {
                continue;
            };
            let Some(set) = self.sets.get_mut(type_id) else {
                continue;
            };
            let Some(meta) = configured_metas
                .get(type_id)
                .or_else(|| self.entity_metas.get(type_id))
            else {
                return self
                    .fail_save(
                        txn,
                        &save_ctx,
                        savepoint.as_deref(),
                        EFError::configuration(format!(
                            "entity metadata not found for {:?}",
                            type_id
                        )),
                    )
                    .await;
            };
            let inserted = match saver
                .insert_added(
                    txn.conn(),
                    &*self.provider,
                    set.as_mut(),
                    &self.change_tracker,
                    meta,
                )
                .await
            {
                Ok(n) => n,
                Err(e) => {
                    return self
                        .fail_save(txn, &save_ctx, savepoint.as_deref(), e)
                        .await
                }
            };
            total_added += inserted;

            let link_indices: Vec<usize> = fixup_links
                .iter()
                .enumerate()
                .filter(|(_, l)| l.parent_type_id == *type_id && l.through_table.is_none())
                .map(|(i, _)| i)
                .collect();

            let mut self_ref_updates: Vec<(String, i64, i64)> = Vec::new();

            for link_idx in &link_indices {
                let link = &fixup_links[*link_idx];
                let parent_pk = {
                    let Some(parent_saver) = self.savers.get(&link.parent_type_id) else {
                        continue;
                    };
                    let Some(parent_set) = self.sets.get(&link.parent_type_id) else {
                        continue;
                    };
                    parent_saver.get_pk_at(parent_set.as_ref(), link.parent_entry_idx)
                };
                let Some(pk) = parent_pk else {
                    continue;
                };

                {
                    let Some(child_saver) = self.savers.get(&link.child_type_id) else {
                        continue;
                    };
                    let Some(child_set) = self.sets.get_mut(&link.child_type_id) else {
                        continue;
                    };
                    for &child_idx in &link.child_entry_indices {
                        child_saver.set_fk_at(
                            child_set.as_mut(),
                            child_idx,
                            link.fk_target_type_id,
                            pk,
                        );
                    }
                }

                if link.child_type_id == link.parent_type_id {
                    let Some(child_meta) = configured_metas
                        .get(&link.child_type_id)
                        .or_else(|| self.entity_metas.get(&link.child_type_id))
                    else {
                        continue;
                    };
                    let fk_col = child_meta
                        .properties
                        .iter()
                        .find(|p| p.is_foreign_key)
                        .map(|p| p.column_name.as_ref());
                    let pk_col = child_meta
                        .properties
                        .iter()
                        .find(|p| p.is_primary_key)
                        .map(|p| p.column_name.as_ref())
                        .unwrap_or("id");
                    if let Some(fk_col) = fk_col {
                        for &child_idx in &link.child_entry_indices {
                            let child_pk = {
                                let Some(child_saver) = self.savers.get(&link.child_type_id) else {
                                    continue;
                                };
                                let Some(child_set) = self.sets.get(&link.child_type_id) else {
                                    continue;
                                };
                                child_saver.get_pk_at(child_set.as_ref(), child_idx)
                            };
                            if let Some(child_pk) = child_pk {
                                let sql = format!(
                                    "UPDATE {} SET {} = ? WHERE {} = ?",
                                    child_meta.table_name, fk_col, pk_col
                                );
                                self_ref_updates.push((sql, pk, child_pk));
                            }
                        }
                    }
                }
            }

            for (sql, fk_val, pk_val) in self_ref_updates {
                if let Err(e) = txn
                    .conn()
                    .execute(&sql, &[DbValue::from(fk_val), DbValue::from(pk_val)])
                    .await
                {
                    return self
                        .fail_save(txn, &save_ctx, savepoint.as_deref(), e)
                        .await;
                }
            }
        }

        // --- M2M join row insertion (after all entity INSERTs) ---
        for link in &fixup_links {
            if link.through_table.is_none() {
                continue;
            }
            let (Some(table), Some(parent_col), Some(child_col)) = (
                link.through_table.as_ref(),
                link.through_parent_fk_col.as_ref(),
                link.through_child_fk_col.as_ref(),
            ) else {
                continue;
            };

            let parent_pk = {
                let Some(parent_saver) = self.savers.get(&link.parent_type_id) else {
                    continue;
                };
                let Some(parent_set) = self.sets.get(&link.parent_type_id) else {
                    continue;
                };
                parent_saver.get_pk_at(parent_set.as_ref(), link.parent_entry_idx)
            };
            let Some(parent_pk) = parent_pk else {
                continue;
            };

            let mut child_pks: Vec<i64> = Vec::new();
            {
                let Some(child_saver) = self.savers.get(&link.child_type_id) else {
                    continue;
                };
                let Some(child_set) = self.sets.get(&link.child_type_id) else {
                    continue;
                };
                for &child_idx in &link.child_entry_indices {
                    if let Some(child_pk) = child_saver.get_pk_at(child_set.as_ref(), child_idx) {
                        child_pks.push(child_pk);
                    }
                }
            }

            if !child_pks.is_empty() {
                let sql = cascade::m2m_insert_sql(table, parent_col, child_col, child_pks.len());
                let mut params: Vec<DbValue> = Vec::with_capacity(child_pks.len() * 2);
                for child_pk in &child_pks {
                    params.push(DbValue::from(parent_pk));
                    params.push(DbValue::from(*child_pk));
                }
                if let Err(e) = txn.conn().execute(&sql, &params).await {
                    return self
                        .fail_save(txn, &save_ctx, savepoint.as_deref(), e)
                        .await;
                }
                total_added += child_pks.len();
            }
        }

        // --- UPSERT phase ---
        for type_id in &insert_order {
            if !self.sets.contains_key(type_id) || !self.savers.contains_key(type_id) {
                continue;
            }
            let Some(saver) = self.savers.get(type_id) else {
                continue;
            };
            let Some(set) = self.sets.get_mut(type_id) else {
                continue;
            };
            let Some(meta) = configured_metas
                .get(type_id)
                .or_else(|| self.entity_metas.get(type_id))
            else {
                return self
                    .fail_save(
                        txn,
                        &save_ctx,
                        savepoint.as_deref(),
                        EFError::configuration(format!(
                            "entity metadata not found for {:?}",
                            type_id
                        )),
                    )
                    .await;
            };
            let n = match saver
                .upsert_added(
                    txn.conn(),
                    &*self.provider,
                    set.as_mut(),
                    &self.change_tracker,
                    meta,
                )
                .await
            {
                Ok(n) => n,
                Err(e) => {
                    return self
                        .fail_save(txn, &save_ctx, savepoint.as_deref(), e)
                        .await
                }
            };
            total_added += n;
        }

        // --- UPDATE phase ---
        for type_id in &insert_order {
            if !self.sets.contains_key(type_id) || !self.savers.contains_key(type_id) {
                continue;
            }
            let Some(saver) = self.savers.get(type_id) else {
                continue;
            };
            let Some(set) = self.sets.get_mut(type_id) else {
                continue;
            };
            let Some(meta) = configured_metas
                .get(type_id)
                .or_else(|| self.entity_metas.get(type_id))
            else {
                return self
                    .fail_save(
                        txn,
                        &save_ctx,
                        savepoint.as_deref(),
                        EFError::configuration(format!(
                            "entity metadata not found for {:?}",
                            type_id
                        )),
                    )
                    .await;
            };
            let n = match saver
                .update_modified(
                    txn.conn(),
                    &*self.provider,
                    set.as_mut(),
                    &self.change_tracker,
                    meta,
                )
                .await
            {
                Ok(n) => n,
                Err(e) => {
                    return self
                        .fail_save(txn, &save_ctx, savepoint.as_deref(), e)
                        .await
                }
            };
            total_updated += n;
        }

        // --- Direct cascade SET NULL SQL (before PK-based deletes) ---
        for directive in &delete_directives {
            if directive.action != CascadeDeleteAction::SetNull {
                continue;
            }
            let sql = format!(
                "UPDATE {} SET {} = NULL WHERE {} = ?",
                directive.table, directive.fk_column, directive.fk_column
            );
            let params = vec![DbValue::from(directive.principal_pk)];
            if let Err(e) = txn.conn().execute(&sql, &params).await {
                return self
                    .fail_save(txn, &save_ctx, savepoint.as_deref(), e)
                    .await;
            }
        }

        // --- DELETE phase (reverse topological order: dependents first) ---
        for type_id in &delete_order {
            if !self.sets.contains_key(type_id) || !self.savers.contains_key(type_id) {
                continue;
            }
            let Some(saver) = self.savers.get(type_id) else {
                continue;
            };
            let Some(set) = self.sets.get_mut(type_id) else {
                continue;
            };
            let Some(meta) = configured_metas
                .get(type_id)
                .or_else(|| self.entity_metas.get(type_id))
            else {
                return self
                    .fail_save(
                        txn,
                        &save_ctx,
                        savepoint.as_deref(),
                        EFError::configuration(format!(
                            "entity metadata not found for {:?}",
                            type_id
                        )),
                    )
                    .await;
            };
            let n = match saver
                .delete_deleted(
                    txn.conn(),
                    &*self.provider,
                    set.as_mut(),
                    &self.change_tracker,
                    meta,
                )
                .await
            {
                Ok(n) => n,
                Err(e) => {
                    return self
                        .fail_save(txn, &save_ctx, savepoint.as_deref(), e)
                        .await
                }
            };
            total_deleted += n;
        }

        // --- Direct cascade DELETE SQL (after PK-based deletes) ---
        for directive in &delete_directives {
            if directive.action != CascadeDeleteAction::Delete {
                continue;
            }
            let sql = format!(
                "DELETE FROM {} WHERE {} = ?",
                directive.table, directive.fk_column
            );
            let params = vec![DbValue::from(directive.principal_pk)];
            if let Err(e) = txn.conn().execute(&sql, &params).await {
                return self
                    .fail_save(txn, &save_ctx, savepoint.as_deref(), e)
                    .await;
            }
        }

        // --- Commit ---
        if let Some(name) = savepoint.as_deref() {
            // The DML already succeeded; a failed RELEASE leaves the savepoint
            // in place but does not invalidate the transaction, so don't fail
            // the save because of it.
            let _ = txn.conn().release_savepoint(name).await;
        }
        match txn {
            TxnSource::Ambient(t) => {
                self.ambient_transaction = Some(t);
            }
            TxnSource::Managed(mut conn) => {
                if let Err(e) = conn.commit_transaction().await {
                    self.interceptor_pipeline
                        .on_save_failed(&save_ctx, &e)
                        .await;
                    return Err(e);
                }
            }
        }
        // --- Accept changes: refresh tracker originals, drop Deleted entries ---
        for type_id in &type_ids {
            let Some(saver) = self.savers.get(type_id) else {
                continue;
            };
            let Some(set) = self.sets.get_mut(type_id) else {
                continue;
            };
            saver.accept_all_changes(set.as_mut(), &mut self.change_tracker);
        }

        let result_ctx = SaveChangesResultContext {
            added: total_added,
            updated: total_updated,
            deleted: total_deleted,
        };
        self.interceptor_pipeline
            .on_saved(&save_ctx, &result_ctx)
            .await?;

        Ok(SaveChangesResult {
            added: total_added,
            updated: total_updated,
            deleted: total_deleted,
        })
    }

    /// Error recovery: roll back to the savepoint for caller-managed
    /// transactions, roll back the whole managed transaction otherwise, fire
    /// `on_save_failed`, and return the error.
    async fn fail_save(
        &mut self,
        txn: TxnSource,
        save_ctx: &SaveChangesContext,
        savepoint: Option<&str>,
        e: EFError,
    ) -> EFResult<SaveChangesResult> {
        match txn {
            TxnSource::Managed(mut conn) => {
                let _ = conn.rollback_transaction().await;
            }
            TxnSource::Ambient(mut t) => {
                if let Some(name) = savepoint {
                    let _ = t.connection().rollback_to_savepoint(name).await;
                    let _ = t.connection().release_savepoint(name).await;
                }
                self.ambient_transaction = Some(t);
            }
        }
        self.interceptor_pipeline.on_save_failed(save_ctx, &e).await;
        Err(e)
    }
}
