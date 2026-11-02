use crate::{
    ActiveModelBehavior, ActiveModelTrait, ConnectionTrait, EntityTrait, Error, ExpectedColumn,
    IntoActiveModel, Linked, QueryFilter, QueryResult, Related, Select, SelectModel, SelectorRaw,
};
use async_trait::async_trait;
pub use pgorm_query::Value;
use pgorm_query::Values;
use std::fmt::Debug;

/// A Trait for a Model
// [spec:pgorm:def:entity.traits.model+3]
#[async_trait]
pub trait ModelTrait: Clone + Send + Debug {
    #[allow(missing_docs)]
    type Entity: EntityTrait;

    /// Get the [Value] of a column from an Entity
    fn get(&self, c: <Self::Entity as EntityTrait>::Column) -> Value;

    /// Set the [Value] of a column in an Entity, reporting a column this model
    /// does not carry, or a value of the wrong type for it, as [`Error::Type`].
    fn set(&mut self, c: <Self::Entity as EntityTrait>::Column, v: Value) -> Result<(), Error>;

    /// Find related Models
    fn find_related<R>(&self, _: R) -> Select<R>
    where
        R: EntityTrait,
        Self::Entity: Related<R>,
    {
        <Self::Entity as Related<R>>::find_related().belongs_to(self)
    }

    /// Find linked Models
    // [spec:pgorm:req:entity.relation.linked+4]
    fn find_linked<L>(&self, l: L) -> Select<L::ToEntity>
    where
        L: Linked<FromEntity = Self::Entity>,
    {
        let tbl_alias = l.last_hop_alias();
        l.find_linked().belongs_to_tbl_alias(self, tbl_alias)
    }

    /// Convert this model into its entity's `ActiveModel`.
    ///
    /// [`IntoActiveModel`] is generic over the destination, so it needs the
    /// caller to name the target type. The entity already knows which
    /// `ActiveModel` belongs to it, so this conversion does not:
    ///
    /// ```
    /// use pgorm::entity::*;
    /// use pgorm::tests_cfg::fruit;
    ///
    /// let model = fruit::Model {
    ///     id: 1,
    ///     name: "Orange".to_owned(),
    ///     cake_id: None,
    /// };
    ///
    /// let mut am = model.into_active();
    /// am.name = set("Apple");
    /// assert!(am.is_changed());
    /// ```
    fn into_active(self) -> <Self::Entity as EntityTrait>::ActiveModel
    where
        Self: IntoActiveModel<<Self::Entity as EntityTrait>::ActiveModel>,
    {
        self.into_active_model()
    }

    /// Delete a model
    async fn delete<'a, A, C>(self, db: &'a C) -> Result<u64, Error>
    where
        Self: IntoActiveModel<A>,
        C: ConnectionTrait,
        A: ActiveModelTrait<Entity = Self::Entity> + ActiveModelBehavior + Send + 'a,
    {
        self.into_active_model().delete(db).await
    }
}

/// A Trait for implementing a [QueryResult]
// [spec:pgorm:def:entity.traits.from-query-result+5]
pub trait FromQueryResult: Sized {
    /// Instantiate a Model from a [QueryResult]
    fn from_query_result(res: &QueryResult, pre: &str) -> Result<Self, Error>;

