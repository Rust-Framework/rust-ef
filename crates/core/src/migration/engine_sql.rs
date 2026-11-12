//! `MigrationEngine` SQL generation — Up/Down DDL from `SchemaChange` lists.

use super::ddl_expr::quote_string;
use super::dialect::MigrationDialect;
use super::diff::index_name;
use super::engine::MigrationEngine;
use super::history::{MIGRATION_HISTORY_TABLE, PRODUCT_VERSION};
use super::schema_change::SchemaChange;
use super::types::{
    ModelSnapshot, SnapshotCheck, SnapshotColumn, SnapshotForeignKey, SnapshotIndex,
    SnapshotIndexColumn, SnapshotTableOptions,
};

impl MigrationEngine {
    pub(crate) fn initial_create_with_fks(&self, current: &ModelSnapshot) -> Vec<SchemaChange> {
        let mut changes = self.initial_create(current);
        for et in current.entity_types.iter().filter(|et| !et.is_view) {
            // SQLite foreign keys are emitted inline in CREATE TABLE because
            // SQLite does not support ALTER TABLE ADD CONSTRAINT.
            if self.dialect != MigrationDialect::Sqlite {
                Self::append_create_table_fks(&mut changes, &et.table_name, &et.columns);
                Self::append_composite_fks(&mut changes, et);
            }
            Self::append_create_table_indexes(&mut changes, &et.table_name, &et.columns);
            Self::append_declared_indexes(&mut changes, et);
        }
        changes
    }

    /// Renders `DEFAULT <expr>` for a column, if configured.
    fn default_clause(&self, col: &SnapshotColumn) -> String {
        match &col.default_value {
            Some(v) if !v.trim().is_empty() => format!(" DEFAULT {}", v),
            _ => String::new(),
        }
    }

    fn collation_clause(&self, col: &SnapshotColumn) -> String {
        match &col.collation {
            Some(c) if !c.trim().is_empty() => format!(" COLLATE {}", c.trim()),
            _ => String::new(),
        }
    }

    /// Renders a full column definition for CREATE TABLE / ADD COLUMN / MySQL
    /// MODIFY COLUMN, including generated columns, collation, default and
    /// (MySQL) inline comment.
    fn render_column_definition(&self, col: &SnapshotColumn) -> String {
        let q = |s: &str| self.dialect.quote(s);
        let (col_type, mysql_auto_increment) = self.column_type(col);

        // Generated/computed columns: no NULL/NOT NULL or DEFAULT.
        if let Some(expr) = &col.computed_sql {
            if expr.trim().is_empty() {
                // fall through to a normal column
            } else {
                let stored = match self.dialect {
                    // PostgreSQL only supports STORED generated columns.
                    MigrationDialect::Postgres => true,
                    _ => col.computed_stored,
                };
                let storage = if stored { " STORED" } else { " VIRTUAL" };
                let comment = if self.dialect == MigrationDialect::MySql {
                    self.comment_clause(col)
                } else {
                    String::new()
                };
                return format!(
                    "{} {} GENERATED ALWAYS AS ({}){}{}",
                    q(&col.column_name),
                    col_type,
                    expr,
                    storage,
                    comment
                );
            }
        }

        // COLLATE is part of the data type specification in PostgreSQL,
        // MySQL and SQLite, so it must precede NULL/NOT NULL.
        let collation = self.collation_clause(col);
        let col_type = if collation.is_empty() {
            col_type
        } else {
            format!("{}{}", col_type, collation)
        };

        let nullable = if col.is_required || col.is_primary_key {
            "NOT NULL"
        } else {
            "NULL"
        };

        // PostgreSQL sequence column: DEFAULT nextval('seq')
        if col.is_sequence && self.dialect == MigrationDialect::Postgres {
            if let Some(seq_name) = &col.sequence_name {
                return format!(
                    "{} {} DEFAULT nextval('{}') {}",
                    q(&col.column_name),
                    col_type,
                    seq_name,
                    nullable
                );
            }
        }

        let mut parts = vec![q(&col.column_name), col_type, nullable.to_string()];
        if !mysql_auto_increment {
            let default = self.default_clause(col);
            if !default.is_empty() {
                parts.push(default.trim_start().to_string());
            }
        }
        if mysql_auto_increment {
            parts.push("AUTO_INCREMENT".to_string());
        }
        if self.dialect == MigrationDialect::MySql {
            let comment = self.comment_clause(col);
            if !comment.is_empty() {
                parts.push(comment.trim_start().to_string());
            }
        }
        parts.join(" ")
    }

