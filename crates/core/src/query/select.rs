//! `SelectQueryBuilder<T>` — projection query builder.
//!
//! Produced by `QueryBuilder::select_internal` when the user projects to
//! named columns. Executes the projection query and returns either raw
//! `Vec<Vec<DbValue>>` rows or strongly-typed tuples via `to_list_typed_n`
//! methods (converting each column via `TryFrom<DbValue>`).

use std::marker::PhantomData;
use std::sync::Arc;

use crate::entity::IEntityType;
use crate::error::EFResult;
use crate::provider::{DbValue, DbValueConvertError, IDatabaseProvider};

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
    pub async fn to_list(self) -> EFResult<Vec<Vec<DbValue>>> {
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::configuration(
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
    // query, then converts each column value via `TryFrom<DbValue>` into the
    // corresponding type parameter, returning `Vec<(V0, ...)>`.
    // -------------------------------------------------------------------

    async fn fetch_rows(self) -> EFResult<Vec<Vec<DbValue>>> {
        self.to_list().await
    }

    /// Single-column typed projection → `Vec<V0>`.
    pub async fn to_list_typed_1<V0>(self) -> EFResult<Vec<V0>>
    where
        V0: TryFrom<DbValue, Error = DbValueConvertError>,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                let cell = row
                    .first()
                    .ok_or_else(|| crate::error::EFError::query("projection row has no columns"))?;
                V0::try_from(cell.clone()).map_err(crate::error::EFError::from)
            })
            .collect()
    }

    /// Two-column typed projection → `Vec<(V0, V1)>`.
    pub async fn to_list_typed_2<V0, V1>(self) -> EFResult<Vec<(V0, V1)>>
    where
        V0: TryFrom<DbValue, Error = DbValueConvertError>,
        V1: TryFrom<DbValue, Error = DbValueConvertError>,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                let c0 = row.first().ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 0")
                })?;
                let c1 = row.get(1).ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 1")
                })?;
                Ok((
                    V0::try_from(c0.clone()).map_err(crate::error::EFError::from)?,
                    V1::try_from(c1.clone()).map_err(crate::error::EFError::from)?,
                ))
            })
            .collect()
    }

    /// Three-column typed projection → `Vec<(V0, V1, V2)>`.
    pub async fn to_list_typed_3<V0, V1, V2>(self) -> EFResult<Vec<(V0, V1, V2)>>
    where
        V0: TryFrom<DbValue, Error = DbValueConvertError>,
        V1: TryFrom<DbValue, Error = DbValueConvertError>,
        V2: TryFrom<DbValue, Error = DbValueConvertError>,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                let c0 = row.first().ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 0")
                })?;
                let c1 = row.get(1).ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 1")
                })?;
                let c2 = row.get(2).ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 2")
                })?;
                Ok((
                    V0::try_from(c0.clone()).map_err(crate::error::EFError::from)?,
                    V1::try_from(c1.clone()).map_err(crate::error::EFError::from)?,
                    V2::try_from(c2.clone()).map_err(crate::error::EFError::from)?,
                ))
            })
            .collect()
    }

    /// Four-column typed projection → `Vec<(V0, V1, V2, V3)>`.
    pub async fn to_list_typed_4<V0, V1, V2, V3>(self) -> EFResult<Vec<(V0, V1, V2, V3)>>
    where
        V0: TryFrom<DbValue, Error = DbValueConvertError>,
        V1: TryFrom<DbValue, Error = DbValueConvertError>,
        V2: TryFrom<DbValue, Error = DbValueConvertError>,
        V3: TryFrom<DbValue, Error = DbValueConvertError>,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                let c0 = row.first().ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 0")
                })?;
                let c1 = row.get(1).ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 1")
                })?;
                let c2 = row.get(2).ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 2")
                })?;
                let c3 = row.get(3).ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 3")
                })?;
                Ok((
                    V0::try_from(c0.clone()).map_err(crate::error::EFError::from)?,
                    V1::try_from(c1.clone()).map_err(crate::error::EFError::from)?,
                    V2::try_from(c2.clone()).map_err(crate::error::EFError::from)?,
                    V3::try_from(c3.clone()).map_err(crate::error::EFError::from)?,
                ))
            })
            .collect()
    }
}
