//! Regression tests for the upstream defect record of 2026-11-14, fixed in
//! rust-ef 2.5.
//!
//! These are pure unit tests: no database server is contacted. Each test pins
//! one defect ID so a regression is immediately attributable:
//!
//! - **EF-MYSQL-01**: a custom enum column with `max_length` must become an
//!   indexable `VARCHAR(n)` on MySQL (unbounded `TEXT` fails `CREATE INDEX`
//!   with MySQL error 1170); PostgreSQL/SQLite keep `TEXT`.
//! - **EF-MYSQL-02**: a composite index mixing a plain column and an
//!   expression must render `` `tenant_id`, (coalesce(...)) `` — never
//!   `((tenant_id)` (MySQL error 3762, "functional index on a column").
//! - **EF-DDL-01**: derived index/foreign-key names must stay within the
//!   portable 63-byte identifier limit and be deterministic.
//! - **EF-DIFF-01**: types introspected with `COLUMN_TYPE` spelling must not
//!   produce drift, and a server-canonicalized `CHECK` expression must never
//!   make the runtime diff emit `DROP CHECK` / `DROP CONSTRAINT`.
//! - **EF-DDL-02**: portable `CHECK` builders must not render PostgreSQL-
//!   invalid boolean arithmetic (`(a IS NOT NULL) + (b IS NOT NULL) = 1`).
//! - **EF-DIFF-02**: [`MigrationDiff::summary`] field semantics.
//!
//! EF-PG-01 (a `DbValue::Null` parameter failing for non-`TEXT` columns) can
//! only be verified against a live server; its acceptance test lives in
//! `tests/postgres_null_binding.rs`.

use rust_ef::check_expr::CheckExpr;
use rust_ef::metadata::{IndexColumn, IndexMeta};
use rust_ef::migration::{
    DiffSummary, DiffWarning, MigrationDialect, MigrationDiff, MigrationEngine, ModelSnapshot,
    SnapshotCheck, SnapshotColumn, SnapshotEntityType, SnapshotIndex, SnapshotIndexColumn,
    WarningKind, WarningSeverity,
};
use rust_ef::schema::{DbCheck, DbColumn, DbIndex, DbIndexColumn, DbTable};

// ---------------------------------------------------------------------------
// Builders (mirror the shape of `migration/live_diff/tests.rs`)
// ---------------------------------------------------------------------------

fn model(entity_types: Vec<SnapshotEntityType>) -> ModelSnapshot {
    ModelSnapshot {
        migration_id: "__upstream_2_5__".to_string(),
        entity_types,
    }
}

fn table(name: &str, columns: Vec<SnapshotColumn>) -> SnapshotEntityType {
    SnapshotEntityType {
        type_name: name.to_string(),
        table_name: name.to_string(),
        columns,
        ..SnapshotEntityType::default()
    }
}

fn column(name: &str, type_name: &str) -> SnapshotColumn {
    SnapshotColumn {
        field_name: name.to_string(),
        column_name: name.to_string(),
        type_name: type_name.to_string(),
        ..SnapshotColumn::default()
    }
}

fn primary_key(name: &str, type_name: &str) -> SnapshotColumn {
    SnapshotColumn {
        is_primary_key: true,
        is_required: true,
        is_auto_increment: true,
        ..column(name, type_name)
    }
}

fn db_column(name: &str, data_type: &str, nullable: bool) -> DbColumn {
    DbColumn {
        name: name.to_string(),
        data_type: data_type.to_string(),
        is_nullable: nullable,
        ..DbColumn::default()
    }
}

fn db_primary_key(name: &str, data_type: &str) -> DbColumn {
    DbColumn {
        data_type: data_type.to_string(),
        is_primary_key: true,
        is_auto_increment: true,
        ..db_column(name, data_type, false)
    }
}