    /// MySQL inline `COMMENT '...'` clause.
    fn comment_clause(&self, col: &SnapshotColumn) -> String {
        match &col.comment {
            Some(c) if !c.is_empty() => format!(" COMMENT {}", quote_string(c, self.dialect)),
            _ => String::new(),
        }
    }

    /// PostgreSQL column comments (separate statements).
    fn column_comment_sql(&self, table: &str, col: &SnapshotColumn) -> Option<String> {
        if self.dialect != MigrationDialect::Postgres {
            return None;
        }
        let comment = col.comment.as_deref()?;
        let q = |s: &str| self.dialect.quote(s);
        Some(format!(
            "COMMENT ON COLUMN {}.{} IS {};\n",
            q(table),
            q(&col.column_name),
            quote_string(comment, self.dialect)
        ))
    }

    fn alter_table_comment_sql(&self, table: &str, comment: Option<&str>) -> String {
        let q = |s: &str| self.dialect.quote(s);
        match self.dialect {
            MigrationDialect::Sqlite => {
                "-- WARNING: SQLite does not support table comments\n".to_string()
            }
            MigrationDialect::Postgres => match comment {
                Some(c) => format!(
                    "COMMENT ON TABLE {} IS {};\n",
                    q(table),
                    quote_string(c, self.dialect)
                ),
                None => format!("COMMENT ON TABLE {} IS NULL;\n", q(table)),
            },
            MigrationDialect::MySql => match comment {
                Some(c) => format!(
                    "ALTER TABLE {} COMMENT = {};\n",
                    q(table),
                    quote_string(c, self.dialect)
                ),
                None => format!("ALTER TABLE {} COMMENT = '';\n", q(table)),
            },
        }
    }

    fn table_comment_sql(&self, table: &str, comment: Option<&str>) -> Option<String> {
        let comment = comment?;
        let q = |s: &str| self.dialect.quote(s);
        match self.dialect {
            MigrationDialect::Postgres => Some(format!(
                "COMMENT ON TABLE {} IS {};\n",
                q(table),
                quote_string(comment, self.dialect)
            )),
            MigrationDialect::MySql => Some(format!(
                "ALTER TABLE {} COMMENT = {};\n",
                q(table),
                quote_string(comment, self.dialect)
            )),
            MigrationDialect::Sqlite => None,
        }
    }

    /// Renders the MySQL table-options suffix (`ENGINE=... DEFAULT CHARSET=...`).
    ///
    /// MySQL always gets `ENGINE`/`DEFAULT CHARSET` so tables do not silently
    /// inherit an instance default (e.g. latin1). PostgreSQL/SQLite ignore
    /// table options.
    fn table_options_suffix(&self, options: &SnapshotTableOptions) -> String {
        if self.dialect != MigrationDialect::MySql {
            return String::new();
        }
        let engine = options.engine.as_deref().unwrap_or("InnoDB");
        let charset = options.charset.as_deref().unwrap_or("utf8mb4");
        let mut suffix = format!(" ENGINE={engine} DEFAULT CHARSET={charset}");
        if let Some(collate) = &options.collate {
            suffix.push_str(&format!(" COLLATE={collate}"));
        }
        suffix
    }

    /// Renders a declared index key part for a CREATE INDEX statement.
    fn render_index_key_part(&self, part: &SnapshotIndexColumn) -> String {
        if part.is_expression {
            format!("({})", part.value)
        } else {
            self.dialect.quote(&part.value)
        }
    }

