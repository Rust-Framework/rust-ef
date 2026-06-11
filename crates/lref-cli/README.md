# lref-cli

[![Crates.io](https://img.shields.io/crates/v/lref-cli)](https://crates.io/crates/lref-cli)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

CLI tool for [Rust Entity Framework (lref)](https://crates.io/crates/lref).  
Provides migration management and database scaffolding commands, analogous to .NET's `dotnet ef`.

---

## Installation

```bash
cargo install lref-cli
```

The binary is named `lref`.

---

## Commands

### Migration Management

| Command | EFCore Equivalent | Description |
|---------|-------------------|-------------|
| `lref migration init` | — | Initialize migrations directory and history tracking |
| `lref migration add <name>` | `dotnet ef migrations add <name>` | Create a new migration with `up.sql` / `down.sql` |
| `lref migration apply` | `dotnet ef database update` | Apply pending migrations |
| `lref migration revert` | `dotnet ef database update <prev>` | Revert the last applied migration |
| `lref migration list` | `dotnet ef migrations list` | List all migrations with status |
| `lref migration script` | `dotnet ef migrations script` | Generate combined SQL script |

### Scaffolding

| Command | EFCore Equivalent | Description |
|---------|-------------------|-------------|
| `lref scaffold-dbcontext -c <conn> -p <provider> -o <dir>` | `dotnet ef dbcontext scaffold` | Generate entity types and DbContext from an existing database |

---

## Quick Start

```bash
# Initialize migrations
lref migration init

# Create first migration
lref migration add InitialCreate
# → migrations/20260611120000_InitialCreate/
#     ├── up.sql
#     └── down.sql

# Write your SQL in up.sql, then apply
lref migration apply

# List migrations
lref migration list
#   [Applied] 20260611120000_InitialCreate
#   [Pending] 20260611130000_AddRatingColumn

# Revert
lref migration revert
```

### Scaffold from existing PostgreSQL database

```bash
lref scaffold-dbcontext \
  -c "postgres://user:pass@localhost/mydb" \
  -p postgres \
  -o src/entities

# Generates:
#   src/entities/
#     ├── blogs.rs      (one file per table)
#     ├── posts.rs
#     └── context.rs    (DbContext implementation)
```

---

## Migration Directory Structure

```
migrations/
├── .history                              ← tracking file (migration names, one per line)
├── 20260611120000_InitialCreate/
│   ├── up.sql                            ← CREATE TABLE statements
│   └── down.sql                          ← DROP TABLE statements
└── 20260611130000_AddRatingColumn/
    ├── up.sql
    └── down.sql
```

---

## License

MIT — see [LICENSE](../../LICENSE)
