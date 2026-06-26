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

        ctx.set::<LinqBlog>().add(LinqBlog {
            blog_id: 0,
            url: "https://dotnet.example".into(),
            rating: 8,
            active: true,
        });
        ctx.set::<LinqBlog>().add(LinqBlog {
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

        ctx.set::<LinqBlog>().add(LinqBlog {
            blog_id: 0,
            url: "high".into(),
            rating: 8,
            active: true,
        });
        ctx.set::<LinqBlog>().add(LinqBlog {
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

        ctx.set::<LinqBlog>().add(LinqBlog {
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

        let ordered = ctx
            .set::<LinqBlog>()
            .filter(linq!(LinqBlog, |b| b.active => b.url))
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

        ctx.set::<LinqBlog>().add(LinqBlog {
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

        ctx.set::<LinqBlog>().add(LinqBlog {
            blog_id: 0,
            url: "https://dotnet.example".into(),
            rating: 8,
            active: true,
        });
        ctx.set::<LinqBlog>().add(LinqBlog {
            blog_id: 0,
            url: "https://spam.example".into(),
            rating: 8,
            active: true,
        });
        ctx.set::<LinqBlog>().add(LinqBlog {
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

        ctx.set::<LinqBlog>().add(LinqBlog {
            blog_id: 0,
            url: "b".into(),
            rating: 3,
            active: true,
        });
        ctx.set::<LinqBlog>().add(LinqBlog {
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
    async fn test_linq_legacy_expr_form() {
        let mut ctx = make_ctx();
        ctx.set::<LinqBlog>();
        ctx.ensure_created().await.unwrap();

        ctx.set::<LinqBlog>().add(LinqBlog {
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
}