    /// Renders a `CREATE INDEX` statement for a declared index.
    ///
    /// MySQL has no partial indexes: a filtered UNIQUE index is emulated with a
    /// functional `CASE WHEN <filter> THEN <col> END` unique index (only rows
    /// matching the predicate produce non-NULL key parts; NULLs are not
    /// compared in a unique index). A filtered non-unique index is skipped with
    /// a warning comment.
    pub(super) fn create_declared_index_sql(&self, table: &str, index: &SnapshotIndex) -> String {
        let q = |s: &str| self.dialect.quote(s);
        let unique_kw = if index.is_unique { "UNIQUE " } else { "" };
        let name = q(&index.name);

        // INCLUDE is PostgreSQL-only; other dialects ignore it with a warning.
        let (include_warning, include_suffix) = if index.include.is_empty() {
            (String::new(), String::new())
        } else {
            match self.dialect {
                MigrationDialect::Postgres => {
                    let cols: Vec<String> = index.include.iter().map(|c| q(c)).collect();
                    (String::new(), format!(" INCLUDE ({})", cols.join(", ")))
                }
                _ => (
                    "-- WARNING: index INCLUDE is only supported by PostgreSQL; ignored\n"
                        .to_string(),
                    String::new(),
                ),
            }
        };

        match (&index.filter, self.dialect) {
            (Some(filter), MigrationDialect::MySql) if index.is_unique => {
                let parts: Vec<String> = index
                    .columns
                    .iter()
                    .map(|part| {
                        let value = if part.is_expression {
                            part.value.clone()
                        } else {
                            q(&part.value)
                        };
                        format!("(CASE WHEN {filter} THEN {value} END)")
                    })
                    .collect();
                format!(
                    "{include_warning}-- MySQL has no partial indexes; emulating filtered UNIQUE index with a functional index\nCREATE UNIQUE INDEX {name} ON {} ({});\n",
                    q(table),
                    parts.join(", ")
                )
            }
            (Some(_), MigrationDialect::MySql) => format!(
                "-- WARNING: MySQL does not support partial non-unique indexes; {name} on {} skipped\n",
                q(table)
            ),
            (Some(filter), _) => {
                let cols: Vec<String> = index
                    .columns
                    .iter()
                    .map(|part| self.render_index_key_part(part))
                    .collect();
                format!(
                    "{include_warning}CREATE {unique_kw}INDEX {name} ON {} ({}){include_suffix} WHERE {filter};\n",
                    q(table),
                    cols.join(", ")
                )
            }
            (None, _) => {
                let cols: Vec<String> = index
                    .columns
                    .iter()
                    .map(|part| self.render_index_key_part(part))
                    .collect();
                format!(
                    "{include_warning}CREATE {unique_kw}INDEX {name} ON {} ({}){include_suffix};\n",
                    q(table),
                    cols.join(", ")
                )
            }
        }
    }

    fn composite_fk_clause(&self, fk: &SnapshotForeignKey) -> String {
        let q = |s: &str| self.dialect.quote(s);
        let cols: Vec<String> = fk.columns.iter().map(|c| q(c)).collect();
        let ref_cols: Vec<String> = fk.referenced_columns.iter().map(|c| q(c)).collect();
        let on_delete = fk
            .on_delete
            .as_deref()
            .map(|c| format!(" ON DELETE {}", c))
            .unwrap_or_default();
        format!(
            "CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}){}",
            q(&fk.name),
            cols.join(", "),
            q(&fk.referenced_table),
            ref_cols.join(", "),
            on_delete
        )
    }

    fn create_composite_fk_sql(&self, table: &str, fk: &SnapshotForeignKey) -> String {
        if self.dialect == MigrationDialect::Sqlite {
            return format!(
                "-- WARNING: SQLite cannot add composite foreign key {} to an existing table; rebuild {} to apply it\n",
                self.dialect.quote(&fk.name),
                self.dialect.quote(table)
            );
        }
        format!(
            "ALTER TABLE {} ADD {};\n",
            self.dialect.quote(table),
            self.composite_fk_clause(fk)
        )
    }

    fn add_check_sql(&self, table: &str, check: &SnapshotCheck) -> String {
        let q = |s: &str| self.dialect.quote(s);
        match self.dialect {
            MigrationDialect::Sqlite => format!(
                "-- WARNING: SQLite cannot add CHECK constraint {} to an existing table; rebuild the table to apply it\n",
                q(&check.name)
            ),
            _ => format!(
                "ALTER TABLE {} ADD CONSTRAINT {} CHECK ({});\n",
                q(table),
                q(&check.name),
                check.expression
            ),
        }
    }

    fn drop_check_sql(&self, table: &str, check: &SnapshotCheck) -> String {
        let q = |s: &str| self.dialect.quote(s);
        match self.dialect {
            MigrationDialect::Sqlite => format!(
                "-- WARNING: SQLite cannot drop CHECK constraint {}; rebuild the table to remove it\n",
                q(&check.name)
            ),
            MigrationDialect::MySql => format!(
                "ALTER TABLE {} DROP CHECK {};\n",
                q(table),
                q(&check.name)
            ),
            MigrationDialect::Postgres => format!(
                "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};\n",
                q(table),
                q(&check.name)
            ),
        }
    }

    // -----------------------------------------------------------------------
    // SQL generation
    // -----------------------------------------------------------------------

