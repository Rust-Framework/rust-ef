//! Include (eager-load) and JOIN methods.

use crate::entity::IEntityType;

use super::super::ast::{IncludePath, JoinKey, JoinKind, JoinSpec};
use super::core::QueryBuilder;

impl<T: IEntityType> QueryBuilder<T> {
    /// Eagerly loads a named navigation (resolves FK/table from entity metadata).
    ///
    /// `#[doc(hidden)]` — called by `linq!(include b.posts)` expansion. Users
    /// should use the `linq!` macro instead of calling this directly.
    #[doc(hidden)]
    pub fn include_internal(mut self, navigation: &'static str) -> Self {
        let meta = T::entity_meta();
        let nav_meta = meta.find_navigation(navigation);
        let (related_table, fk_col, ref_col) = nav_meta
            .map(|n| {
                (
                    n.related_table.as_ref().map(|s| s.to_string()),
                    n.fk_column.as_ref().map(|s| s.to_string()),
                    n.referenced_key_column.as_ref().map(|s| s.to_string()),
                )
            })
            .unwrap_or((None, None, None));

        self.state.includes.push(IncludePath {
            navigation: navigation.to_string(),
            nested: Vec::new(),
            related_table,
            foreign_key_column: fk_col,
            referenced_key_column: ref_col,
        });
        self
    }

    /// Eagerly loads a nested navigation on the last `include_internal` path.
    ///
    /// `#[doc(hidden)]` — called by `linq!(include b.posts then b.comments)`
    /// expansion. The nested navigation field name is a string literal because
    /// the entity type transition is runtime knowledge (resolved via metadata).
    #[doc(hidden)]
    pub fn then_include_internal(mut self, navigation: &'static str) -> Self {
        if let Some(last) = self.state.includes.last_mut() {
            let parent_meta = T::entity_meta();
            if let Some(parent_nav) = parent_meta.find_navigation(&last.navigation) {
                if let Some(meta_fn) = parent_nav.related_entity_meta {
                    let related_meta = meta_fn();
                    if let Some(nav_meta) = related_meta.find_navigation(navigation) {
                        last.nested.push(IncludePath {
                            navigation: navigation.to_string(),
                            nested: Vec::new(),
                            related_table: nav_meta.related_table.as_ref().map(|s| s.to_string()),
                            foreign_key_column: nav_meta.fk_column.as_ref().map(|s| s.to_string()),
                            referenced_key_column: nav_meta
                                .referenced_key_column
                                .as_ref()
                                .map(|s| s.to_string()),
                        });
                    }
                }
            }
        }
        self
    }

    /// Adds a JOIN between the query's root (FROM) table and `J`'s table, with
    /// one or more equality keys.
    ///
    /// `on` is a slice of `(left_column, right_column)` pairs: the left column
    /// belongs to the query's FROM table, the right column to the joined `J`
    /// table. Multiple pairs form a composite ON joined with `AND`. For chained
    /// joins whose left side is a previously joined table, use
    /// [`join_from`](Self::join_from).
    ///
    /// The joined table is taken from `J::entity_meta().table_name`, so the
    /// table name always matches the entity type.
    ///
    /// ```ignore
    /// // ON orders.id = shipments.order_id AND orders.region_id = shipments.region_id
    /// let rows = ctx
    ///     .set::<Order>()
    ///     .query()
    ///     .join::<Shipment>(JoinKind::Inner, &[("id", "order_id"), ("region_id", "region_id")])
    ///     .to_list()
    ///     .await?;
    /// ```
    pub fn join<J: IEntityType>(mut self, kind: JoinKind, on: &[(&str, &str)]) -> Self {
        let left_table = self.state.from.clone();
        let table = J::entity_meta().table_name.to_string();
        self.push_join(kind, table, left_table, on);
        self
    }

    /// Adds a JOIN whose left side is `L`'s table instead of the root table.
    ///
    /// Used for chained joins: `join::<A>` then `join_from::<B, A>` produces
    /// `... JOIN b ON a.<left> = b.<right>`. As with [`join`](Self::join),
    /// table names are resolved from entity metadata.
    ///
    /// ```ignore
    /// // ON order_lines.order_id = shipments.order_id
    /// ctx.set::<OrderLine>().query()
    ///     .join_from::<Shipment, OrderLine>(JoinKind::Inner, &[("order_id", "order_id")])
    /// ```
    pub fn join_from<J: IEntityType, L: IEntityType>(
        mut self,
        kind: JoinKind,
        on: &[(&str, &str)],
    ) -> Self {
        let left_table = L::entity_meta().table_name.to_string();
        let table = J::entity_meta().table_name.to_string();
        self.push_join(kind, table, left_table, on);
        self
    }

