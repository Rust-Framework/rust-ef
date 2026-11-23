//! Typed row materialization for projection / raw-SQL results.
//!
//! [`IRow`] is the result-row counterpart of [`IFromRow`]: it materializes a
//! value from positional column values without requiring the target to be an
//! entity. Three kinds of types implement it:
//!
//! - every entity (`T: IFromRow`) through the blanket impl — entities keep
//!   using their generated `from_row` path;
//! - `#[derive(FromRow)]` structs, for report/raw-SQL rows;
//! - tuples `(T1, ...)` of 1..=12 elements.
//!
//! Mapping is positional: the `SELECT` list order must match the struct field /
//! tuple element order. Providers return values without column names, so named
//! matching is only available for entities via
//! [`IFromRow::from_row_named`].

use crate::entity::IFromRow;
use crate::error::{EFError, EFResult};
use crate::provider::{DbValue, DbValueConvertError};

/// A result-row type that can be materialized from positional column values.
pub trait IRow: Sized {
    /// Materializes `Self` from one row of positional column values.
    ///
    /// Implementations must return a typed [`EFError::TypeConversion`] when the
    /// row is too short or a value cannot be converted.
    fn from_values(values: &[DbValue]) -> EFResult<Self>;
}

/// Every entity is a valid result row: delegate to its generated `from_row`.
impl<T: IFromRow> IRow for T {
    fn from_values(values: &[DbValue]) -> EFResult<Self> {
        T::from_row(values)
    }
}

/// Materializes every row with [`IRow::from_values`], preserving row order.
pub fn materialize_rows<R: IRow>(rows: &[Vec<DbValue>]) -> EFResult<Vec<R>> {
    rows.iter().map(|row| R::from_values(row)).collect()
}

/// Implements [`IRow`] for tuples of 1..=12 elements.
///
/// Each element is converted with `TryFrom<DbValue>`; a row whose length does
/// not match the tuple arity is rejected with a typed `EFError::TypeConversion`
/// naming the tuple type and the expected/actual column counts.
macro_rules! impl_tuple_row {
    ($($index:tt => $name:ident),+) => {
        impl<$($name),+> IRow for ($($name,)+)
        where
            $($name: TryFrom<DbValue, Error = DbValueConvertError>),+
        {
            fn from_values(values: &[DbValue]) -> EFResult<Self> {
                let expected = [$($index),+].len();
                let tuple_name = stringify!($($name),+);
                if values.len() != expected {
                    return Err(EFError::type_conversion(format!(
                        "row has {} value(s) but tuple `({})` needs exactly {}",
                        values.len(),
                        tuple_name,
                        expected
                    )));
                }
                Ok(($(
                    $name::try_from(values[$index].clone()).map_err(|error| {
                        EFError::type_conversion(format!(
                            "tuple `({})` column {} cannot be converted: {}",
                            tuple_name, $index, error
                        ))
                    })?,
                )+))
            }
        }
    };
}

impl_tuple_row!(0 => T1);
impl_tuple_row!(0 => T1, 1 => T2);
impl_tuple_row!(0 => T1, 1 => T2, 2 => T3);
impl_tuple_row!(0 => T1, 1 => T2, 2 => T3, 3 => T4);
impl_tuple_row!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5);
impl_tuple_row!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6);
impl_tuple_row!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7);
impl_tuple_row!(
    0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7, 7 => T8
);
impl_tuple_row!(
    0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7, 7 => T8, 8 => T9
);
impl_tuple_row!(
    0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7, 7 => T8, 8 => T9, 9 => T10
);
impl_tuple_row!(
    0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7, 7 => T8, 8 => T9, 9 => T10,
    10 => T11
);
impl_tuple_row!(
    0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7, 7 => T8, 8 => T9, 9 => T10,
    10 => T11, 11 => T12
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuple_maps_values_by_position() {
        let row = [DbValue::I32(7), DbValue::String("north".into())];
        let mapped = <(i32, String) as IRow>::from_values(&row).expect("tuple row");
        assert_eq!(mapped, (7, "north".to_string()));
    }

    #[test]
    fn single_element_tuple_is_supported() {
        let mapped = <(i64,) as IRow>::from_values(&[DbValue::I64(42)]).expect("one-tuple row");
        assert_eq!(mapped.0, 42);
    }

    #[test]
    fn short_row_is_a_type_conversion_error() {
        let err =
            <(i32, i32) as IRow>::from_values(&[DbValue::I32(1)]).expect_err("short row must fail");
        match err {
            EFError::TypeConversion(message, _) => {
                assert!(message.contains("needs exactly 2"), "unexpected: {message}");
            }
            other => panic!("expected TypeConversion, got {other:?}"),
        }
    }

    #[test]
    fn long_row_is_a_type_conversion_error() {
        let err = <(i32,) as IRow>::from_values(&[DbValue::I32(1), DbValue::I32(2)])
            .expect_err("long row must fail");
        assert!(matches!(err, EFError::TypeConversion(..)));
    }

    #[test]
    fn unconvertible_cell_is_a_type_conversion_error_naming_the_index() {
        let err =
            <(i32, i32) as IRow>::from_values(&[DbValue::I32(1), DbValue::String("x".into())])
                .expect_err("bad cell must fail");
        match err {
            EFError::TypeConversion(message, _) => {
                assert!(message.contains("column 1"), "unexpected: {message}");
            }
            other => panic!("expected TypeConversion, got {other:?}"),
        }
    }
}