    /// Standard foreign-key constraint name used by Add/DropForeignKey.
    ///
    /// Dots from schema-qualified tables are flattened so the name stays a
    /// single identifier.
    pub fn foreign_key_name(table: &str, column: &str, referenced_table: &str) -> String {
        format!(
            "fk_{}_{}_{}",
            table.replace('.', "_"),
            column,
            referenced_table.replace('.', "_")
        )
    }

    /// Standard index name used by CreateIndex/DropIndex.
    pub fn index_name(table: &str, column: &str) -> String {
        format!("ix_{}_{}", table.replace('.', "_"), column)
    }

    /// Returns the column type and whether MySQL `AUTO_INCREMENT` must be
    /// emitted after the nullability clause.
    ///
    /// MySQL's column grammar is `type [NOT NULL|NULL] ... [AUTO_INCREMENT]`;
    /// emitting `INT AUTO_INCREMENT NOT NULL` makes the primary key column
    /// nullable (error 1171).
    fn column_type(&self, col: &SnapshotColumn) -> (String, bool) {
        let col_type = self.dialect.map_column_type(col);
        if self.dialect == MigrationDialect::MySql && (col.is_auto_increment || col.is_sequence) {
            let stripped = col_type
                .strip_suffix(" AUTO_INCREMENT")
                .unwrap_or(&col_type)
                .to_string();
            (stripped, true)
        } else {
            (col_type, false)
        }
    }

    pub fn generate_alter_column_sql(
        &self,
        table: &str,
        column_name: &str,
        new: &SnapshotColumn,
    ) -> String {
        let q = |s: &str| self.dialect.quote(s);
        let col_type = self.dialect.map_column_type(new);
        match self.dialect {
            MigrationDialect::Postgres => {
                if new.computed_sql.is_some() {
                    return format!(
                        "-- WARNING: PostgreSQL cannot alter a generated column expression; drop and re-add {}.{} manually\n",
                        q(table),
                        q(column_name)
                    );
                }
                let nullable = if new.is_required || new.is_primary_key {
                    "SET NOT NULL"
                } else {
                    "DROP NOT NULL"
                };
                let default = match &new.default_value {
                    Some(v) if !v.trim().is_empty() => format!(
                        "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {};\n",
                        q(table),
                        q(column_name),
                        v
                    ),
                    _ => format!(
                        "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT;\n",
                        q(table),
                        q(column_name)
                    ),
                };
                let collation = match &new.collation {
                    Some(c) if !c.trim().is_empty() => format!(" COLLATE {}", c.trim()),
                    _ => String::new(),
                };
                let mut sql = format!(
                    "ALTER TABLE {} ALTER COLUMN {} TYPE {}{};\nALTER TABLE {} ALTER COLUMN {} {};\n{}",
                    q(table),
                    q(column_name),
                    col_type,
                    collation,
                    q(table),
                    q(column_name),
                    nullable,
                    default
                );
                if let Some(stmt) = self.column_comment_sql(table, new) {
                    sql.push_str(&stmt);
                }
                sql
            }
            MigrationDialect::MySql => format!(
                "ALTER TABLE {} MODIFY COLUMN {};\n",
                q(table),
                self.render_column_definition(new)
            ),
            MigrationDialect::Sqlite => format!(
                "-- SQLite does not support ALTER COLUMN type changes; \
                 rebuild table manually for {}.{}\n",
                q(table),
                q(column_name)
            ),
        }
    }

    pub(crate) fn generate_up_sql(&self, changes: &[SchemaChange]) -> String {
        self.generate_up_sql_inner(changes, true)
    }

    pub(crate) fn generate_ddl_sql(&self, changes: &[SchemaChange]) -> String {
        self.generate_up_sql_inner(changes, false)
    }