fn declared_index(name: &str, parts: &[(&str, bool)], is_unique: bool) -> SnapshotIndex {
    SnapshotIndex {
        name: name.to_string(),
        columns: parts
            .iter()
            .map(|(value, is_expression)| SnapshotIndexColumn {
                value: (*value).to_string(),
                is_expression: *is_expression,
            })
            .collect(),
        is_unique,
        filter: None,
        include: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// EF-MYSQL-01: custom enums need a bounded, indexable MySQL type
// ---------------------------------------------------------------------------

#[test]
fn ef_mysql_01_custom_enum_with_length_is_varchar_and_indexable() {
    let custom_enum = SnapshotColumn {
        type_name: "some::CustomEnum".to_string(),
        max_length: Some(32),
        ..SnapshotColumn::default()
    };
    assert_eq!(
        MigrationDialect::MySql.map_column_type(&custom_enum),
        "VARCHAR(32)",
        "EF-MYSQL-01: MySQL cannot index unbounded TEXT; max_length must select VARCHAR(n)"
    );
    assert_eq!(
        MigrationDialect::Postgres.map_column_type(&custom_enum),
        "TEXT",
        "PostgreSQL keeps TEXT (indexable, matches existing databases)"
    );
    assert_eq!(
        MigrationDialect::Sqlite.map_column_type(&custom_enum),
        "TEXT",
        "SQLite keeps TEXT"
    );

    // The same column inside a real CREATE TABLE plan: MySQL must not emit a
    // TEXT column that cannot carry an index.
    let mut link_review = table("link_review", vec![primary_key("id", "i64")]);
    link_review.columns.push(SnapshotColumn {
        is_required: true,
        has_index: true,
        max_length: Some(32),
        ..column("kind", "some::CustomEnum")
    });
    link_review.columns.push(SnapshotColumn {
        is_unique: true,
        max_length: Some(16),
        ..column("code", "some::CustomEnum")
    });

    let diff = MigrationEngine::new(MigrationDialect::MySql).diff(&model(vec![link_review]), &[]);
    let sql = diff.up_sql();

    assert!(
        sql.contains("CREATE TABLE `link_review`"),
        "missing CREATE TABLE:\n{sql}"
    );
    assert!(
        sql.contains("`kind` VARCHAR(32) NOT NULL"),
        "the custom enum column must be VARCHAR(32) on MySQL:\n{sql}"
    );
    assert!(
        !sql.contains("TEXT"),
        "EF-MYSQL-01: no unbounded TEXT column may be created on MySQL:\n{sql}"
    );
    assert!(
        sql.contains("CREATE INDEX `ix_link_review_kind` ON `link_review` (`kind`)"),
        "the indexed custom-enum column must carry a CREATE INDEX:\n{sql}"
    );
    assert!(
        sql.contains("CREATE UNIQUE INDEX `ix_link_review_code` ON `link_review` (`code`)"),
        "the unique custom-enum column must carry a CREATE UNIQUE INDEX:\n{sql}"
    );
    assert!(
        diff.warnings.is_empty(),
        "unexpected warnings: {:?}",
        diff.warnings
    );
}

// ---------------------------------------------------------------------------
// EF-MYSQL-02: expression index key parts keep exactly one level of parentheses
// ---------------------------------------------------------------------------

#[test]
fn ef_mysql_02_expression_index_does_not_double_parenthesize_columns() {
    let mut items = table("items", vec![primary_key("id", "i64")]);
    for (name, length) in [("tenant_id", 32usize), ("channel_id", 32), ("status", 16)] {
        items.columns.push(SnapshotColumn {
            max_length: Some(length),
            ..column(name, "alloc::string::String")
        });
    }
    items.columns.push(SnapshotColumn {
        has_index: true,
        max_length: Some(16),
        ..column("status", "alloc::string::String")
    });
    items.indexes.push(declared_index(
        "ix_items_tenant_scope",
        &[("tenant_id", false), ("coalesce(channel_id, '')", true)],
        true,
    ));

    let diff = MigrationEngine::new(MigrationDialect::MySql).diff(&model(vec![items]), &[]);
    let sql = diff.up_sql();

    assert!(
        sql.contains("(`tenant_id`, (coalesce(channel_id, '')))"),
        "EF-MYSQL-02: the mixed index must render one column part and one \
         expression part:\n{sql}"
    );
    assert!(
        sql.contains("`tenant_id`") && sql.contains("(coalesce(channel_id, ''))"),
        "index key parts must be a quoted column and a parenthesized expression:\n{sql}"
    );
    assert!(
        !sql.contains("((tenant_id)"),
        "EF-MYSQL-02: a plain column part must not be wrapped twice \
         (MySQL error 3762):\n{sql}"
    );
    assert!(
        !sql.contains("(coalesce(channel_id, '')))`"),
        "sanity: the expression part is the last key part:\n{sql}"
    );

    // A plain single-column index must not grow extra parentheses either.
    assert!(
        sql.contains("CREATE INDEX `ix_items_status` ON `items` (`status`)"),
        "missing plain single-column index:\n{sql}"
    );
    assert!(
        !sql.contains("((`status`"),
        "a plain index part must be quoted once:\n{sql}"
    );
}

// ---------------------------------------------------------------------------
// EF-DDL-01: generated names fit the portable identifier limit
// ---------------------------------------------------------------------------

fn assert_portable_identifier(name: &str, context: &str) {
    assert!(
        name.len() <= 63,
        "EF-DDL-01: {context} is {} bytes (limit 63): {name}",
        name.len()
    );
    assert!(
        name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "EF-DDL-01: {context} must be identifier-safe: {name}"
    );
}

#[test]
fn ef_ddl_01_derived_names_are_bounded_and_deterministic() {
    let long_column = "configuration_version_with_a_really_long_suffix";
    let index_name = MigrationEngine::index_name("link_review", long_column);
    assert_portable_identifier(&index_name, "index_name");
    assert_eq!(
        index_name,
        MigrationEngine::index_name("link_review", long_column),
        "EF-DDL-01: derived names must be deterministic"
    );
    assert!(
        index_name.starts_with("ix_link_review_"),
        "derived name keeps the readable prefix: {index_name}"
    );

    let fk_name = MigrationEngine::foreign_key_name(
        "link_review",
        "configuration_version_with_a_really_long_suffix",
        "configuration_version_with_a_really_long_suffix_parent",
    );
    assert_portable_identifier(&fk_name, "foreign_key_name");
    assert_eq!(
        fk_name,
        MigrationEngine::foreign_key_name(
            "link_review",
            "configuration_version_with_a_really_long_suffix",
            "configuration_version_with_a_really_long_suffix_parent",
        ),
        "EF-DDL-01: derived FK names must be deterministic"
    );

    // Four long key parts: the derived name must stay bounded, be stable and
    // still distinguish different declarations (no silent collision).
    let long_index = IndexMeta {
        name: None,
        columns: vec![
            IndexColumn::Column("configuration_version_with_a_really_long_suffix_one".into()),
            IndexColumn::Column("configuration_version_with_a_really_long_suffix_two".into()),
            IndexColumn::Column("configuration_version_with_a_really_long_suffix_three".into()),
            IndexColumn::Column("configuration_version_with_a_really_long_suffix_four".into()),
        ],
        ..IndexMeta::default()
    };
    let derived = long_index.derive_name("link_review");
    assert_portable_identifier(&derived, "IndexMeta::derive_name");
    assert_eq!(
        derived,
        long_index.derive_name("link_review"),
        "EF-DDL-01: IndexMeta::derive_name must be deterministic"
    );

    let mut other = long_index.clone();
    other.columns[3] = IndexColumn::Column("a_different_long_suffix_four".into());
    let other_derived = other.derive_name("link_review");
    assert_portable_identifier(&other_derived, "IndexMeta::derive_name (other)");
    assert_ne!(
        derived, other_derived,
        "EF-DDL-01: different declarations must not collapse to one name"
    );
}

// ---------------------------------------------------------------------------
// EF-DIFF-01: COLUMN_TYPE spellings are not drift
// ---------------------------------------------------------------------------

/// Model of the `orders` table as the migration generator emits it on MySQL.
fn mysql_orders_model() -> SnapshotEntityType {
    let mut orders = table("orders", vec![primary_key("id", "i64")]);
    orders.columns.push(SnapshotColumn {
        max_length: Some(64),
        ..column("name", "alloc::string::String")
    });
    orders.columns.push(column("flag", "bool"));
    orders.columns.push(SnapshotColumn {
        precision: Some(10),
        scale: Some(2),
        ..column("amount", "rust_decimal::Decimal")
    });
    orders.columns.push(SnapshotColumn {
        precision: Some(6),
        ..column("created_at", "chrono::NaiveDateTime")
    });
    orders.columns.push(SnapshotColumn {
        max_length: Some(16),
        has_index: true,
        ..column("status", "alloc::string::String")
    });
    orders.checks.push(SnapshotCheck {
        name: "ck_orders_status".to_string(),
        expression: "status IN ('A', 'B')".to_string(),
    });
    orders
}

/// The same table as MySQL introspection reports it: `data_type` carries the
/// `COLUMN_TYPE` spelling (`varchar(64)`, `tinyint(1)`, `decimal(10,2)`, ...).
fn mysql_orders_live(checks: Vec<DbCheck>) -> DbTable {
    DbTable {
        name: "orders".to_string(),
        columns: vec![
            db_primary_key("id", "bigint"),
            DbColumn {
                max_length: Some(64),
                ..db_column("name", "varchar(64)", true)
            },
            db_column("flag", "tinyint(1)", true),
            DbColumn {
                precision: Some(10),
                scale: Some(2),
                ..db_column("amount", "decimal(10,2)", true)
            },
            DbColumn {
                precision: Some(6),
                ..db_column("created_at", "datetime(6)", true)
            },
            DbColumn {
                max_length: Some(16),
                ..db_column("status", "varchar(16)", true)
            },
        ],
        indexes: vec![DbIndex {
            name: "ix_orders_status".to_string(),
            columns: vec![DbIndexColumn {
                value: "status".to_string(),
                is_expression: false,
            }],
            is_unique: false,
            filter: None,
        }],
        foreign_keys: Vec::new(),
        checks: Some(checks),
        // `Some(vec![])` means "introspected, there are none".
        triggers: Some(Vec::new()),
    }
}

#[test]
fn ef_diff_01_mysql_column_type_spellings_and_canonicalized_checks_are_not_drift() {
    let engine = MigrationEngine::new(MigrationDialect::MySql);
    let live = mysql_orders_live(vec![DbCheck {
        name: Some("ck_orders_status".to_string()),
        expression: "(`status` in (_utf8mb4'A',_utf8mb4'B'))".to_string(),
    }]);

    let diff = engine.diff(&model(vec![mysql_orders_model()]), &[live]);

    assert!(
        diff.statements.is_empty(),
        "EF-DIFF-01: COLUMN_TYPE spellings (`varchar(64)`, `tinyint(1)`, \
         `decimal(10,2)`, `datetime(6)`) must not be drift:\n{}",
        diff.up_sql()
    );
    assert!(
        diff.warnings.is_empty(),
        "a server-canonicalized CHECK (backticks + `_utf8mb4` introducers) is \
         still the same constraint: {:?}",
        diff.warnings
    );
    assert!(
        !diff.has_drift(),
        "a matching schema must not report drift: {:?}",
        diff.summary()
    );
}

#[test]
fn ef_diff_01_unverifiable_check_is_reported_never_dropped() {
    let engine = MigrationEngine::new(MigrationDialect::Postgres);
    let mut orders = table("orders", vec![primary_key("id", "i64")]);
    orders.columns.push(SnapshotColumn {
        max_length: Some(64),
        ..column("name", "alloc::string::String")
    });
    orders.columns.push(column("flag", "bool"));
    orders.columns.push(SnapshotColumn {
        precision: Some(10),
        scale: Some(2),
        ..column("amount", "rust_decimal::Decimal")
    });
    orders.columns.push(SnapshotColumn {
        precision: Some(6),
        ..column("created_at", "chrono::NaiveDateTime")
    });
    orders.columns.push(SnapshotColumn {
        max_length: Some(16),
        has_index: true,
        ..column("status", "alloc::string::String")
    });
    orders.checks.push(SnapshotCheck {
        name: "ck_orders_status".to_string(),
        expression: "status IN ('A', 'B')".to_string(),
    });

    let live = DbTable {
        name: "orders".to_string(),
        columns: vec![
            db_primary_key("id", "bigint"),
            DbColumn {
                max_length: Some(64),
                ..db_column("name", "character varying", true)
            },
            db_column("flag", "boolean", true),
            DbColumn {
                precision: Some(10),
                scale: Some(2),
                ..db_column("amount", "numeric", true)
            },
            DbColumn {
                precision: Some(6),
                ..db_column("created_at", "timestamp without time zone", true)
            },
            DbColumn {
                max_length: Some(16),
                ..db_column("status", "character varying", true)
            },
        ],
        indexes: vec![DbIndex {
            name: "ix_orders_status".to_string(),
            columns: vec![DbIndexColumn {
                value: "status".to_string(),
                is_expression: false,
            }],
            is_unique: false,
            filter: None,
        }],
        foreign_keys: Vec::new(),
        // PostgreSQL rewrote `IN (...)` into `= ANY (ARRAY[...])`; the textual
        // form can no longer be verified against the model.
        checks: Some(vec![DbCheck {
            name: Some("ck_orders_status".to_string()),
            expression: "((status = ANY (ARRAY['A'::text, 'B'::text])))".to_string(),
        }]),
        triggers: Some(Vec::new()),
    };

    let diff = engine.diff(&model(vec![orders]), &[live]);
    let sql = diff.up_sql();

    assert!(
        diff.statements.is_empty(),
        "EF-DIFF-01: an unverifiable CHECK must not generate DDL:\n{sql}"
    );
    let check_warning = diff
        .warnings
        .iter()
        .find(|warning| warning.kind == WarningKind::ConstraintNotIntrospected)
        .unwrap_or_else(|| {
            panic!(
                "expected constraint_not_introspected, got {:?}",
                diff.warnings
            )
        });
    assert_eq!(
        check_warning.severity,
        WarningSeverity::Info,
        "EF-DIFF-01: 'the server canonicalized this expression so it cannot be \
         compared verbatim' is a verification limitation, not a change"
    );
    assert!(
        !diff.has_drift(),
        "EF-DIFF-01: info-level warnings are not drift; a green readiness probe \
         must stay green: {:?}",
        diff.summary()
    );
    assert!(!diff.summary().has_drift);
    for forbidden in ["DROP CHECK", "DROP CONSTRAINT"] {
        assert!(
            !sql.to_ascii_uppercase().contains(forbidden),
            "EF-DIFF-01: runtime diff must never emit {forbidden}:\n{sql}"
        );
    }
    assert!(
        !diff.summary().requires_manual_action,
        "an unverifiable CHECK is informational, not a manual-action blocker: {:?}",
        diff.summary()
    );
    assert_eq!(
        diff.summary().warnings,
        diff.warnings.len(),
        "summary counts every structured warning"
    );
}

#[test]
fn ef_diff_01_missing_column_and_index_are_planned_drift() {
    let engine = MigrationEngine::new(MigrationDialect::MySql);
    // The live table lost the `status` column and its index. The CHECK is still
    // there in its server-canonicalized spelling, so it contributes no DDL.
    let mut live = mysql_orders_live(vec![DbCheck {
        name: Some("ck_orders_status".to_string()),
        expression: "(`status` in (_utf8mb4'A',_utf8mb4'B'))".to_string(),
    }]);
    live.columns.retain(|column| column.name != "status");
    live.indexes.clear();

    let diff = engine.diff(&model(vec![mysql_orders_model()]), &[live]);
    let sql = diff.up_sql();

    assert!(
        sql.contains("ALTER TABLE `orders` ADD COLUMN `status`"),
        "missing column must be added:\n{sql}"
    );
    assert!(
        sql.contains("CREATE INDEX `ix_orders_status` ON `orders` (`status`)"),
        "missing index must be created:\n{sql}"
    );
    for forbidden in ["DROP TABLE", "DROP COLUMN", "DROP INDEX"] {
        assert!(
            !sql.to_ascii_uppercase().contains(forbidden),
            "runtime diff must stay additive, never emit {forbidden}:\n{sql}"
        );
    }

    let summary = diff.summary();
    assert!(diff.has_drift(), "missing column/index is drift");
    assert_eq!(summary.statements, diff.statements.len());
    assert_eq!(
        summary.statements, 2,
        "one ADD COLUMN plus one CREATE INDEX"
    );
    assert_eq!(summary.rebuilds, 0, "MySQL never rebuilds tables");
    assert_eq!(summary.warnings, diff.warnings.len());
    assert!(!summary.requires_manual_action);
}

#[test]
fn ef_diff_01_index_definition_mismatch_is_actionable_drift() {
    let engine = MigrationEngine::new(MigrationDialect::MySql);
    // Same index name as the model, different definition: the runtime diff must
    // report it (it cannot recreate an index in place) and count it as drift.
    let mut live = mysql_orders_live(vec![DbCheck {
        name: Some("ck_orders_status".to_string()),
        expression: "(`status` in (_utf8mb4'A',_utf8mb4'B'))".to_string(),
    }]);
    live.indexes = vec![DbIndex {
        name: "ix_orders_status".to_string(),
        columns: vec![DbIndexColumn {
            value: "name".to_string(),
            is_expression: false,
        }],
        is_unique: false,
        filter: None,
    }];

    let diff = engine.diff(&model(vec![mysql_orders_model()]), &[live]);
    assert!(
        diff.statements.is_empty(),
        "an index mismatch is reported, never silently recreated:\n{}",
        diff.up_sql()
    );
    let warning = diff
        .warnings
        .iter()
        .find(|warning| warning.kind == WarningKind::IndexDefinitionMismatch)
        .unwrap_or_else(|| panic!("expected index_definition_mismatch: {:?}", diff.warnings));
    assert_eq!(
        warning.severity,
        WarningSeverity::Warning,
        "an index mismatch needs an operator follow-up"
    );
    assert!(warning.table.as_deref() == Some("orders"));

    let summary = diff.summary();
    assert!(diff.has_drift(), "a Warning-severity warning is drift");
    assert!(summary.has_drift);
    assert_eq!(summary.statements, 0);
    assert_eq!(summary.warnings, 1);
    assert_eq!(summary.tables, vec!["orders".to_string()]);
    assert!(
        !summary.requires_manual_action,
        "only manual_action warnings block the readiness probe"
    );
}

// ---------------------------------------------------------------------------
// EF-DDL-02: portable CHECK expressions
// ---------------------------------------------------------------------------

#[test]
fn ef_ddl_02_portable_check_expressions_avoid_boolean_arithmetic() {
    let exactly_one = CheckExpr::exactly_one_of(&["tenant_id", "channel_id"]);
    let sql = exactly_one.sql();
    assert!(
        sql.contains("CASE WHEN tenant_id IS NOT NULL THEN 1 ELSE 0 END"),
        "EF-DDL-02: exactly_one_of must use CASE WHEN arithmetic: {sql}"
    );
    assert!(
        !sql.contains("IS NOT NULL) +"),
        "EF-DDL-02: PostgreSQL rejects boolean addition: {sql}"
    );
    assert_eq!(
        sql,
        "(CASE WHEN tenant_id IS NOT NULL THEN 1 ELSE 0 END) + \
         (CASE WHEN channel_id IS NOT NULL THEN 1 ELSE 0 END) = 1"
    );

    // String literals are escaped centrally.
    let in_list = CheckExpr::in_list("status", ["A'B", "C"]);
    assert_eq!(in_list.sql(), "status IN ('A''B', 'C')");

    // Combinators parenthesize each operand so mixed AND/OR keeps its meaning.
    let combined = CheckExpr::any_of([
        CheckExpr::exactly_one_of(&["tenant_id", "channel_id"]),
        CheckExpr::non_negative("amount"),
    ]);
    assert_eq!(
        combined.sql(),
        "((CASE WHEN tenant_id IS NOT NULL THEN 1 ELSE 0 END) + \
         (CASE WHEN channel_id IS NOT NULL THEN 1 ELSE 0 END) = 1) OR (amount >= 0)"
    );
    let all = CheckExpr::all_of([
        CheckExpr::non_negative("amount"),
        CheckExpr::in_list("status", ["A", "B"]),
    ]);
    assert_eq!(all.sql(), "(amount >= 0) AND (status IN ('A', 'B'))");

    // The same expression survives into table DDL unchanged.
    let mut orders = table("orders", vec![primary_key("id", "i64")]);
    orders
        .columns
        .push(column("tenant_id", "alloc::string::String"));
    orders
        .columns
        .push(column("channel_id", "alloc::string::String"));
    orders.checks.push(SnapshotCheck {
        name: "ck_orders_scope".to_string(),
        expression: exactly_one.into(),
    });
    let diff = MigrationEngine::new(MigrationDialect::Postgres).diff(&model(vec![orders]), &[]);
    let sql = diff.up_sql();
    assert!(
        sql.contains("CONSTRAINT \"ck_orders_scope\" CHECK ((CASE WHEN tenant_id IS NOT NULL"),
        "the portable CHECK must be inlined in CREATE TABLE:\n{sql}"
    );
    assert!(
        !sql.contains("IS NOT NULL) +"),
        "EF-DDL-02: no boolean addition may reach PostgreSQL DDL:\n{sql}"
    );
}

// ---------------------------------------------------------------------------
// EF-DIFF-02: summary semantics
// ---------------------------------------------------------------------------

#[test]
fn ef_diff_02_summary_semantics() {
    let empty = MigrationDiff::default();
    let summary = empty.summary();
    assert_eq!(
        summary,
        DiffSummary {
            statements: 0,
            rebuilds: 0,
            warnings: 0,
            tables: Vec::new(),
            has_drift: false,
            requires_manual_action: false,
        },
        "an empty diff summarizes to zeros"
    );

    let with_warning = MigrationDiff {
        statements: vec!["CREATE INDEX `ix_orders_status` ON `orders` (`status`)".to_string()],
        warnings: vec![
            DiffWarning::warning(
                WarningKind::ManualAction,
                Some("orders".to_string()),
                "recreate the constraint manually",
            ),
            // An extra live-only object is *not* drift (the model does not own it).
            DiffWarning::info(
                WarningKind::SkippedRemoval,
                Some("legacy_table".to_string()),
                "extra table in the database",
            ),
        ],
        rebuilds: Vec::new(),
    };
    let summary = with_warning.summary();
    assert_eq!(summary.statements, 1);
    assert_eq!(summary.warnings, 2);
    assert_eq!(summary.rebuilds, 0);
    assert_eq!(
        summary.tables,
        vec!["legacy_table".to_string(), "orders".to_string()],
        "summary tables are the sorted, deduplicated warning tables"
    );
    assert!(
        summary.has_drift,
        "statements and a manual action are drift"
    );
    assert!(
        summary.requires_manual_action,
        "a ManualAction warning requires a manual follow-up"
    );

    // Informational removals alone are not drift and need no manual action.
    let info_only = MigrationDiff {
        statements: Vec::new(),
        warnings: vec![DiffWarning::info(
            WarningKind::SkippedRemoval,
            Some("legacy_table".to_string()),
            "extra table in the database",
        )],
        rebuilds: Vec::new(),
    };
    let summary = info_only.summary();
    assert!(
        !summary.has_drift,
        "a live-only object that the model does not own is not drift"
    );
    assert_eq!(summary.statements, 0);
    assert!(
        info_only.is_empty(),
        "is_empty() only inspects statements, a warning-only plan is empty"
    );
    assert!(!summary.requires_manual_action);
    assert_eq!(summary.tables, vec!["legacy_table".to_string()]);
}
