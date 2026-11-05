use crate::entity::entity_table;
use crate::{
    ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, Error, IntoActiveModel, Iterable,
    QueryTrait, RuntimeError, StaticName,
};
use core::marker::PhantomData;
use pgorm_query::{Expr, InsertStatement, OnConflict, SimpleExpr};

/// The column shape an [`Insert`] has recorded from the models added to it.
///
/// Emptiness is a variant rather than a predicate over the bitmap: `Present` is
/// only reachable through [`Insert::add`] for a model that set at least one
/// column, so an all-`NotSet` model cannot present itself as a row of values.
/// "No model at all" is a variant of its own for the same reason: a batch that
/// asks for nothing and a model that asks for a row of database defaults are
/// different requests, and both leave the statement's value list empty, so only
/// the builder can tell them apart.
/// A model that disagrees with the shape already recorded is a variant too:
/// [`Insert::add`] returns `Self` so calls chain and has nowhere to report the
/// disagreement, so it holds it until an execution path can fail with it.
#[derive(Debug)]
enum InsertColumns {
    /// No model has been added. The initial state.
    Unset,
    /// No column is set. `rows` counts the models added so far, at least one,
    /// every one of which left every column `NotSet`.
    Blank { rows: u32 },
    /// Per-column presence recorded from the first model added, which set at
    /// least one column.
    Present(Vec<bool>),
    /// A model's present columns differed from what was already recorded.
    /// Terminal: the statement took neither that model's columns nor its
    /// values, and models added afterwards are no longer compared.
    Mismatch {
        /// Columns the models before it set and it did not.
        first_only: Vec<String>,
        /// Columns it set and the models before it did not.
        later_only: Vec<String>,
    },
}

impl InsertColumns {
    /// How many all-`NotSet` models have been added: zero outside `Blank`.
    fn blank_rows(&self) -> u32 {
        match self {
            Self::Blank { rows } => *rows,
            Self::Unset | Self::Present(_) | Self::Mismatch { .. } => 0,
        }
    }

    /// Records the disagreement between the presence bitmap already held and
    /// that of the model being added, naming the columns by which they differ.
    /// A `recorded` shorter than `present` reads as "set nothing", which is the
    /// blank state's shape.
    fn mismatch<A>(recorded: &[bool], present: &[bool]) -> Self
    where
        A: ActiveModelTrait,
    {
        let set = |bitmap: &[bool], index: usize| bitmap.get(index).copied().unwrap_or(false);
        let mut first_only = Vec::new();
        let mut later_only = Vec::new();
        for (index, col) in <A::Entity as EntityTrait>::Column::iter().enumerate() {
            match (set(recorded, index), set(present, index)) {
                (true, false) => first_only.push(col.as_str().to_owned()),
                (false, true) => later_only.push(col.as_str().to_owned()),
                (true, true) | (false, false) => {}
            }
        }
        Self::Mismatch {
            first_only,
            later_only,
        }
    }
}

/// Every column of `A`'s entity, in iteration order, with the value an insert
/// of `am` writes into it: a `Set` or `Unchanged` value as its column's
/// `save_as` writes it, so a cast, enum or money column is written as an
/// ordinary insert writes it, and `None` for a column left `NotSet`.
///
/// A column the model does not carry contributes no value, exactly as a
/// `NotSet` one does; where the other models of a batch do carry it, that is
/// the disagreement [`Insert::ensure_uniform_columns`] reports.
// [spec:pgorm:sem:query.build.insert+6]
// [spec:pgorm:sem:disjoint.write+2]
pub(crate) fn saved_values<A>(
    mut am: A,
) -> Vec<(<A::Entity as EntityTrait>::Column, Option<SimpleExpr>)>
where
    A: ActiveModelTrait,
{
    <A::Entity as EntityTrait>::Column::iter()
        .map(|col| {
            let value = match am.take(col).unwrap_or_else(|_| ActiveValue::not_set()) {
                ActiveValue::Set(value) | ActiveValue::Unchanged(value) => {
                    Some(col.save_as(Expr::val(value)))
                }
                ActiveValue::NotSet => None,
            };
            (col, value)
        })
        .collect()
}

