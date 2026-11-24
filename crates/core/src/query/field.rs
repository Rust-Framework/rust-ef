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
//! [`Sortable`] is the compile-time restriction for columns that can be ordered.
//! It is implemented for scalar values (`i*`, `u*`, floats, `bool`, `String`),
//! `Option<T: Sortable>`, and the feature-gated `chrono` / `uuid` / `decimal`
//! types. Blobs (`Vec<u8>`), JSON values and arbitrary structs deliberately do
//! not implement it, so `order_by_field(Entity::payload)` fails to compile.

use std::marker::PhantomData;

/// Marker for field types that may appear in `ORDER BY`.
///
/// The blanket-looking set is explicit on purpose: a blob/JSON column must not
/// be ordered even though SQLite would accept it, and a blanket impl would make
/// every type sortable. See the [module docs](self).
///
/// ```compile_fail
/// use rust_ef::prelude::*;
///
/// #[derive(EntityType)]
/// #[table("blobs")]
/// struct Blob {
///     #[primary_key]
///     id: i64,
///     payload: Vec<u8>,
/// }
///
/// fn order_blob(query: rust_ef::query::QueryBuilder<Blob>) {
///     query.order_by_field(Blob::payload); // Vec<u8> is not Sortable
/// }
/// ```
pub trait Sortable {}

macro_rules! sortable_scalars {
    ($($ty:ty),* $(,)?) => {
        $(impl Sortable for $ty {})*
    };
}

sortable_scalars!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64, bool, char, String,
);

impl<T: Sortable> Sortable for Option<T> {}

#[cfg(feature = "chrono")]
impl<Tz: chrono::TimeZone> Sortable for chrono::DateTime<Tz> {}
#[cfg(feature = "chrono")]
impl Sortable for chrono::NaiveDateTime {}
#[cfg(feature = "chrono")]
impl Sortable for chrono::NaiveDate {}
#[cfg(feature = "chrono")]
impl Sortable for chrono::NaiveTime {}

#[cfg(feature = "uuid")]
impl Sortable for uuid::Uuid {}

#[cfg(feature = "decimal")]
impl Sortable for rust_decimal::Decimal {}

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
