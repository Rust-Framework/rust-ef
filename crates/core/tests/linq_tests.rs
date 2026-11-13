#[cfg(test)]
mod linq_tests {
    use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
    use rust_ef::linq;
    use rust_ef::prelude::*;
    use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

    #[derive(Debug, Clone, EntityType)]
    #[table("linq_blogs")]
    struct LinqBlog {
        #[primary_key]
        #[auto_increment]
        blog_id: i32,
        #[required]
        url: String,
        rating: i32,
        active: bool,
    }

    fn make_ctx() -> DbContext {
        let mut builder = DbContextOptionsBuilder::new();
        builder.use_sqlite_in_memory();
        let options = builder.build();
        DbContext::from_options(&options).unwrap()
    }

    #[tokio::test]
    async fn test_linq_from_set() {
        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();
        ctx.ensure_created().await.unwrap();

        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "https://dotnet.example".into(),
            rating: 8,
            active: true,
        });
        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "https://low.example".into(),
            rating: 2,
            active: false,
        });
        ctx.save_changes().await.unwrap();

        let high = linq!(ctx.set::<LinqBlog>(), |b: LinqBlog| b.rating > 5)
            .to_list()
            .await
            .unwrap();
        assert_eq!(high.len(), 1);
    }

    #[tokio::test]
    async fn test_reusable_linq_expr() {
        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();
        ctx.ensure_created().await.unwrap();

        let min_rating = 5;
        let expr = linq!(|b: LinqBlog| b.rating > min_rating);

        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "high".into(),
            rating: 8,
            active: true,
        });
        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "low".into(),
            rating: 2,
            active: true,
        });
        ctx.save_changes().await.unwrap();

        let high = ctx.set::<LinqBlog>().filter(expr).to_list().await.unwrap();
        assert_eq!(high.len(), 1);

        let ids = [8i32, 2];
        let in_expr = linq!(|b: LinqBlog| ids.contains(b.rating));
        let rows = ctx
            .set::<LinqBlog>()
            .filter(in_expr)
            .to_list()
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn test_linq_filter_on_set() {
        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();
        ctx.ensure_created().await.unwrap();

        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "https://dotnet.example".into(),
            rating: 8,
            active: true,
        });
        ctx.save_changes().await.unwrap();

        let typed = ctx
            .set::<LinqBlog>()
            .filter(linq!(|b: LinqBlog| b.url.contains("dotnet")))
            .to_list()
            .await
            .unwrap();
        assert_eq!(typed.len(), 1);

        let ordered = linq!(ctx.set::<LinqBlog>(), |b: LinqBlog| b.active => b.url)
            .to_list()
            .await
            .unwrap();
        assert_eq!(ordered[0].url, "https://dotnet.example");
    }

    #[tokio::test]
    async fn test_linq_query_filter() {
        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();
        ctx.ensure_created().await.unwrap();

        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "x".into(),
            rating: 7,
            active: true,
        });
        ctx.save_changes().await.unwrap();

        let rows = linq!(ctx.set::<LinqBlog>().query(), |b: LinqBlog| b.rating > 5)
            .to_list()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn test_linq_and_or_parens_not_contains() {
        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();
        ctx.ensure_created().await.unwrap();

        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "https://dotnet.example".into(),
            rating: 8,
            active: true,
        });
        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "https://spam.example".into(),
            rating: 8,
            active: true,
        });
        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "https://low.example".into(),
            rating: 2,
            active: false,
        });
        ctx.save_changes().await.unwrap();

        let dotnet = linq!(ctx.set::<LinqBlog>(), |b: LinqBlog| b
            .url
            .contains("dotnet")
            && !(b.rating < 3))
        .to_list()
        .await
        .unwrap();
        assert_eq!(dotnet.len(), 1);

        let either = linq!(ctx.set::<LinqBlog>(), |b: LinqBlog| (b.rating > 5
            || b.rating < 3)
            && b.active)
        .to_list()
        .await
        .unwrap();
        assert_eq!(either.len(), 2);
    }

    #[tokio::test]
    async fn test_linq_order_by() {
        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();
        ctx.ensure_created().await.unwrap();

        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "b".into(),
            rating: 3,
            active: true,
        });
        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "a".into(),
            rating: 9,
            active: true,
        });
        ctx.save_changes().await.unwrap();

        let ordered = linq!(
            ctx.set::<LinqBlog>(),
            |b: LinqBlog| b.active => b.url
        )
        .to_list()
        .await
        .unwrap();
        assert_eq!(ordered.len(), 2);
        assert_eq!(ordered[0].url, "a");

        let desc = linq!(
            ctx.set::<LinqBlog>(),
            |b: LinqBlog| b.active => -b.rating
        )
        .to_list()
        .await
        .unwrap();
        assert_eq!(desc[0].rating, 9);
    }

    #[tokio::test]
    async fn test_linq_contains_value_means_like() {
        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();
        ctx.ensure_created().await.unwrap();

        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "https://dotnet.example".into(),
            rating: 8,
            active: true,
        });
        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "https://spam.example".into(),
            rating: 8,
            active: true,
        });
        ctx.save_changes().await.unwrap();

        // A variable argument is a *value* (LIKE), not a column (IN).
        let needle = "dotnet";
        let rows = ctx
            .set::<LinqBlog>()
            .filter(linq!(|b: LinqBlog| b.url.contains(needle)))
            .to_list()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].url, "https://dotnet.example");
    }

    #[tokio::test]
    async fn test_linq_and_or_group_on_the_right() {
        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();
        ctx.ensure_created().await.unwrap();

        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "https://dotnet.example".into(),
            rating: 8,
            active: true,
        });
        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "https://spam.example".into(),
            rating: 8,
            active: true,
        });
        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "https://low.example".into(),
            rating: 2,
            active: false,
        });
        ctx.save_changes().await.unwrap();

        // `rating > 5 && (dotnet || low)` must not leak `low` past the rating
        // bound: `(rating > 5) AND (dotnet OR low)` matches one row, while the
        // left-to-right `((rating > 5) AND dotnet) OR low` would match two.
        let rows = linq!(ctx.set::<LinqBlog>(), |b: LinqBlog| b.rating > 5
            && (b.url.contains("dotnet") || b.url.contains("low")))
        .to_list()
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].url, "https://dotnet.example");
    }

    #[tokio::test]
    async fn test_linq_predicate_in_subquery_uses_argument_column() {
        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();
        ctx.ensure_created().await.unwrap();

        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "high".into(),
            rating: 8,
            active: true,
        });
        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "low".into(),
            rating: 2,
            active: true,
        });
        ctx.save_changes().await.unwrap();

        // Predicate form: `ids.contains(b.rating)` → `rating IN (?, ?)`.
        let ids = [8i32, 2];
        let expr = linq!(|b: LinqBlog| ids.contains(b.rating));
        let rows = ctx
            .set::<LinqBlog>()
            .query()
            .filter(expr)
            .to_list()
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn test_linq_source_with_condition() {
        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();
        ctx.ensure_created().await.unwrap();

        ctx.add::<LinqBlog>(LinqBlog {
            blog_id: 0,
            url: "x".into(),
            rating: 7,
            active: true,
        });
        ctx.save_changes().await.unwrap();

        let rows = linq!(ctx.set::<LinqBlog>(), |b: LinqBlog| b.rating > 5)
            .to_list()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
    }

    // ── EF-1: local-collection `IN` and value `LIKE` ──

    /// Travel hot paths: bulk-ID lookups with `Vec<i64>` and `Vec<String>`
    /// receivers must both compile to `IN`.
    #[test]
    fn test_linq_local_collections_i64_and_string_in_sql() {
        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();

        let ids: Vec<i64> = vec![8, 2];
        let sql = ctx
            .set::<LinqBlog>()
            .filter(linq!(|b: LinqBlog| ids.contains(b.rating)))
            .to_sql();
        assert!(sql.contains("rating IN (?, ?)"), "Vec<i64>: {sql}");

        let urls: Vec<String> = vec!["a.example".into(), "b.example".into()];
        let sql = ctx
            .set::<LinqBlog>()
            .filter(linq!(|b: LinqBlog| urls.contains(b.url)))
            .to_sql();
        assert!(sql.contains("url IN (?, ?)"), "Vec<String>: {sql}");
    }

    /// EF-1 acceptance: a local collection on the receiver side compiles to
    /// SQL `IN`, not to `COLUMN_<RECEIVER>`.
    #[test]
    fn test_linq_local_collection_in_sql() {
        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();

        let ids: Vec<i32> = vec![8, 2];
        let sql = ctx
            .set::<LinqBlog>()
            .filter(linq!(|b: LinqBlog| ids.contains(b.rating)))
            .to_sql();
        assert!(sql.contains("rating IN (?, ?)"), "expected IN, got: {sql}");
    }

    /// EF-1: the IN receiver is a collection expression, not a column — it may
    /// be anything iterable (`Vec`, field, function call, `&ids[..]`).
    #[test]
    fn test_linq_in_receiver_accepts_collection_expression() {
        fn ids_for_in() -> Vec<i32> {
            vec![8, 2]
        }

        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();

        let sql = ctx
            .set::<LinqBlog>()
            .filter(linq!(|b: LinqBlog| ids_for_in().contains(b.rating)))
            .to_sql();
        assert!(sql.contains("rating IN (?, ?)"), "expected IN, got: {sql}");
    }

    /// EF-1 acceptance: `b.name.contains(value)` must keep compiling to LIKE
    /// (the receiver really is a column in this spelling).
    #[test]
    fn test_linq_contains_value_still_like_sql() {
        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();

        let needle = "dotnet";
        let sql = ctx
            .set::<LinqBlog>()
            .filter(linq!(|b: LinqBlog| b.url.contains(needle)))
            .to_sql();
        assert!(sql.contains("url LIKE ?"), "expected LIKE, got: {sql}");
        assert!(!sql.contains(" IN "), "must not become IN: {sql}");
    }
}
