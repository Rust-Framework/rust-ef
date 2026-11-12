#[cfg(test)]
mod bool_expr_tests {
    use rust_ef::entity::IEntityType;
    use rust_ef::provider::DbValue;
    use rust_ef::query::{BoolExpr, FilterCondition, QueryBuilder, QueryState};

    fn cond(column: &str, op: &str, value: DbValue) -> BoolExpr {
        BoolExpr::Filter(FilterCondition::with_values(column, op, vec![value]))
    }

    #[test]
    fn test_and_or_combination_sql() {
        let mut state = QueryState::new("items");
        state.where_expr = Some(
            cond("a", "=", DbValue::I32(1))
                .and(BoolExpr::Or(
                    Box::new(cond("a", "=", DbValue::I32(2))),
                    Box::new(cond("b", "<", DbValue::I32(3))),
                ))
                .and(cond("c", ">", DbValue::I32(4))),
        );
        let sql = state.to_sql();
        assert_eq!(
            sql,
            "SELECT * FROM items WHERE ((a = ?) AND ((a = ?) OR (b < ?))) AND (c > ?)"
        );
    }

    #[test]
    fn test_not_filter_sql() {
        let mut state = QueryState::new("items");
        state.where_expr = Some(BoolExpr::Not(Box::new(cond(
            "status",
            "=",
            DbValue::I32(5),
        ))));
        let sql = state.to_sql();
        assert_eq!(sql, "SELECT * FROM items WHERE NOT (status = ?)");
    }

    #[test]
    fn test_filter_in_placeholders() {
        struct Item;
        impl IEntityType for Item {
            fn entity_meta() -> rust_ef::metadata::EntityTypeMeta {
                unimplemented!()
            }
        }

        let sql = QueryBuilder::<Item>::new("items")
            .filter(BoolExpr::Filter(FilterCondition::with_values(
                "id",
                "IN",
                vec![DbValue::I32(1), DbValue::I32(2), DbValue::I32(3)],
            )))
            .to_sql();
        assert!(sql.contains("IN"));
        assert!(sql.contains("?, ?, ?"));
    }

    #[test]
    fn test_complex_or_and_pattern() {
        let mut state = QueryState::new("posts");
        state.where_expr =
            Some(cond("rating", ">", DbValue::I32(3)).or(cond("featured", "=", DbValue::I32(1))));
        let sql = state.to_sql();
        assert_eq!(
            sql,
            "SELECT * FROM posts WHERE (rating > ?) OR (featured = ?)"
        );
    }
}
