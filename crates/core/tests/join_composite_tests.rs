//! Composite (multi-column) JOIN, chained JOIN, runtime `join`/`join_from` and
//! `filter_joined` coverage (rust-ef 2.7 / EF-04).
//!
//! Every JOIN test asserts both the generated SQL shape (`to_sql()`) and the
//! actual SQLite result, so the SQL text and the runtime semantics cannot drift
//! apart. The single-column cases double as the regression test for the legacy
//! `inner_join`/`left_join`/... SQL, which must stay byte-for-byte unchanged.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::linq;
use rust_ef::prelude::*;
use rust_ef::query::{BoolExpr, FilterCondition, JoinKind, Predicate};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// Root table: `region_id` is the second half of the composite join key.
#[derive(Debug, Clone, EntityType)]
#[table("jc_orders")]
struct JcOrder {
    #[primary_key]
    id: i32,
    region_id: i32,
    total: i64,
    status: String,
}

/// Related table whose primary key is composite (`order_id`, `region_id`).
#[derive(Debug, Clone, EntityType)]
#[table("jc_shipments")]
struct JcShipment {
    #[primary_key]
    order_id: i32,
    #[primary_key]
    region_id: i32,
    carrier: String,
}

/// Second related table for the 3-table chain joined through `carrier`.
#[derive(Debug, Clone, EntityType)]
#[table("jc_carrier_notes")]
struct JcCarrierNote {
    #[primary_key]
    carrier: String,
    note: String,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).unwrap()
}

/// Seeds:
///
/// - orders: (1, region 1), (2, region 2), (3, region 1, no shipment);
/// - shipments: (1, 1, DHL), (2, 1, UPS) — the second one's `region_id` does
///   **not** match order 2, so it only joins on `order_id` (composite key test);
/// - notes: one per carrier.
async fn seed() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<JcOrder>();
    ctx.set::<JcShipment>();
    ctx.set::<JcCarrierNote>();
    ctx.ensure_created().await.unwrap();

    for (id, region_id, total, status) in [
        (1, 1, 100, "pending"),
        (2, 2, 200, "paid"),
        (3, 1, 300, "pending"),
    ] {
        ctx.add::<JcOrder>(JcOrder {
            id,
            region_id,
            total,
            status: status.into(),
        });
    }
    for (order_id, region_id, carrier) in [(1, 1, "DHL"), (2, 1, "UPS")] {
        ctx.add::<JcShipment>(JcShipment {
            order_id,
            region_id,
            carrier: carrier.into(),
        });
    }
    for (carrier, note) in [("DHL", "fast"), ("UPS", "slow"), ("FedEx", "unknown")] {
        ctx.add::<JcCarrierNote>(JcCarrierNote {
            carrier: carrier.into(),
            note: note.into(),
        });
    }
    ctx.save_changes().await.unwrap();
    ctx
}

const ROOT: &str = "SELECT * FROM jc_orders";

// ---------------------------------------------------------------------------
// Legacy single-column JOIN: SQL shape is unchanged
// ---------------------------------------------------------------------------

#[tokio::test]
async fn legacy_single_column_join_sql_is_unchanged() {
    let mut ctx = make_ctx();
    ctx.set::<JcOrder>();
    ctx.set::<JcShipment>();

    assert_eq!(
        linq!(ctx.set::<JcOrder>(); inner_join |o: JcOrder, s: JcShipment| o.id == s.order_id)
            .to_sql(),
        format!("{ROOT} INNER JOIN jc_shipments ON jc_orders.id = jc_shipments.order_id")
    );
    assert_eq!(
        linq!(ctx.set::<JcOrder>(); left_join |o: JcOrder, s: JcShipment| o.id == s.order_id)
            .to_sql(),
        format!("{ROOT} LEFT JOIN jc_shipments ON jc_orders.id = jc_shipments.order_id")
    );
    assert_eq!(
        linq!(ctx.set::<JcOrder>(); right_join |o: JcOrder, s: JcShipment| o.id == s.order_id)
            .to_sql(),
        format!("{ROOT} RIGHT JOIN jc_shipments ON jc_orders.id = jc_shipments.order_id")
    );
    assert_eq!(
        linq!(ctx.set::<JcOrder>(); full_join |o: JcOrder, s: JcShipment| o.id == s.order_id)
            .to_sql(),
        format!("{ROOT} FULL JOIN jc_shipments ON jc_orders.id = jc_shipments.order_id")
    );
    assert_eq!(
        linq!(ctx.set::<JcOrder>(); cross_join s: JcShipment).to_sql(),
        format!("{ROOT} CROSS JOIN jc_shipments")
    );
}

