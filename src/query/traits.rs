use pgorm_query::{QueryStatementBuilder, Values};

/// A Trait for any type performing queries on a Model or ActiveModel
// [spec:pgorm:req:query.build+2]
// [spec:pgorm:def:query.build.query-trait+2]
pub trait QueryTrait {
    /// Constrain the QueryStatement to [QueryStatementBuilder] trait
    type QueryStatement: QueryStatementBuilder;

    /// Get a mutable ref to the query builder
    fn query(&mut self) -> &mut Self::QueryStatement;

    /// Get an immutable ref to the query builder
    fn as_query(&self) -> &Self::QueryStatement;

    /// Take ownership of the query builder
    fn into_query(self) -> Self::QueryStatement;

    /// Render the statement as PostgreSQL text plus its bound parameters.
    ///
    /// Values are never inlined: each one becomes a `$n` placeholder in the
    /// SQL string and an entry in [`Values`], in binding order. Use
    /// [`as_query`](QueryTrait::as_query) with `to_string` instead when you
    /// want a self-contained, value-inlined string to read.
    ///
    /// # Example
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    /// use pgorm::pgorm_query::{Value, Values};
    ///
    /// let (sql, values) = cake::Entity::find()
    ///     .filter(cake::Column::Id.eq(3))
    ///     .build();
    ///
    /// assert_eq!(
    ///     sql,
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = $1"#
    /// );
    /// assert_eq!(values, Values(vec![Value::Int(Some(3))]));
    /// ```
    fn build(&self) -> (String, Values) {
        self.as_query().build()
    }

    /// Apply an operation on the [QueryTrait::QueryStatement] if the given `Option<T>` is `Some(_)`
    ///
    /// # Example
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     cake::Entity::find()
    ///         .apply_if(Some(3), |mut query, v| {
    ///             query.filter(cake::Column::Id.eq(v))
    ///         })
    ///         .apply_if(Some(100), QuerySelect::limit)
    ///         .apply_if(None, QuerySelect::offset::<Option<u64>>) // no-op
    ///         .as_query()
    ///         .to_string(),
    ///     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = 3 LIMIT 100"#
    /// );
    /// ```
    fn apply_if<T, F>(self, val: Option<T>, if_some: F) -> Self
    where
        Self: Sized,
        F: FnOnce(Self, T) -> Self,
    {
        if let Some(val) = val {
            if_some(self, val)
        } else {
            self
        }
    }
}
