//! `SelectQueryBuilder<T>` — projection query builder.
//!
//! Produced by `QueryBuilder::select_internal` when the user projects to
//! named columns. Executes the projection query and returns either raw
//! `Vec<Vec<String>>` rows or strongly-typed tuples via `to_list_typed_n`
//! methods (parsing each column via `ParseFromDb`).

use std::marker::PhantomData;
use std::sync::Arc;

use crate::entity::IEntityType;
use crate::error::EFResult;
use crate::provider::IDatabaseProvider;

use super::source::{parse_column, ParseFromDb};
use super::state::QueryState;

/// A query builder for projected column results.
#[derive(Clone)]
pub struct SelectQueryBuilder<T: IEntityType> {
    pub(crate) state: QueryState,
    pub(crate) provider: Option<Arc<dyn IDatabaseProvider>>,
    pub(crate) _phantom: PhantomData<T>,
}

impl<T: IEntityType> SelectQueryBuilder<T> {
    /// Returns the generated SQL.
    pub fn to_sql(&self) -> String {
        if let Some(provider) = &self.provider {
            let gen = provider.sql_generator();
            self.state.to_sql_with(gen)
        } else {
            self.state.to_sql()
        }
    }

    /// Executes the projection query and returns raw column values per row.
    pub async fn to_list(self) -> EFResult<Vec<Vec<String>>> {
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to SelectQueryBuilder.".to_string(),
            )
        })?;
        let gen = provider.sql_generator();
        let sql = self.state.to_sql_with(gen);
        let params = self.state.all_params();
        let mut conn = provider.get_connection().await?;
        conn.query(&sql, &params).await
    }

    // -------------------------------------------------------------------
    // G3: Strongly-typed projection terminal methods.
    //
    // Each `to_list_typed_n::<V0, ...>` method executes the projection
    // query, then parses each column value via `ParseFromDb` into the
    // corresponding type parameter, returning `Vec<(V0, ...)>`.
    // -------------------------------------------------------------------

    async fn fetch_rows(self) -> EFResult<Vec<Vec<String>>> {
        self.to_list().await
    }

    /// Single-column typed projection → `Vec<V0>`.
    pub async fn to_list_typed_1<V0>(self) -> EFResult<Vec<V0>>
    where
        V0: ParseFromDb,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                parse_column::<V0>(row.first().ok_or_else(|| {
                    crate::error::EFError::Query("projection row has no columns".into())
                })?)
            })
            .collect()
    }

    /// Two-column typed projection → `Vec<(V0, V1)>`.
    pub async fn to_list_typed_2<V0, V1>(self) -> EFResult<Vec<(V0, V1)>>
    where
        V0: ParseFromDb,
        V1: ParseFromDb,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                let c0 = row.first().ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 0".into())
                })?;
                let c1 = row.get(1).ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 1".into())
                })?;
                Ok((parse_column::<V0>(c0)?, parse_column::<V1>(c1)?))
            })
            .collect()
    }

    /// Three-column typed projection → `Vec<(V0, V1, V2)>`.
    pub async fn to_list_typed_3<V0, V1, V2>(self) -> EFResult<Vec<(V0, V1, V2)>>
    where
        V0: ParseFromDb,
        V1: ParseFromDb,
        V2: ParseFromDb,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                let c0 = row.first().ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 0".into())
                })?;
                let c1 = row.get(1).ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 1".into())
                })?;
                let c2 = row.get(2).ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 2".into())
                })?;
                Ok((
                    parse_column::<V0>(c0)?,
                    parse_column::<V1>(c1)?,
                    parse_column::<V2>(c2)?,
                ))
            })
            .collect()
    }

    /// Four-column typed projection → `Vec<(V0, V1, V2, V3)>`.
    pub async fn to_list_typed_4<V0, V1, V2, V3>(self) -> EFResult<Vec<(V0, V1, V2, V3)>>
    where
        V0: ParseFromDb,
        V1: ParseFromDb,
        V2: ParseFromDb,
        V3: ParseFromDb,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                let c0 = row.first().ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 0".into())
                })?;
                let c1 = row.get(1).ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 1".into())
                })?;
                let c2 = row.get(2).ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 2".into())
                })?;
                let c3 = row.get(3).ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 3".into())
                })?;
                Ok((
                    parse_column::<V0>(c0)?,
                    parse_column::<V1>(c1)?,
                    parse_column::<V2>(c2)?,
                    parse_column::<V3>(c3)?,
                ))
            })
            .collect()
    }
}