/// Runtime behaviour of the legacy single-column join: both shipments match on
/// `order_id` alone, so orders 1 and 2 come back.
#[tokio::test]
async fn legacy_single_column_join_rows_are_unchanged() {
    let mut ctx = seed().await;
    let orders = linq!(
        ctx.set::<JcOrder>();
        inner_join |o: JcOrder, s: JcShipment| o.id == s.order_id
    )
    .to_list()
    .await
    .unwrap();

    let mut ids: Vec<i32> = orders.iter().map(|o| o.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2], "single-key join matches both shipments");
}

// ---------------------------------------------------------------------------
// Composite JOIN
// ---------------------------------------------------------------------------

#[tokio::test]
async fn composite_join_sql_shape() {
    let mut ctx = make_ctx();
    ctx.set::<JcOrder>();
    ctx.set::<JcShipment>();

    assert_eq!(
        linq!(
            ctx.set::<JcOrder>();
            inner_join |o: JcOrder, s: JcShipment| o.id == s.order_id
                && o.region_id == s.region_id
        )
        .to_sql(),
        format!(
            "{ROOT} INNER JOIN jc_shipments ON jc_orders.id = jc_shipments.order_id \
             AND jc_orders.region_id = jc_shipments.region_id"
        )
    );

    // Reversed operand order is normalized to `<left_table>.<col> = <joined>.<col>`.
    assert_eq!(
        linq!(
            ctx.set::<JcOrder>();
            left_join |o: JcOrder, s: JcShipment| s.region_id == o.region_id
                && s.order_id == o.id
        )
        .to_sql(),
        format!(
            "{ROOT} LEFT JOIN jc_shipments ON jc_orders.region_id = jc_shipments.region_id \
             AND jc_orders.id = jc_shipments.order_id"
        )
    );

    // Parentheses around the `&&` chain are accepted.
    assert_eq!(
        linq!(
            ctx.set::<JcOrder>();
            inner_join |o: JcOrder, s: JcShipment| (o.id == s.order_id && o.region_id == s.region_id)
        )
        .to_sql(),
        format!(
            "{ROOT} INNER JOIN jc_shipments ON jc_orders.id = jc_shipments.order_id \
             AND jc_orders.region_id = jc_shipments.region_id"
        )
    );
}

/// The composite key means order 2's region mismatch excludes it: only order 1
/// matches both key columns.
#[tokio::test]
async fn composite_join_filters_by_both_key_columns() {
    let mut ctx = seed().await;
    let orders = linq!(
        ctx.set::<JcOrder>();
        inner_join |o: JcOrder, s: JcShipment| o.id == s.order_id
            && o.region_id == s.region_id
    )
    .to_list()
    .await
    .unwrap();

    let ids: Vec<i32> = orders.iter().map(|o| o.id).collect();
    assert_eq!(
        ids,
        vec![1],
        "the composite key excludes order 2 (region mismatch)"
    );
}

#[tokio::test]
async fn composite_left_join_keeps_unmatched_root_rows() {
    let mut ctx = seed().await;
    let orders = linq!(
        ctx.set::<JcOrder>();
        left_join |o: JcOrder, s: JcShipment| o.id == s.order_id
            && o.region_id == s.region_id
    )
    .to_list()
    .await
    .unwrap();

    assert_eq!(
        orders.len(),
        3,
        "LEFT JOIN keeps every root row (only order 1 matches the composite key)"
    );
}

// ---------------------------------------------------------------------------
// Chained JOIN (3 tables)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn chained_join_sql_shape() {
    let mut ctx = make_ctx();
    ctx.set::<JcOrder>();
    ctx.set::<JcShipment>();
    ctx.set::<JcCarrierNote>();

    assert_eq!(
        linq!(
            ctx.set::<JcOrder>();
            inner_join |o: JcOrder, s: JcShipment| o.id == s.order_id
                && o.region_id == s.region_id;
            inner_join |s: JcShipment, n: JcCarrierNote| s.carrier == n.carrier
        )
        .to_sql(),
        format!(
            "{ROOT} INNER JOIN jc_shipments ON jc_orders.id = jc_shipments.order_id \
             AND jc_orders.region_id = jc_shipments.region_id \
             INNER JOIN jc_carrier_notes ON jc_shipments.carrier = jc_carrier_notes.carrier"
        ),
        "the second join must continue from the previously joined table"
    );
}