    fn generate_up_sql_inner(&self, changes: &[SchemaChange], record_history: bool) -> String {
        let mut sql = String::from("-- Up Migration (auto-generated by rust-ef)\n\n");
        let q = |s: &str| self.dialect.quote(s);
        let create_kw = if record_history {
            "CREATE TABLE"
        } else {
            "CREATE TABLE IF NOT EXISTS"
        };

        for change in changes {
            match change {
                SchemaChange::CreateTable {
                    table,
                    columns,
                    checks,
                    table_options,
                    foreign_keys,
                    comment,
                } => {
                    // PostgreSQL: create sequences before the table
                    if self.dialect == MigrationDialect::Postgres {
                        for c in columns {
                            if c.is_sequence {
                                if let Some(seq_name) = &c.sequence_name {
                                    sql.push_str(&format!(
                                        "CREATE SEQUENCE IF NOT EXISTS {};\n",
                                        q(seq_name)
                                    ));
                                }
                            }
                        }
                    }

                    sql.push_str(&format!("{} {} (\n", create_kw, q(table)));

                    // Separate primary key columns from regular columns
                    let pk_columns: Vec<&str> = columns
                        .iter()
                        .filter(|c| c.is_primary_key)
                        .map(|c| c.column_name.as_str())
                        .collect();

                    let col_defs: Vec<String> = columns
                        .iter()
                        .map(|c| self.render_column_definition(c))
                        .collect();
                    sql.push_str(&format!("    {}\n", col_defs.join(",\n    ")));

                    // Composite primary key constraint
                    if pk_columns.len() == 1 {
                        sql.push_str(&format!("    ,PRIMARY KEY ({})\n", q(pk_columns[0])));
                    } else if pk_columns.len() > 1 {
                        let pk_list: Vec<String> = pk_columns.iter().map(|c| q(c)).collect();
                        sql.push_str(&format!("    ,PRIMARY KEY ({})\n", pk_list.join(", ")));
                    }

                    // Table-level CHECK constraints
                    for check in checks {
                        sql.push_str(&format!(
                            "    ,CONSTRAINT {} CHECK ({})\n",
                            q(&check.name),
                            check.expression
                        ));
                    }

                    // SQLite: inline foreign keys (it cannot ALTER ADD
                    // CONSTRAINT). Other dialects add FKs after all tables
                    // exist via ALTER TABLE.
                    //
                    // `ensure_created` (playbook: idempotent DDL only, no
                    // indexes/FKs) intentionally skips this; only real
                    // migration scripts get FK enforcement.
                    if record_history && self.dialect == MigrationDialect::Sqlite {
                        for (col, ref_table, ref_col) in columns
                            .iter()
                            .filter_map(|c| super::diff::fk_target(c).map(|(t, r)| (c, t, r)))
                        {
                            let fk_name =
                                Self::foreign_key_name(table, &col.column_name, &ref_table);
                            let on_delete = col
                                .fk_on_delete
                                .as_deref()
                                .map(|c| format!(" ON DELETE {}", c))
                                .unwrap_or_default();
                            sql.push_str(&format!(
                                "    ,CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}){}\n",
                                q(&fk_name),
                                q(&col.column_name),
                                q(&ref_table),
                                q(&ref_col),
                                on_delete
                            ));
                        }
                        for fk in foreign_keys {
                            sql.push_str(&format!("    ,{}\n", self.composite_fk_clause(fk)));
                        }
                    }

                    sql.push(')');
                    sql.push_str(&self.table_options_suffix(table_options));
                    sql.push_str(";\n\n");

                    // Comments: MySQL table comments are emitted as ALTER (to
                    // keep the CREATE statement simple); PostgreSQL uses
                    // separate COMMENT ON statements; SQLite has no comments.
                    if let Some(stmt) = self.table_comment_sql(table, comment.as_deref()) {
                        sql.push_str(&stmt);
                    }
                    for c in columns {
                        if let Some(stmt) = self.column_comment_sql(table, c) {
                            sql.push_str(&stmt);
                        }
                    }
                }
                SchemaChange::DropTable { table } => {
                    sql.push_str(&format!(
                        "-- WARNING: dropping table {} is irreversible\nDROP TABLE IF EXISTS {};\n",
                        q(table),
                        q(table)
                    ));
                }
                SchemaChange::AddColumn { table, column } => {
                    if column.is_required
                        && !column.is_primary_key
                        && column.default_value.is_none()
                        && column.computed_sql.is_none()
                    {
                        sql.push_str(&format!(
                            "-- WARNING: adding NOT NULL column {} without a default fails on non-empty tables; provide has_default(...) or backfill first\n",
                            q(&column.column_name)
                        ));
                    }
                    sql.push_str(&format!(
                        "ALTER TABLE {} ADD COLUMN {};\n",
                        q(table),
                        self.render_column_definition(column)
                    ));
                    if let Some(stmt) = self.column_comment_sql(table, column) {
                        sql.push_str(&stmt);
                    }
                }
                SchemaChange::DropColumn { table, column_name } => {
                    sql.push_str(&format!(
                        "-- WARNING: dropping column {}.{} is irreversible; column renames are not detected (data loss)\n",
                        q(table),
                        q(column_name)
                    ));
                    sql.push_str(&format!(
                        "ALTER TABLE {} DROP COLUMN IF EXISTS {};\n",
                        q(table),
                        q(column_name)
                    ));
                }
                SchemaChange::AlterColumn {
                    table,
                    column_name,
                    old: _,
                    new,
                } => {
                    sql.push_str(&self.generate_alter_column_sql(table, column_name, new));
                }
                SchemaChange::AddForeignKey {
                    table,
                    column,
                    referenced_table,
                    referenced_column,
                    on_delete,
                } => {
                    if self.dialect == MigrationDialect::Sqlite {
                        sql.push_str(&format!(
                            "-- WARNING: SQLite cannot add a foreign key to an existing table; rebuild {} to add ({}) -> {}({})\n",
                            q(table),
                            q(column),
                            q(referenced_table),
                            q(referenced_column)
                        ));
                    } else {
                        let fk_name = Self::foreign_key_name(table, column, referenced_table);
                        let on_delete_clause = on_delete
                            .as_deref()
                            .map(|c| format!(" ON DELETE {}", c))
                            .unwrap_or_default();
                        sql.push_str(&format!(
                            "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}){};\n",
                            q(table),
                            q(&fk_name),
                            q(column),
                            q(referenced_table),
                            q(referenced_column),
                            on_delete_clause
                        ));
                    }
                }
                SchemaChange::DropForeignKey {
                    table,
                    column,
                    referenced_table,
                } => {
                    let fk_name = Self::foreign_key_name(table, column, referenced_table);
                    match self.dialect {
                        MigrationDialect::Sqlite => sql.push_str(&format!(
                            "-- WARNING: SQLite cannot drop a foreign key from an existing table; rebuild {} to remove {}\n",
                            q(table),
                            q(&fk_name)
                        )),
                        MigrationDialect::MySql => sql.push_str(&format!(
                            "ALTER TABLE {} DROP FOREIGN KEY {};\n",
                            q(table),
                            q(&fk_name)
                        )),
                        _ => sql.push_str(&format!(
                            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};\n",
                            q(table),
                            q(&fk_name)
                        )),
                    }
                }
                SchemaChange::CreateIndex {
                    table,
                    column,
                    is_unique,
                } => {
                    let unique_kw = if *is_unique { "UNIQUE " } else { "" };
                    let idx_name = index_name(table, column);
                    sql.push_str(&format!(
                        "CREATE {unique_kw}INDEX {} ON {} ({});\n",
                        q(&idx_name),
                        q(table),
                        q(column)
                    ));
                }
                SchemaChange::DropIndex {
                    table,
                    column,
                    is_unique: _,
                } => {
                    let idx_name = index_name(table, column);
                    match self.dialect {
                        MigrationDialect::MySql => {
                            sql.push_str(&format!("DROP INDEX {} ON {};\n", q(&idx_name), q(table)))
                        }
                        _ => sql.push_str(&format!("DROP INDEX IF EXISTS {};\n", q(&idx_name))),
                    }
                }
                SchemaChange::CreateDeclaredIndex { table, index } => {
                    sql.push_str(&self.create_declared_index_sql(table, index));
                }
                SchemaChange::DropDeclaredIndex { table, index } => match self.dialect {
                    MigrationDialect::MySql => {
                        sql.push_str(&format!("DROP INDEX {} ON {};\n", q(&index.name), q(table)))
                    }
                    _ => sql.push_str(&format!("DROP INDEX IF EXISTS {};\n", q(&index.name))),
                },
                SchemaChange::AddCompositeForeignKey { table, foreign_key } => {
                    sql.push_str(&self.create_composite_fk_sql(table, foreign_key));
                }
                SchemaChange::DropCompositeForeignKey { table, foreign_key } => {
                    match self.dialect {
                        MigrationDialect::Sqlite => sql.push_str(&format!(
                            "-- WARNING: SQLite cannot drop composite foreign key {} from an existing table; rebuild {} to remove it\n",
                            q(&foreign_key.name),
                            q(table)
                        )),
                        MigrationDialect::MySql => sql.push_str(&format!(
                            "ALTER TABLE {} DROP FOREIGN KEY {};\n",
                            q(table),
                            q(&foreign_key.name)
                        )),
                        _ => sql.push_str(&format!(
                            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};\n",
                            q(table),
                            q(&foreign_key.name)
                        )),
                    }
                }
                SchemaChange::AddCheck { table, check } => {
                    sql.push_str(&self.add_check_sql(table, check));
                }
                SchemaChange::DropCheck { table, check } => {
                    sql.push_str(&self.drop_check_sql(table, check));
                }
                SchemaChange::AlterTableOptions { table, options } => {
                    if self.dialect == MigrationDialect::MySql {
                        let engine = options.engine.as_deref().unwrap_or("InnoDB");
                        let charset = options.charset.as_deref().unwrap_or("utf8mb4");
                        sql.push_str(&format!(
                            "ALTER TABLE {} ENGINE={} DEFAULT CHARSET={}",
                            q(table),
                            engine,
                            charset
                        ));
                        if let Some(collate) = &options.collate {
                            sql.push_str(&format!(" COLLATE={}", collate));
                        }
                        sql.push_str(";\n");
                    } else {
                        sql.push_str(&format!(
                            "-- Table options on {} are MySQL-only; ignored for this dialect\n",
                            q(table)
                        ));
                    }
                }
                SchemaChange::AlterTableComment {
                    table,
                    old_comment: _,
                    new_comment,
                } => {
                    sql.push_str(&self.alter_table_comment_sql(table, new_comment.as_deref()));
                }
            }
        }

        if record_history {
            sql.push_str(&format!(
                "INSERT INTO {}(migration_id, product_version) VALUES ('{{migration_id}}', '{}');\n",
                q(MIGRATION_HISTORY_TABLE),
                PRODUCT_VERSION
            ));
        }

        sql
    }

    pub(super) fn generate_down_sql(&self, changes: &[SchemaChange]) -> String {
        let mut sql = String::from("-- Down Migration (auto-generated by rust-ef)\n\n");
        let q = |s: &str| self.dialect.quote(s);

        // Reverse the changes for the down migration
        for change in changes.iter().rev() {
            match change {
                SchemaChange::CreateTable { table, .. } => {
                    sql.push_str(&format!("DROP TABLE IF EXISTS {};\n", q(table)));
                }
                SchemaChange::DropTable { table } => {
                    sql.push_str(&format!(
                        "-- WARNING: Cannot restore table {} (original schema unknown)\n",
                        q(table)
                    ));
                }
                SchemaChange::AddColumn { table, column } => {
                    sql.push_str(&format!(
                        "ALTER TABLE {} DROP COLUMN IF EXISTS {};\n",
                        q(table),
                        q(&column.column_name)
                    ));
                }
                SchemaChange::DropColumn { table, column_name } => {
                    sql.push_str(&format!(
                        "-- WARNING: Cannot restore column {} on {} (original type unknown)\n",
                        q(column_name),
                        q(table)
                    ));
                }
                SchemaChange::AlterColumn {
                    table,
                    column_name,
                    old,
                    new: _,
                } => {
                    sql.push_str(&self.generate_alter_column_sql(table, column_name, old));
                }
                SchemaChange::AddForeignKey {
                    table,
                    column,
                    referenced_table,
                    ..
                } => {
                    let fk_name = Self::foreign_key_name(table, column, referenced_table);
                    match self.dialect {
                        MigrationDialect::Sqlite => sql.push_str(&format!(
                            "-- WARNING: SQLite cannot drop a foreign key from an existing table; rebuild {} to remove {}\n",
                            q(table),
                            q(&fk_name)
                        )),
                        MigrationDialect::MySql => sql.push_str(&format!(
                            "ALTER TABLE {} DROP FOREIGN KEY {};\n",
                            q(table),
                            q(&fk_name)
                        )),
                        _ => sql.push_str(&format!(
                            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};\n",
                            q(table),
                            q(&fk_name)
                        )),
                    }
                }
                SchemaChange::DropForeignKey {
                    table,
                    column,
                    referenced_table,
                    ..
                } => {
                    sql.push_str(&format!(
                        "-- WARNING: Cannot restore foreign key constraint {} on {}.{}\n",
                        q(&Self::foreign_key_name(table, column, referenced_table)),
                        q(table),
                        q(column)
                    ));
                }
                SchemaChange::CreateIndex {
                    table,
                    column,
                    is_unique: _,
                } => {
                    let idx_name = index_name(table, column);
                    match self.dialect {
                        MigrationDialect::MySql => {
                            sql.push_str(&format!("DROP INDEX {} ON {};\n", q(&idx_name), q(table)))
                        }
                        _ => sql.push_str(&format!("DROP INDEX IF EXISTS {};\n", q(&idx_name))),
                    }
                }
                SchemaChange::DropIndex {
                    table,
                    column,
                    is_unique,
                } => {
                    let unique_kw = if *is_unique { "UNIQUE " } else { "" };
                    let idx_name = index_name(table, column);
                    sql.push_str(&format!(
                        "CREATE {unique_kw}INDEX {} ON {} ({});\n",
                        q(&idx_name),
                        q(table),
                        q(column)
                    ));
                }
                SchemaChange::CreateDeclaredIndex { table, index } => match self.dialect {
                    MigrationDialect::MySql => {
                        sql.push_str(&format!("DROP INDEX {} ON {};\n", q(&index.name), q(table)))
                    }
                    _ => sql.push_str(&format!("DROP INDEX IF EXISTS {};\n", q(&index.name))),
                },
                SchemaChange::DropDeclaredIndex { table, index } => {
                    sql.push_str(&self.create_declared_index_sql(table, index));
                }
                SchemaChange::AddCompositeForeignKey { table, foreign_key } => match self.dialect {
                    MigrationDialect::Sqlite => sql.push_str(&format!(
                        "-- WARNING: SQLite cannot drop composite foreign key {} from an existing table; rebuild {} to remove it\n",
                        q(&foreign_key.name),
                        q(table)
                    )),
                    MigrationDialect::MySql => sql.push_str(&format!(
                        "ALTER TABLE {} DROP FOREIGN KEY {};\n",
                        q(table),
                        q(&foreign_key.name)
                    )),
                    _ => sql.push_str(&format!(
                        "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};\n",
                        q(table),
                        q(&foreign_key.name)
                    )),
                },
                SchemaChange::DropCompositeForeignKey { table, foreign_key } => {
                    sql.push_str(&self.create_composite_fk_sql(table, foreign_key));
                }
                SchemaChange::AddCheck { table, check } => {
                    sql.push_str(&self.drop_check_sql(table, check));
                }
                SchemaChange::DropCheck { table, check } => {
                    sql.push_str(&self.add_check_sql(table, check));
                }
                SchemaChange::AlterTableOptions { .. } => {
                    sql.push_str(
                        "-- WARNING: Table option changes cannot be reverted automatically\n",
                    );
                }
                SchemaChange::AlterTableComment {
                    table,
                    old_comment,
                    new_comment: _,
                } => {
                    sql.push_str(&self.alter_table_comment_sql(table, old_comment.as_deref()));
                }
            }
        }

        sql.push_str(&format!(
            "DELETE FROM {} WHERE migration_id = '{{migration_id}}';\n",
            q(MIGRATION_HISTORY_TABLE)
        ));

        sql
    }
}

