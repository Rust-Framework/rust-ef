//! Dependency graph and topological sort — determines entity save order.
//!
//! Principals (independent entities) are ordered before dependents (entities
//! with foreign keys pointing at principals). Delete order is the reverse.
//!
//! Self-referential relationships (a type with a HasMany pointing at itself)
//! do not affect type-level ordering — instance-level ordering is handled by
//! the cascade drain + FK fixup pipeline.

use crate::metadata::EntityTypeMeta;
use std::any::TypeId;
use std::collections::{HashMap, VecDeque};

pub struct DependencyGraph {
    /// child_type_id → list of principal type_ids it depends on.
    edges: HashMap<TypeId, Vec<TypeId>>,
    nodes: Vec<TypeId>,
}

impl DependencyGraph {
    /// Builds the graph from entity metadata. Edges come from HasMany and
    /// ManyToMany navigations: `related_type_id` (child) depends on `type_id`
    /// (principal).
    pub fn build(metas: &HashMap<TypeId, EntityTypeMeta>) -> Self {
        let mut edges: HashMap<TypeId, Vec<TypeId>> = HashMap::new();
        let mut nodes: Vec<TypeId> = Vec::new();
        for (type_id, meta) in metas {
            nodes.push(*type_id);
            for nav in &meta.navigations {
                if matches!(
                    nav.kind,
                    crate::metadata::NavigationKind::HasMany
                        | crate::metadata::NavigationKind::ManyToMany
                ) {
                    edges.entry(nav.related_type_id).or_default().push(*type_id);
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
