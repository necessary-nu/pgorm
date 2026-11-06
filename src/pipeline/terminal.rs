//! The fallible boundary: pipeline → SQL → decoded rows.

use pgorm_query::Values;

use crate::{
    ConnectionTrait, DecodeRaw, Error, FromQueryResult, SelectGetableTuple, SelectModel,
    SelectorRaw, TryGetableMany,
};

use super::adapter;
use super::builder::Pipeline;
use super::census;
use super::error::{PipelineError, RESERVED, unquotable};
use super::sources::typed_cast;

impl Pipeline {
    /// Compile the pipeline to PostgreSQL SQL and the values bound along the
    /// way, in placeholder order.
    ///
    /// This is where everything fallible happens — identifier screening,
    /// reserved-alias screening, then prqlc's name resolution and lowering —
    /// and it fails as a [`PipelineError`], never a panic. The optimizer may
    /// prune an expression nothing reads, placeholder and all, so the emitted
    /// SQL is censused afterwards: values whose placeholders were optimized
    /// away are discarded and the survivors renumber contiguously, keeping
    /// position `N` in the SQL aligned with position `N` in the values.
    ///
    /// The identifier screen is first and is the reason this boundary, rather
    /// than the four constructors that mint identifiers, is where names are
    /// judged: every table, schema segment, column and alias the pipeline
    /// carries passes through here on its way to prqlc, so a name the
    /// compiler cannot be trusted to write as a name — one carrying `"` or
    /// NUL, one beginning with `$`, or `*` — is caught whichever stage
    /// introduced it, including stages written after the screen was. Because the refusal is pgorm's own and
    /// returns before the compiler is called at all, it does not depend on
    /// which prqlc the build resolved, which no dependency pgorm declares can
    /// guarantee for a downstream consumer that patches it.
    // [spec:pgorm:req:pipeline.errors+4]
    // [spec:pgorm:req:pipeline.params+4]
    pub fn into_sql(mut self) -> Result<(String, Values), PipelineError> {
        self.screen(true)?;
        let Some(columns) = self.read.take() else {
            return self.compile();
        };
        let nodes = columns
            .into_iter()
            .map(|(name, cast)| {
                let node = adapter::ident_in(vec!["this".to_owned()], name.clone());
                match cast {
                    Some(cast) => adapter::aliased(typed_cast(node, &cast), name),
                    None => node,
                }
            })
            .collect();
        let read = self.select_nodes(nodes);
        read.screen(false)?;
        read.compile()
    }

    /// The pipeline compiled as a reader of `columns` reads its rows: each
    /// column the name it is read under and the type it is read as where its
    /// reader needs a cast, as an entity model's enum column is read as its
    /// `text`. Where some column has a cast, a wildcard cannot deliver it,
    /// so [`into_sql`](Pipeline::into_sql) ends the pipeline in a projection
    /// of exactly these columns, each referred to through `this` and a cast
    /// column cast to its type under its own name; where none has, the
    /// pipeline is compiled as written. This is the projection
    /// [`into_model`](Pipeline::into_model) writes for a model's columns,
    /// for a reader no Rust type describes, as a language binding's declared
    /// model is. A pipeline embedded in another is read by that one, and
    /// this projection is not written for it.
    ///
    /// ```
    /// use pgorm::pgorm_query::TypeName;
    /// use pgorm::pipeline::{Pipeline, alias};
    ///
    /// let numeric = TypeName::new(alias("numeric"));
    /// let (sql, _) = Pipeline::from(alias("till"))
    ///     .read_columns([("id".to_owned(), None), ("takings".to_owned(), Some(numeric))])
    ///     .into_sql()?;
    /// assert_eq!(sql, "SELECT id, CAST(takings AS numeric) AS takings FROM till");
    /// # Ok::<_, pgorm::pipeline::PipelineError>(())
    /// ```
    // [spec:pgorm:sem:pipeline.described+1]
    pub fn read_columns(
        mut self,
        columns: impl IntoIterator<Item = (String, Option<pgorm_query::TypeName>)>,
    ) -> Self {
        let columns: Vec<_> = columns.into_iter().collect();
        self.read = columns
            .iter()
            .any(|(_, cast)| cast.is_some())
            .then_some(columns);
        self
    }

    /// The identifier screen, and with `aliases` the reserved-alias screen,
    /// over every stage and binding the pipeline carries.
    // [spec:pgorm:req:pipeline.errors+4]
    fn screen(&self, aliases: bool) -> Result<(), PipelineError> {
        let (mut identifiers, mut introduced) = (Vec::new(), Vec::new());
        for stage in self.bindings.iter().flatten().chain(&self.stages) {
            adapter::collect_identifiers(stage, &mut identifiers);
            if aliases {
                adapter::collect_aliases(stage, &mut introduced);
            }
        }
        if let Some(name) = identifiers.into_iter().find(|name| unquotable(name)) {
            return Err(PipelineError::UnquotableIdentifier(name));
        }
        if let Some(name) = introduced
            .into_iter()
            .find(|alias| RESERVED.contains(&alias.as_str()))
        {
            return Err(PipelineError::ReservedAlias(name));
        }
        Ok(())
    }