/// Names the columns each side of a mismatch sets and the other does not,
/// dropping the side that has none.
fn columns_mismatch_err(first_only: &[String], later_only: &[String]) -> Error {
    let sides = [
        (first_only, "set in the first model but not in a later one"),
        (later_only, "set in a later model but not in the first"),
    ]
    .into_iter()
    .filter(|(names, _)| !names.is_empty())
    .map(|(names, side)| format!("{} {side}", quoted_columns(names)))
    .collect::<Vec<_>>();

    Error::Query(RuntimeError::Internal(format!(
        "models added to one insert do not share a column set: {}",
        sides.join("; ")
    )))
}

/// Renders `` `id` is `` for one column and `` `id`, `name` are `` for several.
fn quoted_columns(names: &[String]) -> String {
    let list = names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let verb = if names.len() == 1 { "is" } else { "are" };
    format!("{list} {verb}")
}

/// The row count an [`Insert`] holding exactly one model is built for: it comes
/// from [`Insert::one`], and its returning terminals answer with one key and
/// one model.
// [spec:pgorm:sem:query.build.insert+6]
#[derive(Debug, Clone, Copy)]
pub struct OneRow;

/// The row count an [`Insert`] of a batch is built for — any number of models,
/// none included: it comes from [`Insert::many`], and from adding a model to
/// any insert, and its returning terminals answer with a key and a model for
/// every row written.
// [spec:pgorm:sem:query.build.insert+6]
#[derive(Debug, Clone, Copy)]
pub struct ManyRows;

/// Performs INSERT operations on a ActiveModel
///
/// `R` is the row count the builder holds, [`OneRow`] or [`ManyRows`], and it
/// decides which returning terminals exist: `exec_returning_pk` and
/// `exec_returning_model` answer for one row, `exec_returning_pks` and
/// `exec_returning_models` for a batch. A batch cannot ask for one row's
/// answer, so it cannot be handed whichever row came back last.
// [spec:pgorm:sem:query.build.insert+6]
#[derive(Debug)]
pub struct Insert<A, R>
where
    A: ActiveModelTrait,
{
    pub(crate) query: InsertStatement,
    columns: InsertColumns,
    pub(crate) model: PhantomData<A>,
    rows: PhantomData<R>,
}

impl<A> Default for Insert<A, ManyRows>
where
    A: ActiveModelTrait,
{
    fn default() -> Self {
        Self::new()
    }
}

