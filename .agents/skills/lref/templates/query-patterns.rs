// Template: lref QueryBuilder — LINQ-style patterns for common operations.
//
// All queries start with: ctx.set::<Entity>().query()

use lref::prelude::*;

async fn query_examples(ctx: &mut DbContext) -> Result<(), LrefError> {
    // === Basic filtering ===

    // Equality / comparison
    let active = ctx.set::<Blog>().query()
        .filter_column("rating", ">", 3)
        .to_list().await?;

    // LIKE (pattern matching)
    let matched = ctx.set::<Post>().query()
        .filter_column("title", "LIKE", "%lref%")
        .to_list().await?;

    // IN clause
    let selected = ctx.set::<Post>().query()
        .filter_in("blog_id", &[1, 2, 3])
        .to_list().await?;

    // NULL checks
    let with_content = ctx.set::<Post>().query()
        .filter_is_not_null("content")
        .to_list().await?;

    let without_content = ctx.set::<Post>().query()
        .filter_is_null("content")
        .to_list().await?;

    // Range
    let mid_range = ctx.set::<Blog>().query()
        .filter_between("rating", 2, 4)
        .to_list().await?;

    // === Ordering & Pagination ===

    let paged = ctx.set::<Blog>().query()
        .order_by_desc_column("rating")
        .skip(10).take(20)
        .to_list().await?;

    // === JOIN ===

    let joined = ctx.set::<Post>().query()
        .inner_join("blogs", "blog_id", "blog_id")
        .to_list().await?;

    // === Aggregation ===

    let count = ctx.set::<Post>().query().count().await?;
    let has_any = ctx.set::<Post>().query()
        .filter_column("title", "=", "Hello")
        .any().await?;
    let sum_ratings = ctx.set::<Blog>().query().sum("rating").await?;
    let avg_rating = ctx.set::<Blog>().query().avg("rating").await?;

    // === Single entity ===

    let first = ctx.set::<Blog>().query()
        .filter_column("url", "=", "https://example.com")
        .first().await?;

    let maybe = ctx.set::<Blog>().query()
        .filter_column("blog_id", "=", 999)
        .first_or_default().await?;  // Option<Blog>

    // === Bulk operations ===

    let updated = ctx.set::<Blog>().query()
        .filter_column("rating", "<", 1)
        .execute_update()
        .set_column("rating", 1)
        .execute().await?;

    let deleted = ctx.set::<Post>().query()
        .filter_column("blog_id", "=", 0)
        .execute_delete().await?;

    let _ = (active, matched, selected, with_content, without_content,
             mid_range, paged, joined, count, has_any, sum_ratings,
             avg_rating, first, maybe, updated, deleted);
    Ok(())
}