    /// Instantiate a Model from a row that may not carry one, telling an
    /// absent row apart from a present one that fails to decode.
    ///
    /// This is how the joined decode reads the related side: a `LEFT JOIN`
    /// that matched nothing still yields a row, one whose right-hand columns
    /// are all `NULL`. That shape, and only that shape, is `Ok(None)`.
    ///
    /// The witness is the set of columns this type reads under `pre`:
    ///
    /// - When [`expected_columns`](FromQueryResult::expected_columns) reports
    ///   them, the row counts as absent only if every one of them is a column
    ///   of the result set and holds SQL `NULL`. A column the statement never
    ///   projected is a projection mistake rather than an absent row, so its
    ///   error propagates.
    /// - A type that reports no columns is judged against the result set
    ///   instead: absent only if the row carries at least one column named
    ///   with `pre` and every such column is `NULL`.
    ///
    /// Every other decode failure propagates — a column of the wrong type, an
    /// unrecognised enum label, a malformed JSON payload. A related row that
    /// is present but undecodable is an error, never a missing row.
    ///
    /// A present row whose witness columns are genuinely all `NULL` is
    /// indistinguishable from an unmatched outer join in the result set, and
    /// is reported absent. An entity's primary key is `NOT NULL`, so this only
    /// arises for a projection that leaves the key out.
    // [spec:pgorm:req:exec.decode.absent]
    fn from_query_result_optional(res: &QueryResult, pre: &str) -> Result<Option<Self>, Error> {
        // The witness is judged before decoding: a projection of nothing but
        // nullable fields decodes an all-`NULL` row successfully, and that
        // shape is still an absent row, not a value.
        let absent = match Self::expected_columns() {
            Some(cols) => res.all_null(pre, cols.iter().map(ExpectedColumn::name)),
            None => res.all_null_under(pre),
        };
        if absent {
            return Ok(None);
        }
        Self::from_query_result(res, pre).map(Some)
    }

    /// The columns [`from_query_result`](FromQueryResult::from_query_result)
    /// reads, in the order it reads them, or [`None`] when this type does not
    /// report them.
    ///
    /// The `FromQueryResult` derive fills this in from the struct's fields —
    /// skipped fields read no column and are left out — so that
    /// [`VerifyStatement::verify`](crate::VerifyStatement::verify) can check a
    /// statement's result columns against the type before a row exists. The
    /// default answers [`None`]: a hand-written implementation decodes by means
    /// only it knows, so it is reported as unverifiable rather than treated as
    /// verified. Overriding this method opts such an implementation back in.
    // [spec:pgorm:def:exec.verify]    the column-shape reflection hook
    fn expected_columns() -> Option<Vec<ExpectedColumn>> {
        None
    }

    /// Run a raw statement and decode every row into `Self`.
    ///
    /// A statement that returns no rows decodes into an empty `Vec` whatever
    /// `Self` looks like, so a mismatched target only fails once data arrives.
    /// [`VerifyStatement::verify`](crate::VerifyStatement::verify) checks the
    /// same statement against `Self` at prepare time instead.
    ///
    /// ```no_run
    /// # #[cfg(feature = "macros")]
    /// # {
    /// # use pgorm::{error::*, query::*, DatabasePool, FromQueryResult};
    /// #
    /// # async fn example(pool: &DatabasePool) -> Result<(), Error> {
    /// #[derive(Debug, PartialEq, FromQueryResult)]
    /// struct SelectResult {
    ///     name: String,
    ///     num_of_cakes: i64,
    /// }
    ///
    /// let db = pool.get().await?;
    ///
    /// let res: Vec<SelectResult> = SelectResult::find_by_statement(
    ///     r#"SELECT "name", COUNT(*) AS "num_of_cakes" FROM "cake" GROUP BY("name")"#,
    ///     vec![],
    /// )
    /// .all(&db)
    /// .await?;
    /// # Ok(())
    /// # }
    /// # }
    /// ```
    fn find_by_statement(
        stmt: impl Into<String>,
        values: Vec<Value>,
    ) -> SelectorRaw<SelectModel<Self>> {
        SelectorRaw::<SelectModel<Self>>::from_statement(stmt.into(), Values(values))
    }
}

/// A Trait for any type that can be converted into an Model
pub trait TryIntoModel<M>
where
    M: ModelTrait,
{
    /// Method to call to perform the conversion
    fn try_into_model(self) -> Result<M, Error>;
}

impl<M> TryIntoModel<M> for M
where
    M: ModelTrait,
{
    fn try_into_model(self) -> Result<M, Error> {
        Ok(self)
    }
}
