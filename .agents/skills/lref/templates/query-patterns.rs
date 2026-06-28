// Template: rust-ef QueryBuilder — LINQ-style patterns.
// 
// PREFER linq! over string-based API (filter_column, order_by_column):
// - linq! provides compile-time field checking
// - filter_column/order_by_column use string column names — typos not caught
// - Use query().find(id) only for simple primary-key lookups

use rust_ef::linq;
use rust_ef::prelude::*;

async fn query_examples(ctx: &mut DbContext) -> EFResult<()> {
    let min_rating = 3;

    // ── Form A: filter + terminals ──

    let active = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > min_rating)
        .to_list()
        .await?;

    let matched = linq!(ctx.set::<Post>(), |p: Post| p.title.contains("rust"))
        .to_list()
        .await?;

    let ids = [1i32, 2, 3];
    let selected = linq!(ctx.set::<Post>(), |p: Post| ids.contains(p.blog_id))
        .to_list()
        .await?;

    let with_content = linq!(ctx.set::<Post>(), |p: Post| p.content.is_not_null())
        .to_list()
        .await?;

    let mid_range = linq!(ctx.set::<Blog>(), |b: Blog| b.rating.between(2, 4))
        .to_list()
        .await?;

    // ── Form B: multi-clause (include, order_by, group_by, etc.) ──

    let paged = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 0;
        order_by b.rating desc;
    ).skip(10).take(20).to_list().await?;

    let joined = linq!(ctx.set::<Post>(); inner_join |p: Post, b: Blog| p.blog_id == b.blog_id)
        .to_list()
        .await?;

    let with_nav = linq!(ctx.set::<Blog>(); include b.posts then b.comments)
        .to_list().await?;

    // ── Aggregates ──

    let count = ctx.set::<Post>().query().count().await?;
    let has_any = linq!(ctx.set::<Post>(), |p: Post| p.title == "Hello")
        .any()
        .await?;
    let sum_ratings = linq!(ctx.set::<Blog>(); sum b.rating).await?;
    let avg_rating = linq!(ctx.set::<Blog>(); avg b.rating).await?;

    // ── Single entity ──

    let first = linq!(ctx.set::<Blog>(), |b: Blog| b.url == "https://example.com")
        .first()
        .await?;

    let maybe = linq!(ctx.set::<Blog>(), |b: Blog| b.blog_id == 999)
        .first_or_default()
        .await?;

    // Simple PK lookup — query().find(id) is fine here
    let by_id = ctx.set::<Blog>().query().find(1).await?;

    // ── Bulk operations ──

    let updated = linq!(
        ctx.set::<Blog>(), |b: Blog| b.rating < 1;
        set b.rating, 1;
        execute_update
    ).await?;

    let deleted = linq!(ctx.set::<Post>(), |p: Post| p.blog_id == 0)
        .execute_delete()
        .await?;

    // ── Global query filter (register once at startup) ──
    ctx.model().entity::<Blog>()
        .has_query_filter(linq!(filter |b: Blog| !b.is_deleted));

    let _ = (
        active, matched, selected, with_content, mid_range, paged, joined,
        with_nav, count, has_any, sum_ratings, avg_rating, first, maybe,
        by_id, updated, deleted,
    );
    Ok(())
}
