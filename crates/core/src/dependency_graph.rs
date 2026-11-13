//! Dependency graph and topological sort — determines entity save order.
//!
//! Principals (independent entities) are ordered before dependents (entities
//! with foreign keys pointing at principals). Delete order is the reverse.
//!
//! Self-referential relationships (a type with a HasMany pointing at itself)
//! do not affect type-level ordering — instance-level ordering is handled by
//! the cascade drain + FK fixup pipeline.

use crate::metadata::{EntityTypeMeta, NavigationKind};
use std::any::TypeId;
use std::collections::{HashMap, HashSet, VecDeque};

pub struct DependencyGraph {
    /// child_type_id → list of principal type_ids it depends on.
    edges: HashMap<TypeId, Vec<TypeId>>,
    nodes: Vec<TypeId>,
}

impl DependencyGraph {
    /// Builds the graph from entity metadata.
    ///
    /// Edges (child → principal it depends on) come from three sources:
    ///
    /// - `HasMany` / `ManyToMany` navigations: `related_type_id` (child)
    ///   depends on `type_id` (principal).
    /// - `BelongsTo` navigations (EF-16): the declaring entity is the child, so
    ///   the edge is reversed — `type_id` depends on `related_type_id`.
    /// - Foreign keys (EF-16): the referenced table is reverse-looked-up in
    ///   `metas` to find the principal's `TypeId`. This covers scalar
    ///   `#[foreign_key(Target)]` columns without a navigation, as well as
    ///   composite keys declared with `has_foreign_key`. Views and keyless
    ///   entities are not tables and are excluded from the lookup.
    ///
    /// Edges are deduplicated: a child that declares the relationship twice
    /// (`BelongsTo` navigation *and* the scalar FK attribute resolving to the
    /// same principal) must produce a single edge, otherwise `in_degree` and
    /// the decrement count in [`Self::topological_sort`] disagree and the sort
    /// silently falls back to metadata order.
    pub fn build(metas: &HashMap<TypeId, EntityTypeMeta>) -> Self {
        let mut edges: HashMap<TypeId, Vec<TypeId>> = HashMap::new();
        let mut nodes: Vec<TypeId> = Vec::new();

        // Reverse lookup: table name → entity type. Only unambiguous tables are
        // used, and views/keyless entities are skipped (they are not
        // insertable principals). Duplicate table mappings (projection types
        // sharing a table, views, …) are ambiguous and therefore ignored.
        let mut table_owner: HashMap<&str, TypeId> = HashMap::new();
        let mut ambiguous: HashSet<&str> = HashSet::new();
        for (type_id, meta) in metas {
            if meta.is_view || meta.is_keyless {
                continue;
            }
            let table = meta.table_name.as_ref();
            if table_owner.insert(table, *type_id).is_some() {
                ambiguous.insert(table);
            }
        }
        for table in ambiguous {
            table_owner.remove(table);
        }

        let mut seen: HashSet<(TypeId, TypeId)> = HashSet::new();
        for (type_id, meta) in metas {
            nodes.push(*type_id);
            for nav in &meta.navigations {
                match nav.kind {
                    NavigationKind::HasMany | NavigationKind::ManyToMany => {
                        push_edge(&mut edges, &mut seen, nav.related_type_id, *type_id);
                    }
                    NavigationKind::BelongsTo => {
                        push_edge(&mut edges, &mut seen, *type_id, nav.related_type_id);
                    }
                    // `HasOne` carries the FK on the *other* side; it is not
                    // enough to derive a type-level dependency.
                    NavigationKind::HasOne => {}
                }
            }
            for fk in &meta.foreign_keys {
                if let Some(principal) = table_owner.get(fk.referenced_table.as_ref()) {
                    push_edge(&mut edges, &mut seen, *type_id, *principal);
                }
            }
        }
        Self { edges, nodes }
    }

