//! Database provider abstraction trait.
//!
//! Corresponds to EFCore's database provider model, allowing multiple
//! database backends (PostgreSQL, MySQL, SQLite, etc.) to be plugged in.
//!
//! `DbValue` and `DbValueConvertError` live in `db_value.rs`; `TryFrom<DbValue>`
//! impls in `db_value_convert.rs`; provider traits (`ISqlGenerator`,
//! `IAsyncConnection`, `IDatabaseProvider`) in `traits.rs`.

mod db_value;
mod db_value_convert;
mod db_value_key;
mod traits;

pub use db_value::{DbValue, DbValueConvertError};
pub use db_value_key::DbValueKey;
pub use traits::{IAsyncConnection, IDatabaseProvider, ISqlGenerator, IsolationLevel};