/// Only order 1's shipment (DHL) has a note, so the 3-table chain yields order 1.
#[tokio::test]
async fn chained_join_runtime_rows() {
    let mut ctx = seed().await;
    let orders = linq!(
        ctx.set::<JcOrder>();
        inner_join |o: JcOrder, s: JcShipment| o.id == s.order_id
            && o.region_id == s.region_id;
        inner_join |s: JcShipment, n: JcCarrierNote| s.carrier == n.carrier
    )
    .to_list()
    .await
    .unwrap();

    let ids: Vec<i32> = orders.iter().map(|o| o.id).collect();
    assert_eq!(ids, vec![1]);
}

// ---------------------------------------------------------------------------
// Runtime API: join / join_from
// ---------------------------------------------------------------------------

#[tokio::test]
async fn runtime_join_builds_composite_on() {
    let mut ctx = seed().await;

    let sql = ctx
        .set::<JcOrder>()
        .query()
        .join::<JcShipment>(
            JoinKind::Inner,
            &[("id", "order_id"), ("region_id", "region_id")],
        )
        .to_sql();
    assert_eq!(
        sql,
        format!(
            "{ROOT} INNER JOIN jc_shipments ON jc_orders.id = jc_shipments.order_id \
             AND jc_orders.region_id = jc_shipments.region_id"
        )
    );

    let orders = ctx
        .set::<JcOrder>()
        .query()
        .join::<JcShipment>(
            JoinKind::Inner,
            &[("id", "order_id"), ("region_id", "region_id")],
        )
        .to_list()
        .await
        .unwrap();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].id, 1);
}

#[tokio::test]
async fn runtime_join_from_chains_off_a_joined_table() {
    let mut ctx = seed().await;

    let query = ctx
        .set::<JcOrder>()
        .query()
        .join::<JcShipment>(JoinKind::Inner, &[("id", "order_id")])
        .join_from::<JcCarrierNote, JcShipment>(JoinKind::Inner, &[("carrier", "carrier")]);

    assert_eq!(
        query.to_sql(),
        format!(
            "{ROOT} INNER JOIN jc_shipments ON jc_orders.id = jc_shipments.order_id \
             INNER JOIN jc_carrier_notes ON jc_shipments.carrier = jc_carrier_notes.carrier"
        )
    );

    let orders = query.to_list().await.unwrap();
    let mut ids: Vec<i32> = orders.iter().map(|o| o.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2], "both carriers have a note");
}

// ---------------------------------------------------------------------------
// from_cte + JOIN
// ---------------------------------------------------------------------------

/// A JOIN that follows `from <cte>` must use the CTE name as its left table
/// (the pre-2.7 behaviour), not the root entity's table name — the entity
/// table is not in scope anymore.
#[tokio::test]
async fn join_after_from_cte_uses_the_cte_as_left_table() {
    let mut ctx = seed().await;

    let query = linq!(
        ctx.set::<JcOrder>();
        with filtered as |o: JcOrder| o.id > 0;
        from filtered;
        inner_join |a: JcOrder, b: JcShipment| a.id == b.order_id
    );

    assert_eq!(
        query.to_sql(),
        "WITH filtered AS (SELECT * FROM \"jc_orders\" WHERE id > ?) \
         SELECT * FROM filtered INNER JOIN jc_shipments ON filtered.id = jc_shipments.order_id",
        "the first JOIN after `from <cte>` must continue from the CTE"
    );

    let orders = query.to_list().await.unwrap();
    let mut ids: Vec<i32> = orders.iter().map(|o| o.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2]);
}

// ---------------------------------------------------------------------------
// filter_joined
// ---------------------------------------------------------------------------

