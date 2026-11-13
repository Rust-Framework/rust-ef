//! Unit tests for runtime model diffing.

use crate::schema::{
    DbCheck, DbColumn, DbForeignKey, DbIndex, DbIndexColumn, DbTable,
};

use crate::migration::types::{ModelSnapshot, SnapshotCheck, SnapshotColumn, SnapshotEntityType};
use crate::migration::MigrationDialect;
use super::{MigrationDiff, MigrationEngine, WarningKind, WarningSeverity};

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn model(entity_types: Vec<SnapshotEntityType>) -> ModelSnapshot {
    ModelSnapshot {
        migration_id: "__test__".to_string(),
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
        is_primary_key: true,
        is_auto_increment: true,
        ..db_column(name, data_type, false)
    }
}

fn live_table(name: &str, columns: Vec<DbColumn>) -> DbTable {
    DbTable {
        name: name.to_string(),
        columns,
        ..DbTable::default()
    }
}

fn diff(dialect: MigrationDialect, model: &ModelSnapshot, live: &[DbTable]) -> MigrationDiff {
    MigrationEngine::new(dialect).diff(model, live)
}

fn has_warning(diff: &MigrationDiff, kind: WarningKind) -> bool {
    diff.warnings.iter().any(|warning| warning.kind == kind)
}

// ---------------------------------------------------------------------------
// Adding tables
// ---------------------------------------------------------------------------

#[test]
fn creates_missing_table_with_unique_index() {
    let mut items = table("items", vec![primary_key("id", "i32")]);
    let mut name = column("name", "String");
    name.max_length = Some(32);
    name.is_unique = true;
    items.columns.push(name);

    let diff = diff(
        MigrationDialect::Sqlite,
        &model(vec![items]),
        &[],
    );

    let sql = diff.up_sql();
    assert!(
        sql.contains("CREATE TABLE \"items\""),
        "missing CREATE TABLE:\n{sql}"
    );
    assert!(
        sql.contains("CREATE UNIQUE INDEX \"ix_items_name\" ON \"items\" (\"name\")"),
        "missing unique index:\n{sql}"
    );
    assert!(
        !sql.contains("__ef_migrations_history"),
        "runtime diff must not write migration history:\n{sql}"
    );
    assert!(diff.warnings.is_empty(), "unexpected warnings: {:?}", diff.warnings);
    assert!(!diff.is_empty());
}

#[test]
fn creates_missing_table_with_inline_foreign_key_on_sqlite() {
    let parents = table("parents", vec![primary_key("id", "i32")]);
    let mut children = table("children", vec![primary_key("id", "i32")]);
    let mut parent_id = column("parent_id", "i32");
    parent_id.is_foreign_key = true;
    parent_id.fk_referenced_table = Some("parents".to_string());
    parent_id.fk_referenced_column = Some("id".to_string());
    parent_id.fk_on_delete = Some("CASCADE".to_string());
    children.columns.push(parent_id);

    let live = vec![live_table("parents", vec![db_primary_key("id", "INTEGER")])];
    let diff = diff(
        MigrationDialect::Sqlite,
        &model(vec![parents, children]),
        &live,
    );

    let sql = diff.up_sql();
    assert!(
        sql.contains("CONSTRAINT \"fk_children_parent_id_parents\" FOREIGN KEY (\"parent_id\") REFERENCES \"parents\" (\"id\") ON DELETE CASCADE"),
        "SQLite must inline the FK in CREATE TABLE:\n{sql}"
    );
    assert!(
        !sql.contains("ADD CONSTRAINT"),
        "SQLite cannot ADD CONSTRAINT:\n{sql}"
    );
}

// ---------------------------------------------------------------------------
// Adding columns / indexes
// ---------------------------------------------------------------------------

#[test]
fn adds_column_and_index_to_existing_table() {
    let mut items = table("items", vec![primary_key("id", "i32")]);
    let mut stock = column("stock", "i32");
    stock.default_value = Some("0".to_string());
    stock.is_required = true;
    stock.has_index = true;
    items.columns.push(stock);
    items.columns.push(column("note", "String"));

    let live = vec![live_table(
        "items",
        vec![db_primary_key("id", "INTEGER")],
    )];
    let diff = diff(
        MigrationDialect::Sqlite,
        &model(vec![items]),
        &live,
    );

    let sql = diff.up_sql();
    assert!(
        sql.contains("ALTER TABLE \"items\" ADD COLUMN \"stock\" INTEGER NOT NULL DEFAULT 0"),
        "missing ADD COLUMN with default:\n{sql}"
    );
    assert!(
        sql.contains("ALTER TABLE \"items\" ADD COLUMN \"note\" TEXT NULL"),
        "missing nullable ADD COLUMN:\n{sql}"
    );
    assert!(
        sql.contains("CREATE INDEX \"ix_items_stock\" ON \"items\" (\"stock\")"),
        "missing CREATE INDEX:\n{sql}"
    );
    // Columns are added before the index that covers them.
    let stock_position = sql.find("ADD COLUMN \"stock\"").unwrap();
    let index_position = sql.find("ix_items_stock").unwrap();
    assert!(stock_position < index_position, "index must follow column:\n{sql}");
    assert!(
        !has_warning(&diff, WarningKind::NotNullColumnWithoutDefault),
        "column has a default, no warning expected: {:?}",
        diff.warnings
    );
}