    /// Kahn's algorithm topological sort: principals first, dependents after.
    /// Self-edges (type depends on itself) are excluded from in-degree
    /// counting — same-type instance ordering is handled by the cascade
    /// drain + fixup pipeline.
    pub fn topological_sort(&self) -> Vec<TypeId> {
        let mut in_degree: HashMap<TypeId, usize> = HashMap::new();
        for node in &self.nodes {
            in_degree.entry(*node).or_insert(0);
        }
        for (child, parents) in &self.edges {
            let count = parents.iter().filter(|p| **p != *child).count();
            *in_degree.entry(*child).or_insert(0) += count;
        }

        let mut queue: VecDeque<TypeId> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&k, _)| k)
            .collect();
        let mut result: Vec<TypeId> = Vec::new();
        while let Some(node) = queue.pop_front() {
            result.push(node);
            for (child, parents) in &self.edges {
                if parents.iter().any(|p| *p == node && *p != *child) {
                    let deg = in_degree.entry(*child).or_insert(0);
                    if *deg > 0 {
                        *deg -= 1;
                    }
                    if *deg == 0 && !result.contains(child) && !queue.contains(child) {
                        queue.push_back(*child);
                    }
                }
            }
        }
        for node in &self.nodes {
            if !result.contains(node) {
                result.push(*node);
            }
        }
        result
    }

    /// Deletion order: reverse topological (dependents before principals).
    pub fn deletion_order(&self) -> Vec<TypeId> {
        let mut order = self.topological_sort();
        order.reverse();
        order
    }
}

