#[cfg(test)]
mod bool_expr_tests {
    use rust_ef::provider::DbValue;
    use rust_ef::query::{BoolExpr, FilterCondition, QueryState};

    #[test]
    fn test_and_or_combination_sql() {
        let mut state = QueryState::new("items");
        state.parameters.push(DbValue::I32(1));
        state.parameters.push(DbValue::I32(2));
        state.parameters.push(DbValue::I32(3));
        state.where_expr = Some(
            BoolExpr::Filter(FilterCondition::new("a", "=", 1))
                .and(BoolExpr::Or(
                    Box::new(BoolExpr::Filter(FilterCondition::new("a", "=", 1))),
                    Box::new(BoolExpr::Filter(FilterCondition::new("a", "=", 1))),
                ))
                .and(BoolExpr::Filter(FilterCondition::new("b", ">", 1))),
        );
        let sql = state.to_sql();
        assert!(sql.contains("WHERE"));
        assert!(sql.contains("AND"));
        assert!(sql.contains("OR"));
        assert!(sql.contains("b"));
    }

    #[test]
    fn test_not_filter_sql() {
        let mut state = QueryState::new("items");
        state.parameters.push(DbValue::I32(5));
        state.where_expr = Some(BoolExpr::Not(Box::new(BoolExpr::Filter(
            FilterCondition::new("status", "=", 1),
        ))));
        let sql = state.to_sql();
        assert!(sql.contains("NOT"));
        assert!(sql.contains("status"));
    }

    #[test]
    fn test_filter_in_placeholders() {
        use rust_ef::entity::IEntityType;
        struct Item;
        impl IEntityType for Item {
            fn entity_meta() -> rust_ef::metadata::EntityTypeMeta {
                unimplemented!()
            }
        }
        let sql = rust_ef::query::QueryBuilder::<Item>::new("items")
            .filter_in(
                "id",
                vec![
                    rust_ef::provider::DbValue::I32(1),
                    rust_ef::provider::DbValue::I32(2),
                    rust_ef::provider::DbValue::I32(3),
                ],
            )
            .to_sql();
        assert!(sql.contains("IN"));
        assert!(sql.contains("?, ?, ?"));
    }

    #[test]
    fn test_raw_filter_no_params() {
        let mut state = QueryState::new("blogs");
        state.where_expr = Some(BoolExpr::Raw("is_deleted = 0".to_string()));
        let sql = state.to_sql();
        assert!(sql.contains("is_deleted = 0"));
    }

    #[test]
    fn test_complex_or_and_pattern() {
        let mut state = QueryState::new("posts");
        state.parameters.push(DbValue::I32(3));
        state.parameters.push(DbValue::I32(1));
        state.where_expr = Some(
            BoolExpr::Filter(FilterCondition::new("rating", ">", 1)).or(BoolExpr::Filter(
                FilterCondition::new("featured", "=", 1),
            )),
        );
        let sql = state.to_sql();
        assert!(sql.contains("OR"));
        assert!(sql.contains("rating"));
        assert!(sql.contains("featured"));
    }
}
