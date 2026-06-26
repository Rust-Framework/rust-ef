// Template: rust-ef QueryBuilder ? LINQ-style patterns.

use rust_ef::linq;
use rust_ef::prelude::*;

async fn query_examples(ctx: &mut DbContext) -> EFResult<()> {
    let min_rating = 3;

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

    let paged = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 0 => -b.rating)
        .skip(10)
        .take(20)
        .to_list()
        .await?;

    let joined = linq!(ctx.set::<Post>(); inner_join |p: Post, b: Blog| p.blog_id == b.blog_id)
        .to_list()
        .await?;

    let count = ctx.set::<Post>().query().count().await?;
    let has_any = linq!(ctx.set::<Post>(), |p: Post| p.title == "Hello")
        .any()
        .await?;
    let sum_ratings = linq!(ctx.set::<Blog>(); sum b.rating).await?;
    let avg_rating = linq!(ctx.set::<Blog>(); avg b.rating).await?;

    let first = linq!(ctx.set::<Blog>(), |b: Blog| b.url == "https://example.com")
        .first()
        .await?;

    let maybe = linq!(ctx.set::<Blog>(), |b: Blog| b.blog_id == 999)
        .first_or_default()
        .await?;

    let updated = linq!(
        ctx.set::<Blog>(), |b: Blog| b.rating < 1;
        set b.rating, 1;
        execute_update
    ).await?;

    let deleted = linq!(ctx.set::<Post>(), |p: Post| p.blog_id == 0)
        .execute_delete()
        .await?;

    let _ = (
        active, matched, selected, with_content, mid_range, paged, joined, count, has_any,
        sum_ratings, avg_rating, first, maybe, updated, deleted,
    );
    Ok(())
}
