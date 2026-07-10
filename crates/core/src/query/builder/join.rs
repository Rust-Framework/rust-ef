//! Include (eager-load) and JOIN methods.

use crate::entity::IEntityType;

use super::super::ast::{IncludePath, JoinSpec};
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

    /// Adds an INNER JOIN.
    ///
    /// `#[doc(hidden)]` — called by `linq!(inner_join |a: T1, b: T2| a.col == b.col)`
    /// expansion.
    #[doc(hidden)]
    pub fn inner_join_internal(
        mut self,
        table: &'static str,
        left_column: &'static str,
        right_column: &'static str,
    ) -> Self {
        let on_clause = format!(
            "{}.{} = {}.{}",
            self.state.from, left_column, table, right_column
        );
        self.state.joins.push(JoinSpec {
            join_type: "INNER".to_string(),
            table: table.to_string(),
            on_clause,
        });
        self
    }

    /// Adds a LEFT JOIN.
    ///
    /// `#[doc(hidden)]` — called by `linq!(left_join |a: T1, b: T2| a.col == b.col)`
    /// expansion.
    #[doc(hidden)]
    pub fn left_join_internal(
        mut self,
        table: &'static str,
        left_column: &'static str,
        right_column: &'static str,
    ) -> Self {
        let on_clause = format!(
            "{}.{} = {}.{}",
            self.state.from, left_column, table, right_column
        );
        self.state.joins.push(JoinSpec {
            join_type: "LEFT".to_string(),
            table: table.to_string(),
            on_clause,
        });
        self
    }

    /// Adds a RIGHT JOIN.
    ///
    /// `#[doc(hidden)]` — called by `linq!(right_join |a: T1, b: T2| a.col == b.col)`
    /// expansion.
    #[doc(hidden)]
    pub fn right_join_internal(
        mut self,
        table: &'static str,
        left_column: &'static str,
        right_column: &'static str,
    ) -> Self {
        let on_clause = format!(
            "{}.{} = {}.{}",
            self.state.from, left_column, table, right_column
        );
        self.state.joins.push(JoinSpec {
            join_type: "RIGHT".to_string(),
            table: table.to_string(),
            on_clause,
        });
        self
    }

    /// Adds a FULL OUTER JOIN.
    ///
    /// `#[doc(hidden)]` — called by `linq!(full_join |a: T1, b: T2| a.col == b.col)`
    /// expansion.
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
        let on_clause = format!(
            "{}.{} = {}.{}",
            self.state.from, left_column, table, right_column
        );
        self.state.joins.push(JoinSpec {
            join_type: "FULL".to_string(),
            table: table.to_string(),
            on_clause,
        });
        self
    }

    /// Adds a CROSS JOIN (no ON condition).
    ///
    /// `#[doc(hidden)]` — called by `linq!(cross_join b: T2)` expansion.
    #[doc(hidden)]
    pub fn cross_join_internal(mut self, table: &'static str) -> Self {
        self.state.joins.push(JoinSpec {
            join_type: "CROSS".to_string(),
            table: table.to_string(),
            on_clause: String::new(),
        });
        self
    }
}