    /// Adds a JOIN from entity table-name constants — the `linq!` macro entry
    /// point for the `inner_join` / `left_join` / `right_join` / `full_join`
    /// clauses.
    ///
    /// `left_table` is the table of the clause's first closure parameter (the
    /// table the new join continues from). When the query runs
    /// `from <cte>` (`state.from` is the CTE name instead of the root entity's
    /// table), the first join references the CTE — the same behaviour the
    /// legacy single-key `*_join_internal` methods have always had.
    #[doc(hidden)]
    pub fn join_internal(
        mut self,
        kind: JoinKind,
        left_table: &'static str,
        table: &'static str,
        keys: &'static [(&'static str, &'static str)],
    ) -> Self {
        let root_table = T::entity_meta().table_name.to_string();
        let effective_left = if left_table == root_table.as_str() && self.state.from != root_table {
            self.state.from.clone()
        } else {
            left_table.to_string()
        };
        let keys = keys
            .iter()
            .map(|(left_column, right_column)| JoinKey {
                left_table: effective_left.clone(),
                left_column: left_column.to_string(),
                right_column: right_column.to_string(),
            })
            .collect();
        self.state.joins.push(JoinSpec {
            kind,
            table: table.to_string(),
            keys,
        });
        self
    }

    /// Adds an INNER JOIN on a single equality key.
    ///
    /// `#[doc(hidden)]` — kept for compatibility; `linq!` now emits
    /// [`join_internal`](Self::join_internal) so composite keys and chained
    /// joins are supported. The generated SQL is unchanged
    /// (`<state.from>.<left_column> = <table>.<right_column>`).
    #[doc(hidden)]
    pub fn inner_join_internal(
        mut self,
        table: &'static str,
        left_column: &'static str,
        right_column: &'static str,
    ) -> Self {
        self.push_root_join(JoinKind::Inner, table, left_column, right_column);
        self
    }

    /// Adds a LEFT JOIN on a single equality key.
    ///
    /// `#[doc(hidden)]` — kept for compatibility; see
    /// [`inner_join_internal`](Self::inner_join_internal).
    #[doc(hidden)]
    pub fn left_join_internal(
        mut self,
        table: &'static str,
        left_column: &'static str,
        right_column: &'static str,
    ) -> Self {
        self.push_root_join(JoinKind::Left, table, left_column, right_column);
        self
    }

    /// Adds a RIGHT JOIN on a single equality key.
    ///
    /// `#[doc(hidden)]` — kept for compatibility; see
    /// [`inner_join_internal`](Self::inner_join_internal).
    #[doc(hidden)]
    pub fn right_join_internal(
        mut self,
        table: &'static str,
        left_column: &'static str,
        right_column: &'static str,
    ) -> Self {
        self.push_root_join(JoinKind::Right, table, left_column, right_column);
        self
    }

    /// Adds a FULL OUTER JOIN on a single equality key.
    ///
    /// `#[doc(hidden)]` — kept for compatibility; see
    /// [`inner_join_internal`](Self::inner_join_internal).
    ///
    /// Note: MySQL does not support FULL OUTER JOIN; the SQL will fail at
    /// execution time on MySQL providers.
    #[doc(hidden)]
    pub fn full_join_internal(
        mut self,
        table: &'static str,
        left_column: &'static str,
        right_column: &'static str,
    ) -> Self {
        self.push_root_join(JoinKind::Full, table, left_column, right_column);
        self
    }

    /// Adds a CROSS JOIN (no ON condition).
    ///
    /// `#[doc(hidden)]` — called by `linq!(cross_join b: T2)` expansion.
    #[doc(hidden)]
    pub fn cross_join_internal(mut self, table: &'static str) -> Self {
        self.state.joins.push(JoinSpec {
            kind: JoinKind::Cross,
            table: table.to_string(),
            keys: Vec::new(),
        });
        self
    }

    /// Pushes a JOIN whose keys all use `left_table`.
    fn push_join(
        &mut self,
        kind: JoinKind,
        table: String,
        left_table: String,
        on: &[(&str, &str)],
    ) {
        let keys = on
            .iter()
            .map(|(left_column, right_column)| JoinKey {
                left_table: left_table.clone(),
                left_column: left_column.to_string(),
                right_column: right_column.to_string(),
            })
            .collect();
        self.state.joins.push(JoinSpec { kind, table, keys });
    }

    /// Pushes a legacy single-key JOIN whose left table is the current FROM
    /// table (resolved at call time, exactly as the pre-2.7 code did).
    fn push_root_join(
        &mut self,
        kind: JoinKind,
        table: &'static str,
        left_column: &'static str,
        right_column: &'static str,
    ) {
        self.state.joins.push(JoinSpec {
            kind,
            table: table.to_string(),
            keys: vec![JoinKey {
                left_table: self.state.from.clone(),
                left_column: left_column.to_string(),
                right_column: right_column.to_string(),
            }],
        });
    }
}