/// Renders a single-quoted SQL string literal.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::schema_change::SchemaChange;
    use crate::migration::types::SnapshotColumn;

    #[test]
    fn mysql_auto_increment_follows_nullability() {
        let engine = MigrationEngine::new(MigrationDialect::MySql);
        let column = SnapshotColumn {
            column_name: "id".into(),
            type_name: "i32".into(),
            is_primary_key: true,
            is_required: true,
            is_auto_increment: true,
            ..Default::default()
        };
        let sql = engine.generate_ddl_sql(&[SchemaChange::CreateTable {
            table: "items".into(),
            columns: vec![column],
            checks: Vec::new(),
            table_options: Default::default(),
            foreign_keys: Vec::new(),
            comment: None,
        }]);
        assert!(
            sql.contains("`id` INT NOT NULL AUTO_INCREMENT"),
            "unexpected DDL: {sql}"
        );
        assert!(
            !sql.contains("INT AUTO_INCREMENT NOT NULL"),
            "unexpected DDL: {sql}"
        );
    }

    #[test]
    fn primary_key_columns_are_not_null_even_without_required() {
        let engine = MigrationEngine::new(MigrationDialect::Postgres);
        let column = SnapshotColumn {
            column_name: "id".into(),
            type_name: "i32".into(),
            is_primary_key: true,
            is_required: false,
            is_auto_increment: true,
            ..Default::default()
        };
        let sql = engine.generate_ddl_sql(&[SchemaChange::CreateTable {
            table: "items".into(),
            columns: vec![column],
            checks: Vec::new(),
            table_options: Default::default(),
            foreign_keys: Vec::new(),
            comment: None,
        }]);
        assert!(
            sql.contains("\"id\" SERIAL NOT NULL"),
            "unexpected DDL: {sql}"
        );
    }
}