#[test]
fn not_null_column_without_default_warns() {
    let mut items = table("items", vec![primary_key("id", "i32")]);
    items.columns.push(SnapshotColumn {
        is_required: true,
        ..column("stock", "i32")
    });

    let live = vec![live_table("items", vec![db_primary_key("id", "INTEGER")])];
    let diff = diff(MigrationDialect::Sqlite, &model(vec![items]), &live);

    assert!(
        has_warning(&diff, WarningKind::NotNullColumnWithoutDefault),
        "expected NOT NULL warning: {:?}",
        diff.warnings
    );
    assert!(diff.up_sql().contains("ADD COLUMN \"stock\""));
}

// ---------------------------------------------------------------------------
// Constraints on existing tables
// ---------------------------------------------------------------------------

#[test]
fn postgres_adds_check_and_foreign_key_with_native_alter() {
    let parents = table("parents", vec![primary_key("id", "i32")]);
    let mut children = table("children", vec![primary_key("id", "i32")]);
    let mut parent_id = column("parent_id", "i32");
    parent_id.is_foreign_key = true;
    parent_id.fk_referenced_table = Some("parents".to_string());
    parent_id.fk_referenced_column = Some("id".to_string());
    parent_id.fk_on_delete = Some("CASCADE".to_string());
    children.columns.push(parent_id);
    children.checks.push(SnapshotCheck {
        name: "ck_children_parent_id".to_string(),
        expression: "parent_id > 0".to_string(),
    });

    let mut live_children = live_table(
        "children",
        vec![
            db_primary_key("id", "integer"),
            db_column("parent_id", "integer", true),
        ],
    );
    live_children.checks = Some(Vec::new());
    let live = vec![
        live_table("parents", vec![db_primary_key("id", "integer")]),
        live_children,
    ];
    let diff = diff(
        MigrationDialect::Postgres,
        &model(vec![parents, children]),
        &live,
    );

    let sql = diff.up_sql();
    assert!(
        sql.contains(
            "ALTER TABLE \"children\" ADD CONSTRAINT \"ck_children_parent_id\" CHECK (parent_id > 0)"
        ),
        "missing native ADD CHECK:\n{sql}"
    );
    assert!(
        sql.contains(
            "ALTER TABLE \"children\" ADD CONSTRAINT \"fk_children_parent_id_parents\" FOREIGN KEY (\"parent_id\") REFERENCES \"parents\" (\"id\") ON DELETE CASCADE"
        ),
        "missing native ADD FOREIGN KEY:\n{sql}"
    );
    assert!(
        !has_warning(&diff, WarningKind::UnsupportedChange),
        "PostgreSQL supports the change natively: {:?}",
        diff.warnings
    );
}

#[test]
fn sqlite_check_constraint_on_existing_table_reports_rebuild_required() {
    let mut widgets = table("widgets", vec![primary_key("id", "i32")]);
    widgets.columns.push(column("stock", "i32"));
    widgets.checks.push(SnapshotCheck {
        name: "ck_widgets_stock".to_string(),
        expression: "stock >= 0".to_string(),
    });

    let mut live_widgets = live_table(
        "widgets",
        vec![
            db_primary_key("id", "INTEGER"),
            db_column("stock", "INTEGER", true),
        ],
    );
    live_widgets.checks = Some(Vec::new());
    let live = vec![live_widgets];
    let diff = diff(MigrationDialect::Sqlite, &model(vec![widgets]), &live);

    assert!(
        has_warning(&diff, WarningKind::UnsupportedChange),
        "expected rebuild warning: {:?}",
        diff.warnings
    );
    assert!(
        diff.up_sql().contains("rebuild") || diff.warnings.iter().any(|w| w.message.contains("rebuild")),
        "warning must explain the rebuild: {:?}",
        diff.warnings
    );
    assert!(
        !diff.up_sql().contains("ADD CONSTRAINT"),
        "must not emit invalid SQLite SQL:\n{}",
        diff.up_sql()
    );
}