// [spec:pgorm:sem:query.build.insert+6]
impl<A> Insert<A, OneRow>
where
    A: ActiveModelTrait,
{
    /// Insert one Model or ActiveModel
    ///
    /// Model
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     Insert::one(cake::Model {
    ///         id: 1,
    ///         name: "Apple Pie".to_owned(),
    ///     })
    ///     .as_query()
    ///     .to_string(),
    ///     r#"INSERT INTO "cake" ("id", "name") VALUES (1, 'Apple Pie')"#,
    /// );
    /// ```
    /// ActiveModel
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     Insert::one(cake::ActiveModel {
    ///         id: ActiveValue::NotSet,
    ///         name: set("Apple Pie"),
    ///     })
    ///     .as_query()
    ///     .to_string(),
    ///     r#"INSERT INTO "cake" ("name") VALUES ('Apple Pie')"#,
    /// );
    /// ```
    pub fn one<M>(m: M) -> Self
    where
        M: IntoActiveModel<A>,
    {
        Self::new().push(m)
    }

    /// [`one`](Insert::one) into the table `entity` names: the instance's
    /// schema and table rather than [`Default`]'s, for an entity whose names
    /// are computed at run time.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::tenant_cake};
    ///
    /// let acme = tenant_cake::Entity { tenant: "acme" };
    /// assert_eq!(
    ///     Insert::one_in(
    ///         acme,
    ///         tenant_cake::ActiveModel {
    ///             name: set("Apple Pie"),
    ///             ..Default::default()
    ///         },
    ///     )
    ///     .as_query()
    ///     .to_string(),
    ///     r#"INSERT INTO "acme"."cake" ("name") VALUES ('Apple Pie')"#,
    /// );
    /// ```
    // [spec:pgorm:sem:query.build.insert+6]
    pub fn one_in<M>(entity: A::Entity, m: M) -> Self
    where
        M: IntoActiveModel<A>,
    {
        Self::new_in(entity).push(m)
    }
}

// [spec:pgorm:sem:query.build.insert+6]
impl<A> Insert<A, ManyRows>
where
    A: ActiveModelTrait,
{
    /// Insert many Model or ActiveModel
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, tests_cfg::cake};
    ///
    /// assert_eq!(
    ///     Insert::many([
    ///         cake::Model {
    ///             id: 1,
    ///             name: "Apple Pie".to_owned(),
    ///         },
    ///         cake::Model {
    ///             id: 2,
    ///             name: "Orange Scone".to_owned(),
    ///         }
    ///     ])
    ///     .as_query()
    ///     .to_string(),
    ///     r#"INSERT INTO "cake" ("id", "name") VALUES (1, 'Apple Pie'), (2, 'Orange Scone')"#,
    /// );
    /// ```
    #[doc(alias = "insert_many")]
    pub fn many<M, I>(models: I) -> Self
    where
        M: IntoActiveModel<A>,
        I: IntoIterator<Item = M>,
    {
        Self::new().add_many(models)
    }

    /// [`many`](Insert::many) into the table `entity` names, as
    /// [`one_in`](Insert::one_in) writes it.
    // [spec:pgorm:sem:query.build.insert+6]
    pub fn many_in<M, I>(entity: A::Entity, models: I) -> Self
    where
        M: IntoActiveModel<A>,
        I: IntoIterator<Item = M>,
    {
        Self::new_in(entity).add_many(models)
    }
}

// [spec:pgorm:sem:query.build.insert+6]
impl<A, R> Insert<A, R>
where
    A: ActiveModelTrait,
{
    pub(crate) fn new() -> Self {
        Self::new_in(A::Entity::default())
    }

    /// An insert into the table `entity` names, holding no model yet.
    // [spec:pgorm:sem:query.build.insert+6]
    pub(crate) fn new_in(entity: A::Entity) -> Self {
        Self {
            query: InsertStatement::new()
                .into_table(entity_table(&entity))
                .or_default_values()
                .to_owned(),
            columns: InsertColumns::Unset,
            model: PhantomData,
            rows: PhantomData,
        }
    }

    /// The same statement and recorded columns, built for another row count.
    fn into_rows<S>(self) -> Insert<A, S> {
        Insert {
            query: self.query,
            columns: self.columns,
            model: PhantomData,
            rows: PhantomData,
        }
    }

    /// Whether the statement carries no column at all: either no model was
    /// added, or every model added left every column `NotSet`.
    // [spec:pgorm:sem:query.build.insert.empty-failsafe+5]
    pub(crate) fn is_empty(&self) -> bool {
        matches!(
            self.columns,
            InsertColumns::Unset | InsertColumns::Blank { .. }
        )
    }

    /// Whether no model was ever added, as distinct from models that left every
    /// column `NotSet`: nothing was asked for, so there is no row to write.
    // [spec:pgorm:sem:query.build.insert+6]
    pub(crate) fn has_no_models(&self) -> bool {
        matches!(self.columns, InsertColumns::Unset)
    }

    /// The columns mismatch recorded while models were added, if any.
    ///
    /// [`add`](Self::add) returns the builder so that calls chain, so a model
    /// whose present columns disagree with the first model's is recorded rather
    /// than reported there. Every execution path of a batch asks here and
    /// fails with the resulting [`Error::Query`] before sending any SQL;
    /// callers that want the error sooner can ask directly.
    // [spec:pgorm:req:query.build.insert.uniform-columns+4]
    pub fn ensure_uniform_columns(&self) -> Result<(), Error> {
        match &self.columns {
            InsertColumns::Mismatch {
                first_only,
                later_only,
            } => Err(columns_mismatch_err(first_only, later_only)),
            InsertColumns::Unset | InsertColumns::Blank { .. } | InsertColumns::Present(_) => {
                Ok(())
            }
        }
    }

    /// Add a Model to Self, which makes it a batch: an insert holding a
    /// second model answers for every row it writes.
    ///
    /// A model whose present columns differ from those of the models already
    /// added contributes neither columns nor values; the disagreement is
    /// recorded and reported by [`ensure_uniform_columns`](Self::ensure_uniform_columns)
    /// and by every execution path.
    // [spec:pgorm:req:query.build.insert.uniform-columns+4]
    // [spec:pgorm:sem:query.build.insert+6]
    #[allow(clippy::should_implement_trait)]
    pub fn add<M>(self, m: M) -> Insert<A, ManyRows>
    where
        M: IntoActiveModel<A>,
    {
        self.push(m).into_rows()
    }

    /// Record one model's columns and values, keeping the row count the
    /// builder was made for.
    // [spec:pgorm:req:query.build.insert.uniform-columns+4]
    // [spec:pgorm:sem:query.build.insert+6]
    fn push<M>(mut self, m: M) -> Self
    where
        M: IntoActiveModel<A>,
    {
        let saved = saved_values::<A>(m.into_active_model());
        let present: Vec<bool> = saved.iter().map(|(_, value)| value.is_some()).collect();
        let (columns, values): (Vec<_>, Vec<_>) = saved
            .into_iter()
            .filter_map(|(col, value)| value.map(|value| (col, value)))
            .unzip();

        match &self.columns {
            InsertColumns::Mismatch { .. } => return self,
            InsertColumns::Unset | InsertColumns::Blank { .. } if columns.is_empty() => {
                let rows = self.columns.blank_rows().saturating_add(1);
                self.columns = InsertColumns::Blank { rows };
                self.query.or_default_values_many(rows);
                return self;
            }
            InsertColumns::Unset => self.columns = InsertColumns::Present(present),
            InsertColumns::Blank { .. } => {
                self.columns = InsertColumns::mismatch::<A>(&[], &present);
                return self;
            }
            InsertColumns::Present(recorded) if *recorded != present => {
                self.columns = InsertColumns::mismatch::<A>(recorded, &present);
                return self;
            }
            InsertColumns::Present(_) => {}
        }

        self.query.columns(columns);
        self.query.values_panic(values);
        self
    }

    /// Add many Models to Self, which makes it a batch
    pub fn add_many<M, I>(self, models: I) -> Insert<A, ManyRows>
    where
        M: IntoActiveModel<A>,
        I: IntoIterator<Item = M>,
    {
        let mut batch = self.into_rows::<ManyRows>();
        for model in models.into_iter() {
            batch = batch.push(model);
        }
        batch
    }

    /// On conflict
    ///
    /// on conflict do nothing
    /// ```
    /// use pgorm::{entity::*, query::*, pgorm_query::OnConflict, tests_cfg::cake};
    ///
    /// let orange = cake::ActiveModel {
    ///     id: set(2),
    ///     name: set("Orange"),
    /// };
    /// assert_eq!(
    ///     Insert::one(orange)
    ///         .on_conflict(OnConflict::column(cake::Column::Name).do_nothing())
    ///         .as_query()
    ///         .to_string(),
    ///     r#"INSERT INTO "cake" ("id", "name") VALUES (2, 'Orange') ON CONFLICT ("name") DO NOTHING"#,
    /// );
    /// ```
    ///
    /// on conflict do update
    /// ```
    /// use pgorm::{entity::*, query::*, pgorm_query::OnConflict, tests_cfg::cake};
    ///
    /// let orange = cake::ActiveModel {
    ///     id: set(2),
    ///     name: set("Orange"),
    /// };
    /// assert_eq!(
    ///     Insert::one(orange)
    ///         .on_conflict(
    ///             OnConflict::column(cake::Column::Name).update_column(cake::Column::Name)
    ///         )
    ///         .as_query()
    ///         .to_string(),
    ///     r#"INSERT INTO "cake" ("id", "name") VALUES (2, 'Orange') ON CONFLICT ("name") DO UPDATE SET "name" = "excluded"."name""#,
    /// );
    /// ```
    pub fn on_conflict<T>(mut self, on_conflict: T) -> Self
    where
        T: Into<OnConflict>,
    {
        self.query.on_conflict(on_conflict);
        self
    }

    /// Make an empty insert a no-op instead of an error.
    ///
    /// Converts to [`TryInsert`] without touching the statement, so an insert
    /// with nothing to write reports `TryInsertResult::Empty` rather than
    /// sending SQL. Distinct from
    /// [`OnConflict::do_nothing`](pgorm_query::OnConflict::do_nothing), which
    /// attaches an `ON CONFLICT` clause to a statement that does run.
    // [spec:pgorm:sem:query.build.insert.empty-failsafe+5]
    #[doc(alias = "do_nothing")]
    pub fn on_empty_do_nothing(self) -> TryInsert<A, R>
    where
        A: ActiveModelTrait,
    {
        TryInsert::from_insert(self)
    }

    /// Set ON CONFLICT on the primary key columns to do nothing.
    ///
    /// An entity with no primary key column gets the arbiter-less
    /// `ON CONFLICT DO NOTHING`, which answers for every constraint — the
    /// widest reading of "the primary key conflicted" when there is no primary
    /// key to name.
    ///
    /// ```
    /// use pgorm::{entity::*, query::*, pgorm_query::OnConflict, tests_cfg::cake};
    ///
    /// let orange = cake::ActiveModel {
    ///     id: set(2),
    ///     name: set("Orange"),
    /// };
    ///
    /// assert_eq!(
    ///     Insert::one(orange)
    ///         .on_conflict_do_nothing()
    ///         .as_query()
    ///         .to_string(),
    ///     r#"INSERT INTO "cake" ("id", "name") VALUES (2, 'Orange') ON CONFLICT ("id") DO NOTHING"#,
    /// );
    /// ```
    // [spec:pgorm:sem:query.build.insert.empty-failsafe+5]
    pub fn on_conflict_do_nothing(mut self) -> TryInsert<A, R>
    where
        A: ActiveModelTrait,
    {
        let mut primary_keys = <A::Entity as EntityTrait>::PrimaryKey::iter();
        let on_conflict = match primary_keys.next() {
            Some(first) => OnConflict::column(first)
                .and_columns(primary_keys)
                .do_nothing(),
            None => OnConflict::do_nothing(),
        };
        self.query.on_conflict(on_conflict);

        TryInsert::from_insert(self)
    }
}

impl<A, R> QueryTrait for Insert<A, R>
where
    A: ActiveModelTrait,
{
    type QueryStatement = InsertStatement;

    fn query(&mut self) -> &mut InsertStatement {
        &mut self.query
    }

    fn as_query(&self) -> &InsertStatement {
        &self.query
    }

    fn into_query(self) -> InsertStatement {
        self.query
    }
}

/// Performs INSERT operations on a ActiveModel, will do nothing if input is empty.
///
/// All functions works the same as if it is `Insert<A, R>`, row count
/// included. Please refer to the `Insert` page for more information
// [spec:pgorm:sem:query.build.insert.empty-failsafe+5]
#[derive(Debug)]
pub struct TryInsert<A, R>
where
    A: ActiveModelTrait,
{
    pub(crate) insert_struct: Insert<A, R>,
}

impl<A> Default for TryInsert<A, ManyRows>
where
    A: ActiveModelTrait,
{
    fn default() -> Self {
        Self::from_insert(Insert::new())
    }
}

#[allow(missing_docs)]
impl<A> TryInsert<A, OneRow>
where
    A: ActiveModelTrait,
{
    pub fn one<M>(m: M) -> Self
    where
        M: IntoActiveModel<A>,
    {
        Self::from_insert(Insert::one(m))
    }

    // [spec:pgorm:sem:query.build.insert+6]
    pub fn one_in<M>(entity: A::Entity, m: M) -> Self
    where
        M: IntoActiveModel<A>,
    {
        Self::from_insert(Insert::one_in(entity, m))
    }
}

#[allow(missing_docs)]
impl<A> TryInsert<A, ManyRows>
where
    A: ActiveModelTrait,
{
    pub fn many<M, I>(models: I) -> Self
    where
        M: IntoActiveModel<A>,
        I: IntoIterator<Item = M>,
    {
        Self::from_insert(Insert::many(models))
    }

    // [spec:pgorm:sem:query.build.insert+6]
    pub fn many_in<M, I>(entity: A::Entity, models: I) -> Self
    where
        M: IntoActiveModel<A>,
        I: IntoIterator<Item = M>,
    {
        Self::from_insert(Insert::many_in(entity, models))
    }
}

#[allow(missing_docs)]
impl<A, R> TryInsert<A, R>
where
    A: ActiveModelTrait,
{
    #[allow(clippy::should_implement_trait)]
    pub fn add<M>(self, m: M) -> TryInsert<A, ManyRows>
    where
        M: IntoActiveModel<A>,
    {
        TryInsert::from_insert(self.insert_struct.add(m))
    }

    pub fn add_many<M, I>(self, models: I) -> TryInsert<A, ManyRows>
    where
        M: IntoActiveModel<A>,
        I: IntoIterator<Item = M>,
    {
        TryInsert::from_insert(self.insert_struct.add_many(models))
    }

    pub fn on_conflict<T>(mut self, on_conflict: T) -> Self
    where
        T: Into<OnConflict>,
    {
        self.insert_struct.query.on_conflict(on_conflict);
        self
    }

    // helper function for do_nothing in Insert<A, R>
    pub fn from_insert(insert: Insert<A, R>) -> Self {
        Self {
            insert_struct: insert,
        }
    }

    /// The columns mismatch recorded while models were added, if any; see
    /// [`Insert::ensure_uniform_columns`].
    // [spec:pgorm:req:query.build.insert.uniform-columns+4]
    pub fn ensure_uniform_columns(&self) -> Result<(), Error> {
        self.insert_struct.ensure_uniform_columns()
    }
}

impl<A, R> QueryTrait for TryInsert<A, R>
where
    A: ActiveModelTrait,
{
    type QueryStatement = InsertStatement;

    fn query(&mut self) -> &mut InsertStatement {
        &mut self.insert_struct.query
    }

    fn as_query(&self) -> &InsertStatement {
        &self.insert_struct.query
    }

    fn into_query(self) -> InsertStatement {
        self.insert_struct.query
    }
}
#[cfg(test)]
mod tests {
    use pgorm_query::OnConflict;

    use crate::tests_cfg::cake::{self};
    use crate::{ActiveValue, Insert, IntoActiveModel, ManyRows, QueryTrait, set};

    #[test]
    fn insert_1() {
        assert_eq!(
            Insert::<cake::ActiveModel, ManyRows>::new()
                .add(cake::ActiveModel {
                    id: ActiveValue::not_set(),
                    name: set("Apple Pie"),
                })
                .as_query()
                .to_string(),
            r#"INSERT INTO "cake" ("name") VALUES ('Apple Pie')"#,
        );
    }

    #[test]
    fn insert_2() {
        assert_eq!(
            Insert::<cake::ActiveModel, ManyRows>::new()
                .add(cake::ActiveModel {
                    id: set(1),
                    name: set("Apple Pie"),
                })
                .as_query()
                .to_string(),
            r#"INSERT INTO "cake" ("id", "name") VALUES (1, 'Apple Pie')"#,
        );
    }

    #[test]
    fn insert_3() {
        assert_eq!(
            Insert::<cake::ActiveModel, ManyRows>::new()
                .add(cake::Model {
                    id: 1,
                    name: "Apple Pie".to_owned(),
                })
                .as_query()
                .to_string(),
            r#"INSERT INTO "cake" ("id", "name") VALUES (1, 'Apple Pie')"#,
        );
    }

    #[test]
    fn insert_4() {
        assert_eq!(
            Insert::<cake::ActiveModel, ManyRows>::new()
                .add_many([
                    cake::Model {
                        id: 1,
                        name: "Apple Pie".to_owned(),
                    },
                    cake::Model {
                        id: 2,
                        name: "Orange Scone".to_owned(),
                    }
                ])
                .as_query()
                .to_string(),
            r#"INSERT INTO "cake" ("id", "name") VALUES (1, 'Apple Pie'), (2, 'Orange Scone')"#,
        );
    }

    #[test]
    fn insert_5() {
        let apple = cake::ActiveModel {
            name: set("Apple"),
            ..Default::default()
        };
        let orange = cake::ActiveModel {
            id: set(2),
            name: set("Orange"),
        };
        let insert = Insert::<cake::ActiveModel, ManyRows>::new().add_many([apple, orange]);

        assert!(insert.ensure_uniform_columns().is_err());
        assert_eq!(
            insert.as_query().to_string(),
            r#"INSERT INTO "cake" ("name") VALUES ('Apple')"#,
        );
    }

    #[test]
    fn insert_6() {
        let orange = cake::ActiveModel {
            id: set(2),
            name: set("Orange"),
        };

        assert_eq!(
            Insert::one(orange)
                .on_conflict(OnConflict::column(cake::Column::Name).do_nothing())
                .as_query()
                .to_string(),
            r#"INSERT INTO "cake" ("id", "name") VALUES (2, 'Orange') ON CONFLICT ("name") DO NOTHING"#,
        );
    }

    #[test]
    fn insert_7() {
        let orange = cake::ActiveModel {
            id: set(2),
            name: set("Orange"),
        };

        assert_eq!(
            Insert::one(orange)
                .on_conflict(
                    OnConflict::column(cake::Column::Name).update_column(cake::Column::Name)
                )
                .as_query()
                .to_string(),
            r#"INSERT INTO "cake" ("id", "name") VALUES (2, 'Orange') ON CONFLICT ("name") DO UPDATE SET "name" = "excluded"."name""#,
        );
    }

    #[test]
    fn insert_8() {
        mod post {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "posts")]
            pub struct Model {
                #[pgorm(primary_key, select_as = "INTEGER", save_as = "TEXT")]
                pub id: i32,
                pub title: String,
                pub text: String,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        let model = post::Model {
            id: 1,
            title: "News wrap up 2022".into(),
            text: "brbrbrrrbrbrbrr...".into(),
        };

        assert_eq!(
            Insert::one(model.into_active_model())
                .as_query()
                .to_string(),
            r#"INSERT INTO "posts" ("id", "title", "text") VALUES (CAST(1 AS TEXT), 'News wrap up 2022', 'brbrbrrrbrbrbrr...')"#,
        );
    }

    #[test]
    fn insert_9() {
        mod post {
            use crate as pgorm;
            use crate::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, Eq, Entity)]
            #[pgorm(table_name = "posts")]
            pub struct Model {
                #[pgorm(
                    primary_key,
                    auto_increment = false,
                    select_as = "INTEGER",
                    save_as = "TEXT"
                )]
                pub id_primary: i32,
                #[pgorm(
                    primary_key,
                    auto_increment = false,
                    select_as = "INTEGER",
                    save_as = "TEXT"
                )]
                pub id_secondary: i32,
                pub title: String,
                pub text: String,
            }

            #[derive(Copy, Clone, Debug, Relation)]
            pub enum Relation {}
        }

        let model = post::Model {
            id_primary: 1,
            id_secondary: 1001,
            title: "News wrap up 2022".into(),
            text: "brbrbrrrbrbrbrr...".into(),
        };

        assert_eq!(
            Insert::one(model.into_active_model())
                .as_query()
                .to_string(),
            [
                r#"INSERT INTO "posts" ("id_primary", "id_secondary", "title", "text")"#,
                r#"VALUES (CAST(1 AS TEXT), CAST(1001 AS TEXT), 'News wrap up 2022', 'brbrbrrrbrbrbrr...')"#,
            ]
            .join(" "),
        );
    }
}
