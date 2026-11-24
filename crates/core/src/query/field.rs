//! Typed column handles for query composition.
//!
//! `#[derive(EntityType)]` emits one `Field` constant per persisted scalar
//! field, named after the Rust field:
//!
//! ```ignore
//! let page = ctx.set::<Order>().query()
//!     .order_by_field(Order::created_at)
//!     .then_by_field(Order::id)
//!     .to_page(&paging)
//!     .await?;
//! ```
//!
//! The handle carries a compile-time column name and the field's Rust type, so a
//! typo or a renamed field is a compile error at the call site instead of a
//! runtime SQL error. The constructor is doc-hidden: only the derive (and the
//! framework's own macros) should create handles; writing
//! `Field::from_column_unchecked("...")` manually bypasses the safety and is
//! deliberately not part of the public API.
//!
//! `Sortable` is the future compile-time restriction for columns that cannot be
//! ordered (blobs/JSON); the type parameter `V` is reserved for it.

use std::marker::PhantomData;

/// A persisted column handle: the column name plus the Rust field type.
///
/// Created by `#[derive(EntityType)]`; see the [module docs](self).
pub struct Field<E, V> {
    column: &'static str,
    _marker: PhantomData<fn() -> (E, V)>,
}

impl<E, V> Field<E, V> {
    /// Wrap a column name. Hidden: generated code only.
    #[doc(hidden)]
    pub const fn from_column_unchecked(column: &'static str) -> Self {
        Self {
            column,
            _marker: PhantomData,
        }
    }

    /// The physical column name to pass to the query compiler.
    pub const fn column(&self) -> &'static str {
        self.column
    }

    /// Alias for [`Self::column`] for call sites that read better as `name()`.
    pub const fn name(&self) -> &'static str {
        self.column
    }
}

impl<E, V> Copy for Field<E, V> {}
impl<E, V> Clone for Field<E, V> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<E, V> PartialEq for Field<E, V> {
    fn eq(&self, other: &Self) -> bool {
        self.column == other.column
    }
}
impl<E, V> Eq for Field<E, V> {}
impl<E, V> std::fmt::Debug for Field<E, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Field").field(&self.column).finish()
    }
}