#[tokio::test]
async fn filter_joined_qualifies_columns_with_the_joined_table() {
    let mut ctx = make_ctx();
    ctx.set::<JcOrder>();
    ctx.set::<JcShipment>();

    let query = ctx
        .set::<JcOrder>()
        .query()
        .join::<JcShipment>(JoinKind::Inner, &[("id", "order_id")])
        .filter_joined(linq!(|s: JcShipment| s.carrier == "UPS"));

    assert_eq!(
        query.to_sql(),
        format!(
            "{ROOT} INNER JOIN jc_shipments ON jc_orders.id = jc_shipments.order_id \
             WHERE jc_shipments.carrier = ?"
        ),
        "the joined-entity predicate must filter the joined table"
    );
    let (_, params) = query.compile_sql();
    assert_eq!(params, vec![DbValue::from("UPS")]);
}

#[tokio::test]
async fn filter_joined_filters_on_joined_table_rows() {
    let mut ctx = seed().await;

    let ups = ctx
        .set::<JcOrder>()
        .query()
        .join::<JcShipment>(JoinKind::Inner, &[("id", "order_id")])
        .filter_joined(linq!(|s: JcShipment| s.carrier == "UPS"))
        .to_list()
        .await
        .unwrap();
    assert_eq!(ups.len(), 1);
    assert_eq!(ups[0].id, 2, "UPS ships order 2");

    let fedex = ctx
        .set::<JcOrder>()
        .query()
        .join::<JcShipment>(JoinKind::Inner, &[("id", "order_id")])
        .filter_joined(linq!(|s: JcShipment| s.carrier == "FedEx"))
        .to_list()
        .await
        .unwrap();
    assert!(fedex.is_empty(), "no shipment uses FedEx");
}

#[tokio::test]
async fn filter_joined_keeps_already_qualified_columns() {
    let mut ctx = make_ctx();
    ctx.set::<JcOrder>();
    ctx.set::<JcShipment>();

    // A hand-built predicate whose column already carries a table qualifier
    // must not be qualified twice.
    let predicate = Predicate::<JcShipment>::from_expr(BoolExpr::Filter(
        FilterCondition::with_values("jc_shipments.carrier", "=", vec![DbValue::from("DHL")]),
    ));
    let sql = ctx
        .set::<JcOrder>()
        .query()
        .join::<JcShipment>(JoinKind::Inner, &[("id", "order_id")])
        .filter_joined(predicate)
        .to_sql();
    assert_eq!(
        sql,
        format!(
            "{ROOT} INNER JOIN jc_shipments ON jc_orders.id = jc_shipments.order_id \
             WHERE jc_shipments.carrier = ?"
        )
    );
}

// ---------------------------------------------------------------------------
// Composite JOIN + filter_joined through `linq!`
// ---------------------------------------------------------------------------

#[tokio::test]
async fn linq_composite_join_with_filter_joined_runs() {
    let mut ctx = seed().await;

    let dhl = linq!(
        ctx.set::<JcOrder>();
        inner_join |o: JcOrder, s: JcShipment| o.id == s.order_id
            && o.region_id == s.region_id;
    )
    .filter_joined(linq!(|s: JcShipment| s.carrier == "DHL"))
    .to_list()
    .await
    .unwrap();
    assert_eq!(dhl.len(), 1);
    assert_eq!(dhl[0].id, 1);

    // UPS belongs to order 2, whose region does not match the composite key, so
    // the joined row is filtered out by the composite ON before WHERE applies.
    let ups = linq!(
        ctx.set::<JcOrder>();
        inner_join |o: JcOrder, s: JcShipment| o.id == s.order_id
            && o.region_id == s.region_id;
    )
    .filter_joined(linq!(|s: JcShipment| s.carrier == "UPS"))
    .to_list()
    .await
    .unwrap();
    assert!(ups.is_empty());
}

#[test]
fn a_join_without_keys_emits_a_valid_on_clause() {
    // A keyless non-CROSS JOIN is a cross product; emitting `ON 1 = 1` keeps
    // the statement valid instead of a dangling `JOIN t ON`. The runtime
    // builder also debug-asserts non-empty keys, so this asserts the compiled
    // shape of the public `JoinSpec` directly.
    let spec = rust_ef::query::JoinSpec {
        kind: JoinKind::Inner,
        table: "jc_shipments".to_string(),
        keys: Vec::new(),
    };
    assert_eq!(spec.to_sql(), "INNER JOIN jc_shipments ON 1 = 1");

    let cross = rust_ef::query::JoinSpec {
        kind: JoinKind::Cross,
        table: "jc_shipments".to_string(),
        keys: Vec::new(),
    };
    assert_eq!(cross.to_sql(), "CROSS JOIN jc_shipments");
}