    /// prqlc's name resolution and lowering, then the placeholder census.
    // [spec:pgorm:req:pipeline.params+4]
    fn compile(self) -> Result<(String, Values), PipelineError> {
        let sql = adapter::compile(self.bindings, self.stages).map_err(PipelineError::Compile)?;
        Ok(census::prune(sql, self.values))
    }

    /// Compile the pipeline as `M` reads it: as a reader of the columns `M`
    /// reports ([`ExpectedColumn::read_cast`](crate::ExpectedColumn::read_cast))
    /// reads it, through [`read_columns`](Pipeline::read_columns), so a
    /// column `M` reads through a cast — an entity model's enum column, read
    /// as its `text`, or one its `select_as` overrides — is projected cast,
    /// which a wildcard cannot deliver: the server sends the column's own
    /// type, which the field does not decode. A type reporting no columns is
    /// compiled as written.
    ///
    /// The aliases the projection writes are `M`'s column names and are
    /// screened as identifiers only: they end the pipeline, so no later stage
    /// can resolve one to the PRQL name it shares, which is what the
    /// reserved-alias screen keeps an alias from doing.
    // [spec:pgorm:sem:pipeline.terminal+1]
    pub(super) fn into_sql_for<M: FromQueryResult>(
        self,
    ) -> Result<(String, Values), PipelineError> {
        let Some(columns) = M::expected_columns() else {
            return self.into_sql();
        };
        self.read_columns(
            columns
                .iter()
                .map(|column| (column.name().to_owned(), column.read_cast().cloned())),
        )
        .into_sql()
    }

    /// Compile and stage the pipeline for decoding into a
    /// [`FromQueryResult`] type.
    ///
    /// A type that reads a column through a cast, as an entity's model reads
    /// an enum column as its `text`, gets the column cast: the pipeline ends
    /// in a projection of the columns it reads, each cast column cast to the
    /// type it is read as, so `lunch_set::Model`, whose `tea` is an enum,
    /// reads `SELECT id, name, CAST(tea AS text) AS tea FROM lunch_set`
    /// where [`into_sql`](Pipeline::into_sql) gives `SELECT * FROM lunch_set`.
    /// That projection names only the columns the type reads, so it needs a
    /// relation in which each of those names means one column: after a join
    /// of two relations sharing a name it is refused as ambiguous, and
    /// [`select_sources`](Pipeline::select_sources) decodes each source's
    /// model instead.
    // [spec:pgorm:sem:pipeline.terminal+1]
    pub fn into_model<M>(self) -> Result<SelectorRaw<SelectModel<M>>, PipelineError>
    where
        M: FromQueryResult,
    {
        Ok(self.into_sql_for::<M>()?.into_model::<M>())
    }

    /// Compile and stage the pipeline for decoding into a tuple by column
    /// position.
    // [spec:pgorm:sem:pipeline.terminal+1]
    pub fn into_tuple<T>(self) -> Result<SelectorRaw<SelectGetableTuple<T>>, PipelineError>
    where
        T: TryGetableMany,
    {
        Ok(self.into_sql()?.into_tuple::<T>())
    }

    /// Compile, execute and decode every row into `M`, under the casts
    /// [`into_model`](Pipeline::into_model) writes for it.
    // [spec:pgorm:sem:pipeline.terminal+1]
    pub async fn all<M, C>(self, db: &C) -> Result<Vec<M>, Error>
    where
        M: FromQueryResult,
        C: ConnectionTrait,
    {
        self.into_model::<M>()?.all(db).await
    }

    /// Compile, execute and decode the first row into `M`, failing with
    /// [`Error::RecordNotFound`] when there is none.
    ///
    /// A `take 1` stage is appended, so at most one row leaves the server.
    // [spec:pgorm:sem:pipeline.terminal+1]
    pub async fn one<M, C>(self, db: &C) -> Result<M, Error>
    where
        M: FromQueryResult,
        C: ConnectionTrait,
    {
        self.take(1).into_model::<M>()?.one(db).await
    }

    /// Compile, execute and decode the first row into `M`, or `None` when
    /// there is none. Appends `take 1` like [`one`](Pipeline::one).
    // [spec:pgorm:sem:pipeline.terminal+1]
    pub async fn one_opt<M, C>(self, db: &C) -> Result<Option<M>, Error>
    where
        M: FromQueryResult,
        C: ConnectionTrait,
    {
        self.take(1).into_model::<M>()?.one_opt(db).await
    }
}