/// Records one dependency edge (child → principal), ignoring duplicates.
fn push_edge(
    edges: &mut HashMap<TypeId, Vec<TypeId>>,
    seen: &mut HashSet<(TypeId, TypeId)>,
    child: TypeId,
    principal: TypeId,
) {
    if seen.insert((child, principal)) {
        edges.entry(child).or_default().push(principal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{ForeignKeyMeta, NavigationKind, NavigationMeta};
    use std::borrow::Cow;

    // Distinct marker types give distinct `TypeId`s without needing entities.
    struct Principal;
    struct Child;
    struct GrandChild;
    struct Unrelated;

    fn principal() -> TypeId {
        TypeId::of::<Principal>()
    }
    fn child() -> TypeId {
        TypeId::of::<Child>()
    }
    fn grand_child() -> TypeId {
        TypeId::of::<GrandChild>()
    }
    fn unrelated() -> TypeId {
        TypeId::of::<Unrelated>()
    }

    fn meta(type_id: TypeId, table: &'static str) -> EntityTypeMeta {
        EntityTypeMeta {
            type_id,
            type_name: Cow::Borrowed(table),
            table_name: Cow::Borrowed(table),
            ..EntityTypeMeta::default()
        }
    }

    fn nav(kind: NavigationKind, related: TypeId) -> NavigationMeta {
        NavigationMeta {
            field_name: Cow::Borrowed("nav"),
            kind,
            related_type_id: related,
            related_type_name: Cow::Borrowed(""),
            foreign_key_field: None,
            inverse_navigation: None,
            through_type_id: None,
            through_table: None,
            through_parent_fk: None,
            through_related_fk: None,
            through_parent_fk_index: 0,
            through_related_fk_index: 0,
            related_table: None,
            fk_column: None,
            referenced_key_column: None,
            fk_row_index: 0,
            pk_row_index: 0,
            related_entity_meta: None,
            delete_behavior: None,
        }
    }

    fn foreign_key(referenced_table: &'static str) -> ForeignKeyMeta {
        ForeignKeyMeta {
            name: None,
            columns: vec![Cow::Borrowed("fk_col")],
            referenced_table: Cow::Borrowed(referenced_table),
            referenced_columns: vec![Cow::Borrowed("id")],
            on_delete: None,
        }
    }

    fn composite_foreign_key(referenced_table: &'static str) -> ForeignKeyMeta {
        ForeignKeyMeta {
            name: None,
            columns: vec![Cow::Borrowed("tenant_id"), Cow::Borrowed("fk_col")],
            referenced_table: Cow::Borrowed(referenced_table),
            referenced_columns: vec![Cow::Borrowed("tenant_id"), Cow::Borrowed("id")],
            on_delete: None,
        }
    }

    fn metas(items: Vec<EntityTypeMeta>) -> HashMap<TypeId, EntityTypeMeta> {
        items.into_iter().map(|m| (m.type_id, m)).collect()
    }

    fn location(order: &[TypeId], type_id: TypeId) -> usize {
        order
            .iter()
            .position(|t| *t == type_id)
            .unwrap_or_else(|| panic!("type {type_id:?} missing from order"))
    }

    fn parents_of(graph: &DependencyGraph, type_id: TypeId) -> &[TypeId] {
        graph
            .edges
            .get(&type_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    #[test]
    fn empty_graph_returns_empty() {
        let metas: HashMap<TypeId, EntityTypeMeta> = HashMap::new();
        let graph = DependencyGraph::build(&metas);
        assert!(graph.topological_sort().is_empty());
    }

    #[test]
    fn deletion_order_is_reverse_of_insert() {
        let metas: HashMap<TypeId, EntityTypeMeta> = HashMap::new();
        let graph = DependencyGraph::build(&metas);
        let insert = graph.topological_sort();
        let delete = graph.deletion_order();
        assert_eq!(delete, insert.into_iter().rev().collect::<Vec<_>>());
    }

    /// EF-16: a `BelongsTo` navigation means the *current* entity is the
    /// dependent, so the edge points from it to the related principal — the
    /// opposite direction of a `HasMany` edge.
    #[test]
    fn belongs_to_edge_points_from_dependent_to_principal() {
        let mut child_meta = meta(child(), "child_table");
        child_meta
            .navigations
            .push(nav(NavigationKind::BelongsTo, principal()));

        let graph = DependencyGraph::build(&metas(vec![
            meta(principal(), "principal_table"),
            child_meta,
        ]));

        assert_eq!(parents_of(&graph, child()), &[principal()]);
        assert!(parents_of(&graph, principal()).is_empty());

        let order = graph.topological_sort();
        assert!(location(&order, principal()) < location(&order, child()));
    }

    /// Existing `HasMany` behavior is unchanged: the related child depends on
    /// the declaring principal.
    #[test]
    fn has_many_edge_direction_unchanged() {
        let mut principal_meta = meta(principal(), "principal_table");
        principal_meta
            .navigations
            .push(nav(NavigationKind::HasMany, child()));

        let graph =
            DependencyGraph::build(&metas(vec![principal_meta, meta(child(), "child_table")]));

        assert_eq!(parents_of(&graph, child()), &[principal()]);
        assert!(parents_of(&graph, principal()).is_empty());

        let order = graph.topological_sort();
        assert!(location(&order, principal()) < location(&order, child()));
    }

    /// EF-16: a scalar `#[foreign_key(Target)]` without any navigation must
    /// still produce an edge by reverse-looking-up the principal's table name.
    #[test]
    fn scalar_foreign_key_adds_edge_without_navigation() {
        let mut child_meta = meta(child(), "child_table");
        child_meta.foreign_keys.push(foreign_key("principal_table"));

        let graph = DependencyGraph::build(&metas(vec![
            meta(principal(), "principal_table"),
            child_meta,
        ]));

        assert_eq!(parents_of(&graph, child()), &[principal()]);
        let order = graph.topological_sort();
        assert!(location(&order, principal()) < location(&order, child()));
    }

    /// Composite foreign keys carry the same `referenced_table` and must work
    /// for the reverse lookup as well.
    #[test]
    fn composite_foreign_key_adds_edge() {
        let mut child_meta = meta(child(), "child_table");
        child_meta
            .foreign_keys
            .push(composite_foreign_key("principal_table"));

        let graph = DependencyGraph::build(&metas(vec![
            meta(principal(), "principal_table"),
            child_meta,
        ]));

        assert_eq!(parents_of(&graph, child()), &[principal()]);
        let order = graph.topological_sort();
        assert!(location(&order, principal()) < location(&order, child()));
    }

    /// EF-16: a child that declares the relationship twice (BelongsTo
    /// navigation *and* a scalar FK that resolves to the same principal) must
    /// only get one edge. Duplicate edges make `in_degree` and the decrement
    /// count asymmetric and can silently fall back to insertion order.
    #[test]
    fn duplicate_belongs_to_and_scalar_fk_edges_are_deduped() {
        let mut child_meta = meta(child(), "child_table");
        child_meta
            .navigations
            .push(nav(NavigationKind::BelongsTo, principal()));
        child_meta.foreign_keys.push(foreign_key("principal_table"));

        let graph = DependencyGraph::build(&metas(vec![
            meta(principal(), "principal_table"),
            child_meta,
        ]));

        assert_eq!(
            parents_of(&graph, child()),
            &[principal()],
            "BelongsTo + scalar FK must produce exactly one deduplicated edge"
        );

        let order = graph.topological_sort();
        assert_eq!(
            order.len(),
            2,
            "each node must appear exactly once: {order:?}"
        );
        assert!(location(&order, principal()) < location(&order, child()));
    }

    /// A three-level chain that also declares the FK twice on the middle link:
    /// deduplication must keep the full topological order intact.
    #[test]
    fn duplicate_edges_keep_multi_level_order() {
        let mut child_meta = meta(child(), "child_table");
        child_meta
            .navigations
            .push(nav(NavigationKind::BelongsTo, principal()));
        child_meta.foreign_keys.push(foreign_key("principal_table"));

        let mut grand_child_meta = meta(grand_child(), "grand_child_table");
        grand_child_meta
            .navigations
            .push(nav(NavigationKind::BelongsTo, child()));
        grand_child_meta
            .foreign_keys
            .push(foreign_key("child_table"));

        let graph = DependencyGraph::build(&metas(vec![
            meta(principal(), "principal_table"),
            child_meta,
            grand_child_meta,
        ]));

        let order = graph.topological_sort();
        assert_eq!(order.len(), 3);
        assert!(location(&order, principal()) < location(&order, child()));
        assert!(location(&order, child()) < location(&order, grand_child()));
    }

    /// Self-referential relationships keep the existing ignore strategy: the
    /// self edge exists in the graph but does not affect type-level ordering.
    #[test]
    fn self_referential_fk_is_ignored_by_sort() {
        let mut self_meta = meta(child(), "node_table");
        self_meta
            .navigations
            .push(nav(NavigationKind::BelongsTo, child()));
        self_meta.foreign_keys.push(foreign_key("node_table"));

        let graph = DependencyGraph::build(&metas(vec![self_meta]));

        assert_eq!(parents_of(&graph, child()), &[child()]);
        let order = graph.topological_sort();
        assert_eq!(order, vec![child()]);
    }

    /// A metadata cycle must not hang or drop nodes: the existing fallback
    /// appends whatever the sort could not order.
    #[test]
    fn cycle_falls_back_and_keeps_all_nodes() {
        let mut a_meta = meta(principal(), "a_table");
        a_meta
            .navigations
            .push(nav(NavigationKind::BelongsTo, child()));
        let mut b_meta = meta(child(), "b_table");
        b_meta
            .navigations
            .push(nav(NavigationKind::BelongsTo, principal()));

        let graph = DependencyGraph::build(&metas(vec![a_meta, b_meta]));

        let order = graph.topological_sort();
        assert_eq!(
            order.len(),
            2,
            "cycle fallback must keep every node: {order:?}"
        );
        assert!(order.contains(&principal()));
        assert!(order.contains(&child()));
    }

    /// Views and keyless entities are not insertable tables and must not be
    /// targets of reverse-looked-up FK edges.
    #[test]
    fn view_and_keyless_tables_do_not_contribute_edges() {
        let mut view_meta = meta(unrelated(), "principal_view");
        view_meta.is_view = true;
        let mut keyless_meta = meta(unrelated(), "principal_keyless");
        keyless_meta.is_keyless = true;

        let mut child_meta = meta(child(), "child_table");
        child_meta.foreign_keys.push(foreign_key("principal_view"));
        child_meta
            .foreign_keys
            .push(foreign_key("principal_keyless"));

        let graph = DependencyGraph::build(&metas(vec![view_meta, keyless_meta, child_meta]));

        assert!(
            parents_of(&graph, child()).is_empty(),
            "view/keyless targets must be excluded: {:?}",
            parents_of(&graph, child())
        );
    }

    /// A declared FK whose referenced table is not a registered entity is
    /// skipped instead of panicking.
    #[test]
    fn unknown_foreign_key_target_is_skipped() {
        let mut child_meta = meta(child(), "child_table");
        child_meta.foreign_keys.push(foreign_key("missing_table"));

        let graph = DependencyGraph::build(&metas(vec![child_meta]));

        assert!(parents_of(&graph, child()).is_empty());
        assert_eq!(graph.topological_sort(), vec![child()]);
    }

    /// Deletion order is the reverse of insert order, so a dependent is
    /// deleted before the principal it references.
    #[test]
    fn deletion_order_deletes_dependent_first() {
        let mut child_meta = meta(child(), "child_table");
        child_meta
            .navigations
            .push(nav(NavigationKind::BelongsTo, principal()));

        let graph = DependencyGraph::build(&metas(vec![
            meta(principal(), "principal_table"),
            child_meta,
        ]));

        let delete = graph.deletion_order();
        assert!(location(&delete, child()) < location(&delete, principal()));
    }
}