#[test]
fn postgres_adds_check_only_when_checks_were_introspected() {
    let mut widgets = table("widgets", vec![primary_key("id", "i32")]);
    widgets.checks.push(SnapshotCheck {
        name: "ck_widgets_stock".to_string(),
        expression: "stock >= 0".to_string(),
    });

    // `checks: None` = the provider could not introspect CHECK constraints.
    let mut live = live_table("widgets", vec![db_primary_key("id", "integer")]);
    live.checks = None;
    let diff = diff(
        MigrationDialect::Postgres,
        &model(vec![widgets]),
        &[live],
    );

    assert!(
        has_warning(&diff, WarningKind::ConstraintNotIntrospected),
        "expected introspection warning: {:?}",
        diff.warnings
    );
    assert!(
        !diff.up_sql().contains("ADD CONSTRAINT"),
        "must not add a check that cannot be verified:\n{}",
        diff.up_sql()
    );
}

#[test]
fn postgres_skips_check_already_present() {
    let mut widgets = table("widgets", vec![primary_key("id", "i32")]);
    widgets.checks.push(SnapshotCheck {
        name: "ck_widgets_stock".to_string(),
        expression: "stock >= 0".to_string(),
    });

    let mut live = live_table("widgets", vec![db_primary_key("id", "integer")]);
    live.checks = Some(vec![DbCheck {
        name: Some("ck_widgets_stock".to_string()),
        expression: "stock >= 0".to_string(),
    }]);
    let diff = diff(
        MigrationDialect::Postgres,
        &model(vec![widgets]),
        &[live],
    );

    assert!(
        diff.is_empty(),
        "already-present check must not be re-added:\n{}",
        diff.up_sql()
    );
    assert!(diff.warnings.is_empty(), "unexpected warnings: {:?}", diff.warnings);
}

#[test]
fn sqlite_check_match_by_expression_when_constraint_name_is_unknown() {
    let mut widgets = table("widgets", vec![primary_key("id", "i32")]);
    widgets.checks.push(SnapshotCheck {
        name: "ck_widgets_stock".to_string(),
        expression: "stock >= 0".to_string(),
    });

    // SQLite does not expose CHECK names: the provider matches by expression.
    let mut live = live_table("widgets", vec![db_primary_key("id", "INTEGER")]);
    live.checks = Some(vec![DbCheck {
        name: None,
        expression: "stock >= 0".to_string(),
    }]);
    let diff = diff(MigrationDialect::Sqlite, &model(vec![widgets]), &[live]);

    assert!(
        diff.is_empty(),
        "matching CHECK expression must be treated as present:\n{}",
        diff.up_sql()
    );
}

// ---------------------------------------------------------------------------
// Destructive / unknown changes
// ---------------------------------------------------------------------------

#[test]
fn extra_live_objects_are_reported_not_dropped() {
    let posts = table("posts", vec![primary_key("id", "i32")]);
    let mut live_posts = live_table(
        "posts",
        vec![
            db_primary_key("id", "INTEGER"),
            db_column("legacy_flag", "INTEGER", true),
        ],
    );
    live_posts.indexes.push(DbIndex {
        name: "ix_posts_legacy_flag".to_string(),
        columns: vec![DbIndexColumn {
            value: "legacy_flag".to_string(),
            is_expression: false,
        }],
        ..DbIndex::default()
    });
    let live = vec![live_posts, live_table("legacy_audit", vec![])];

    let diff = diff(MigrationDialect::Sqlite, &model(vec![posts]), &live);

    assert!(diff.is_empty(), "no statements expected:\n{}", diff.up_sql());
    let skipped = diff
        .warnings
        .iter()
        .filter(|warning| warning.kind == WarningKind::SkippedRemoval)
        .count();
    assert_eq!(
        skipped, 3,
        "expected column + index + table removal warnings: {:?}",
        diff.warnings
    );
    assert!(
        diff.warnings
            .iter()
            .all(|warning| warning.severity == WarningSeverity::Info),
        "removals are informational: {:?}",
        diff.warnings
    );
}

#[test]
fn matching_schema_is_a_no_op() {
    let items = table(
        "items",
        vec![primary_key("id", "i32"), column("name", "String")],
    );
    let mut live_items = live_table(
        "items",
        vec![
            db_primary_key("id", "INTEGER"),
            db_column("name", "TEXT", true),
        ],
    );
    live_items.checks = Some(Vec::new());
    live_items.triggers = Some(Vec::new());
    let diff = diff(MigrationDialect::Sqlite, &model(vec![items]), &[live_items]);

    assert!(diff.is_empty(), "unexpected statements:\n{}", diff.up_sql());
    assert!(diff.warnings.is_empty(), "unexpected warnings: {:?}", diff.warnings);
}

