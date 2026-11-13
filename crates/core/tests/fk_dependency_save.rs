//! EF-16: insert ordering when only the *dependent* side declares the
//! relationship.
//!
//! Before 2.4.0 `DependencyGraph::build` only derived edges from
//! `HasMany`/`ManyToMany` navigations. A child that declared the relationship
//! on its own side (`#[foreign_key(Parent)]` and/or `parent: BelongsTo<Parent>`)
//! while the parent had **no** `HasMany<Child>` produced no edge at all, so the
//! type-level INSERT order was metadata (HashMap) order. With SQLite foreign-key
//! enforcement on, that failed roughly half of the time with
//! `FOREIGN KEY constraint failed`.
//!
//! These tests run the real save pipeline against an in-memory SQLite database
//! whose schema is created by `DbContext::bootstrap` (bootstrap — unlike
//! `ensure_created` — emits `REFERENCES` constraints), with
//! `PRAGMA foreign_keys=ON`.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

const ITERATIONS: i32 = 100;

// ── Case 1: dependent declares BelongsTo + scalar FK, principal has no HasMany ──

#[derive(Debug, Clone, EntityType)]
#[table("fk_order_users")]
pub struct FkOrderUser {
    #[primary_key]
    pub user_id: i32,
    #[required]
    pub name: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("fk_order_members")]
pub struct FkOrderMember {
    #[primary_key]
    pub member_id: i32,
    #[foreign_key(FkOrderUser)]
    pub user_id: i32,
    #[navigation]
    pub user: BelongsTo<FkOrderUser>,
}

// ── Case 2: dependent declares only a scalar FK (no navigation at all) ──

#[derive(Debug, Clone, EntityType)]
#[table("fk_scalar_owners")]
pub struct FkScalarOwner {
    #[primary_key]
    pub owner_id: i32,
    #[required]
    pub label: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("fk_scalar_pets")]
pub struct FkScalarPet {
    #[primary_key]
    pub pet_id: i32,
    #[foreign_key(FkScalarOwner)]
    pub owner_id: i32,
    #[required]
    pub name: String,
}

fn fk_enforcing_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory_with(|options| {
        // Connection-level switch: without it SQLite silently ignores
        // REFERENCES constraints, which would hide the ordering bug.
        options.foreign_keys(true);
    });
    let mut ctx = DbContext::from_options(&builder.build()).expect("context");
    ctx.set::<FkOrderUser>();
    ctx.set::<FkOrderMember>();
    ctx.set::<FkScalarOwner>();
    ctx.set::<FkScalarPet>();
    ctx
}

async fn fk_violations(ctx: &DbContext) -> i64 {
    // `PRAGMA foreign_key_check` reports one row per orphaned FK.
    let mut conn = ctx.provider().get_connection().await.expect("connection");
    let rows = conn
        .query("PRAGMA foreign_key_check", &[])
        .await
        .expect("foreign_key_check");
    rows.len() as i64
}

#[tokio::test]
async fn belongs_to_child_inserted_after_parent_for_100_saves() {
    let mut ctx = fk_enforcing_ctx();
    ctx.bootstrap().await.expect("bootstrap with FK DDL");

    for i in 1..=ITERATIONS {
        ctx.add::<FkOrderUser>(FkOrderUser {
            user_id: i,
            name: format!("user-{i}"),
        });
        ctx.add::<FkOrderMember>(FkOrderMember {
            member_id: i,
            user_id: i,
            user: BelongsTo::new(),
        });
        ctx.save_changes()
            .await
            .unwrap_or_else(|e| panic!("save #{i} failed (FK insert order): {e}"));
    }

    let users = ctx
        .set::<FkOrderUser>()
        .query()
        .to_list()
        .await
        .expect("users");
    let members = ctx
        .set::<FkOrderMember>()
        .query()
        .to_list()
        .await
        .expect("members");
    assert_eq!(users.len(), ITERATIONS as usize);
    assert_eq!(members.len(), ITERATIONS as usize);
    assert_eq!(fk_violations(&ctx).await, 0, "no orphaned FK rows");
}

#[tokio::test]
async fn scalar_fk_child_inserted_after_parent_for_100_saves() {
    let mut ctx = fk_enforcing_ctx();
    ctx.bootstrap().await.expect("bootstrap with FK DDL");

    for i in 1..=ITERATIONS {
        ctx.add::<FkScalarOwner>(FkScalarOwner {
            owner_id: i,
            label: format!("owner-{i}"),
        });
        ctx.add::<FkScalarPet>(FkScalarPet {
            pet_id: i,
            owner_id: i,
            name: format!("pet-{i}"),
        });
        ctx.save_changes()
            .await
            .unwrap_or_else(|e| panic!("save #{i} failed (FK insert order): {e}"));
    }

    let owners = ctx
        .set::<FkScalarOwner>()
        .query()
        .to_list()
        .await
        .expect("owners");
    let pets = ctx
        .set::<FkScalarPet>()
        .query()
        .to_list()
        .await
        .expect("pets");
    assert_eq!(owners.len(), ITERATIONS as usize);
    assert_eq!(pets.len(), ITERATIONS as usize);
    assert_eq!(fk_violations(&ctx).await, 0, "no orphaned FK rows");
}

/// Sanity check that the test setup really enforces foreign keys: an orphan
/// child inserted directly through the provider must be rejected. If this ever
/// regresses, the ordering tests above would pass vacuously.
#[tokio::test]
async fn test_database_rejects_orphan_child() {
    let ctx = fk_enforcing_ctx();
    ctx.bootstrap().await.expect("bootstrap with FK DDL");

    let mut conn = ctx.provider().get_connection().await.expect("connection");
    let err = conn
        .execute(
            "INSERT INTO fk_order_members (member_id, user_id) VALUES (1, 999)",
            &[],
        )
        .await;
    assert!(err.is_err(), "orphan FK insert must be rejected");
}
