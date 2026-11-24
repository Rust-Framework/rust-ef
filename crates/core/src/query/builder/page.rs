//! `QueryBuilder::to_page` — the offset-pagination terminal.
//!
//! One call runs the two statements a list endpoint needs against the **same
//! filter chain**: a `COUNT(*)` (paging/order removed) and the requested page
//! (`LIMIT`/`OFFSET`). It returns the [`PagedResult`] envelope directly, so a
//! handler can return it without a second translation step.

use rust_core::{PagedRequest, PagedResult, MAX_PAGE_SIZE};

use crate::entity::{
    IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, ILazyInit, INavigationSetter,
};
use crate::error::EFResult;

use super::core::QueryBuilder;

impl<T> QueryBuilder<T>
where
    T: IEntityType + IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
{
    /// Builds the count query for [`to_page`](Self::to_page).
    ///
    /// A single-value `COUNT(*)` terminal rejects `ORDER BY` / `skip` / `take`
    /// (`QueryState::validate`), so the counter is a clone with those clauses
    /// dropped. Everything that defines *which rows* — FROM/JOIN/WHERE and the
    /// deferred builder validation — is kept, so the total always refers to the
    /// same filter as the page. Grouping/DISTINCT/window/set-operation shapes
    /// are left untouched on purpose: their total is undefined, and `count()`
    /// reports a typed `EFError::configuration` instead of a wrong number.
    fn count_query(&self) -> Self {
        let mut counter = self.clone();
        counter.state.orderings.clear();
        counter.state.offset = None;
        counter.state.limit = None;
        counter
    }

    /// Executes this query as one offset-pagination page and returns the flat
    /// [`PagedResult`] envelope directly (there is no tuple variant and no "skip
    /// the count" switch).
    ///
    /// The envelope composes the operation outcome and the page rows:
    /// `{success,status,message,items,total}`; `PagedResult::new` uses
    /// `success=true/status=0/message="ok"`, and the endpoint can override the
    /// outcome with `with_status`/`with_message`. The page part still has no
    /// `page`/`page_size`/`total_pages` echo.
    ///
    /// Semantics:
    ///
    /// - **Two statements, one filter**: `COUNT(*)` runs first on a clone with
    ///   `ORDER BY`/`skip`/`take` removed and every other clause kept, then the
    ///   page runs with `skip(paging.offset()).take(paging.limit())`. Ordering
    ///   on the builder applies to the page only, never to the total. The two
    ///   statements are not a snapshot: under concurrent writes `total` and
    ///   `items` can come from different instants — wrap the call in an
    ///   explicit transaction when the list response must be consistent.
    /// - **Out-of-range page**: `items` is empty and `total` is the real total;
    ///   the request is not an error.
    /// - **Offset width**: `PagedRequest::offset()` is a `u64`; it is saturated
    ///   to `usize::MAX` on targets where it does not fit, never truncated.
    /// - **No clamping**: `PagedRequest` validation (page/page_size ranges)
    ///   belongs to the binding layer — this terminal trusts the contract. A
    ///   `debug_assert!` flags an out-of-range `page_size` as a programming
    ///   error; it is not silently corrected (correcting it would make the
    ///   client's `total_pages = ceil(total / page_size)` disagree with the
    ///   page it received).
    /// - **Entity page**: the returned items are entities (`T` need not be
    ///   `Serialize`). Map them to the response DTO with
    ///   [`PagedResult::map_items`] before serializing.
    /// - **Stable order**: the caller supplies `order_by`/`order_by_desc` before
    ///   calling; the last ordering column must be unique (e.g. the primary
    ///   key) so rows cannot shift between pages.
    ///
    /// The supported query shapes are the ones of [`to_list`](Self::to_list) and
    /// [`count`](Self::count): grouped queries, `DISTINCT`, aggregate/window
    /// projections and set operations cannot produce a well-defined total and
    /// are reported as typed configuration errors.
    ///
    /// ```ignore
    /// let page = ctx
    ///     .set::<Order>()
    ///     .query()
    ///     .filter(linq!(|o: Order| o.tenant_id == tenant_id))
    ///     .order_by_field_desc(Order::created_at)
    ///     .then_by_field_desc(Order::id) // unique tie-breaker
    ///     .to_page(&req.paging)
    ///     .await?;
    ///
    /// // The endpoint maps the entity page to its DTO page:
    /// let page: PagedResult<OrderModel> = page.map_items(OrderModel::from);
    /// ```
    pub async fn to_page(self, paging: &PagedRequest) -> EFResult<PagedResult<T>> {
        debug_assert!(
            (1..=MAX_PAGE_SIZE).contains(&paging.page_size),
            "PagedRequest::page_size out of contract (1..={MAX_PAGE_SIZE}); the binding layer \
             must validate it, to_page does not clamp"
        );

        let total = self.count_query().count().await?;
        let offset = usize::try_from(paging.offset()).unwrap_or(usize::MAX);
        let items = self
            .skip(offset)
            .take(paging.limit() as usize)
            .to_list()
            .await?;

        // `count()` is a COUNT(*); a negative value cannot occur, but stay
        // panic-free if a future provider ever reports one.
        Ok(PagedResult::new(items, total.max(0) as u64))
    }
}