#[test]
fn sqlite_column_default_change_reports_rebuild_required() {
    let mut widgets = table("widgets", vec![primary_key("id", "i32")]);
    let mut status = column("status", "String");
    status.default_value = Some("'active'".to_string());
    widgets.columns.push(status);

    let live = vec![live_table(
        "widgets",
        vec![
            db_primary_key("id", "INTEGER"),
            db_column("status", "TEXT", true),
        ],
    )];
    let diff = diff(MigrationDialect::Sqlite, &model(vec![widgets]), &live);

    assert!(
        has_warning(&diff, WarningKind::UnsupportedChange),
        "default change requires a rebuild: {:?}",
        diff.warnings
    );
}

#[test]
fn implicit_primary_key_index_is_not_reported_as_extra() {
    let posts = table("posts", vec![primary_key("id", "i32")]);
    let mut live_posts = live_table("posts", vec![db_primary_key("id", "INTEGER")]);
    live_posts.indexes.push(DbIndex {
        name: "sqlite_autoindex_posts_1".to_string(),
        columns: vec![DbIndexColumn {
            value: "id".to_string(),
            is_expression: false,
        }],
        is_unique: true,
        ..DbIndex::default()
    });
    let diff = diff(
        MigrationDialect::Sqlite,
        &model(vec![posts]),
        &[live_posts],
    );

    assert!(
        diff.warnings.is_empty(),
        "implicit PK index must be ignored: {:?}",
        diff.warnings
    );
}

// ---------------------------------------------------------------------------
// Report / MySQL specifics
// ---------------------------------------------------------------------------

#[test]
fn report_lists_statements_and_warnings() {
    let mut widgets = table("widgets", vec![primary_key("id", "i32")]);
    widgets.checks.push(SnapshotCheck {
        name: "ck_widgets_stock".to_string(),
        expression: "stock >= 0".to_string(),
    });
    let mut live = vec![live_table("widgets", vec![db_primary_key("id", "INTEGER")])];
    live[0].checks = Some(Vec::new());
    let diff = diff(MigrationDialect::Sqlite, &model(vec![widgets]), &live);

    let report = diff.report();
    assert!(report.contains("rust-ef runtime schema diff"), "{report}");
    assert!(report.contains("warnings: 1"), "{report}");
    assert!(report.contains("unsupported_change"), "{report}");
}

#[test]
fn mysql_uses_backtick_quotas_for_new_index() {
    let mut items = table("items", vec![primary_key("id", "i32")]);
    let mut name = column("name", "String");
    name.max_length = Some(64);
    name.has_index = true;
    items.columns.push(name);

    let diff = diff(MigrationDialect::MySql, &model(vec![items]), &[]);
    let sql = diff.up_sql();
    assert!(
        sql.contains("CREATE INDEX `ix_items_name` ON `items` (`name`)"),
        "missing MySQL-quoted index:\n{sql}"
    );
}

// ---------------------------------------------------------------------------
// Foreign-key matching
// ---------------------------------------------------------------------------

#[test]
fn existing_foreign_key_is_not_added_twice() {
    let parents = table("parents", vec![primary_key("id", "i32")]);
    let mut children = table("children", vec![primary_key("id", "i32")]);
    let mut parent_id = column("parent_id", "i32");
    parent_id.is_foreign_key = true;
    parent_id.fk_referenced_table = Some("parents".to_string());
    parent_id.fk_referenced_column = Some("id".to_string());
    children.columns.push(parent_id);

    let mut live_children = live_table(
        "children",
        vec![
            db_primary_key("id", "integer"),
            db_column("parent_id", "integer", true),
        ],
    );
    live_children.foreign_keys.push(DbForeignKey {
        name: None,
        columns: vec!["parent_id".to_string()],
        referenced_table: "parents".to_string(),
        referenced_columns: vec!["id".to_string()],
        ..DbForeignKey::default()
    });
    let live = vec![
        live_table("parents", vec![db_primary_key("id", "integer")]),
        live_children,
    ];
    let diff = diff(
        MigrationDialect::Postgres,
        &model(vec![parents, children]),
        &live,
    );

    assert!(
        diff.is_empty(),
        "existing FK must not be added twice:\n{}",
        diff.up_sql()
    );
}
