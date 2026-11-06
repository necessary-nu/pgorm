//! The write terminals that read a written row's two versions: an update's
//! row before and after it, and an upsert's answer to whether it inserted
//! each row or updated the one that was there.
//!
//! PostgreSQL 18's `RETURNING` reads the row as it was before the statement
//! wrote it and as the statement left it (`sql.ast.returning`). Each terminal
//! here returns every column of both, the old under `o_` and the new under
//! `n_`, and decodes a model from each.

use crate::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, Insert,
    IntoActiveModel, Iterable, ManyRows, OneRow, QueryResult, UpdateMany, UpdateOne, error::*,
};
use pgorm_query::{Expr, IntoName, Name, Query, ReturningClause, StaticName, Values};
use tokio_postgres::types::ToSql;

use super::{ValueHolder, result_name::result_column_name, update::find_unchanged_model};

/// A row as it was before a statement wrote it, and as the statement left
/// it.
// [spec:pgorm:sem:exec.crud.versions]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change<M> {
    /// The row before the write.
    pub old: M,
    /// The row as the write left it.
    pub new: M,
}

/// What an upsert did with a row it wrote.
// [spec:pgorm:sem:exec.crud.versions]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Upserted<M> {
    /// The row was not there, and the insert wrote it.
    Inserted(M),
    /// A row was there, and `ON CONFLICT DO UPDATE` updated it.
    Updated(Change<M>),
}

impl<M> Upserted<M> {
    /// The row as the statement left it, whichever it did.
    pub fn into_model(self) -> M {
        match self {
            Self::Inserted(model) => model,
            Self::Updated(change) => change.new,
        }
    }
}

/// The names the list reads the two versions by. They are always renamed:
/// `old` and `new` are scoped like any relation name, so a relation of the
/// statement called either — the entity's own table, an `UpdateMany::from`
/// item — would take the keyword, and the server would answer `old."col"`
/// from it without complaint. A relation called one of these instead is
/// refused (`42712`), which says so rather than answering wrong.
const OLD: &str = "pgorm_old";
const NEW: &str = "pgorm_new";

const OLD_PREFIX: &str = "o_";
const NEW_PREFIX: &str = "n_";

/// `RETURNING WITH (OLD AS .., NEW AS ..)` every column of `E` from both
/// versions, each read through the column's own `select_as` and named under
/// its version's prefix by the bounded composition every prefixed decode
/// reads (`query.graph.writer`).
// [spec:pgorm:sem:exec.crud.versions]
fn both_versions<E: EntityTrait>() -> ReturningClause {
    let items = [(OLD, OLD_PREFIX), (NEW, NEW_PREFIX)]
        .into_iter()
        .flat_map(|(row, prefix)| {
            E::Column::iter().map(move |column| {
                let read = column.select_as(Expr::col((Name::runtime(row), column.into_name())));
                let alias = result_column_name(prefix, column.as_str());
                (read, Name::runtime(alias))
            })
        });
    Query::returning()
        .exprs_as(items)
        .old_as(Name::runtime(OLD))
        .new_as(Name::runtime(NEW))
}

/// A returned row's two versions: the old one absent where it reads `NULL` in
/// every column, as it does for a row the statement inserted
/// (`exec.decode.absent`).
// [spec:pgorm:sem:exec.crud.versions]
fn versions_of<M: FromQueryResult>(row: QueryResult) -> Result<(Option<M>, M), Error> {
    Ok((
        M::from_query_result_optional(&row, OLD_PREFIX)?,
        M::from_query_result(&row, NEW_PREFIX)?,
    ))
}

/// An updated row's versions, the old one present, as an update's always is.
fn change_of<M: FromQueryResult>(row: QueryResult) -> Result<Change<M>, Error> {
    match versions_of(row)? {
        (Some(old), new) => Ok(Change { old, new }),
        (None, _) => Err(Error::Query(RuntimeError::Internal(
            "an updated row returned no old row".to_owned(),
        ))),
    }
}

/// Run `sql` and return every row.
async fn rows_of<C: ConnectionTrait>(
    sql: &str,
    values: Values,
    db: &C,
) -> Result<Vec<QueryResult>, Error> {
    let values = values.into_iter().map(ValueHolder).collect::<Vec<_>>();
    let values = values
        .iter()
        .map(|x| x as _)
        .collect::<Vec<&(dyn ToSql + Sync)>>();
    Ok(db
        .query_all(sql, &values)
        .await?
        .into_iter()
        .map(|row| QueryResult { row })
        .collect())
}

impl<A> UpdateOne<A>
where
    A: ActiveModelTrait,
{
    /// Execute the update and return the row before and after it.
    ///
    /// With nothing to set no statement is sent, as
    /// [`exec_returning_model`](Self::exec_returning_model) holds, and the
    /// row read under the update's `WHERE` is both versions, nothing having
    /// changed it. An update matching no row is [`Error::RecordNotFound`].
    // [spec:pgorm:sem:exec.crud.versions]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    pub async fn exec_returning_change<C>(
        mut self,
        db: &C,
    ) -> Result<Change<<A::Entity as EntityTrait>::Model>, Error>
    where
        <A::Entity as EntityTrait>::Model: IntoActiveModel<A>,
        C: ConnectionTrait,
    {
        if self.query.get_values().is_empty() {
            let model = find_unchanged_model::<A, C>(self.entity, &self.query, db).await?;
            return Ok(Change {
                old: model.clone(),
                new: model,
            });
        }
        self.query.returning(both_versions::<A::Entity>());
        let (sql, values) = self.query.build();
        match rows_of(&sql, values, db).await?.pop() {
            Some(row) => change_of(row),
            None => Err(Error::RecordNotFound),
        }
    }
}

impl<E> UpdateMany<E>
where
    E: EntityTrait,
{
    /// Execute the update and return every row it changed, before and after,
    /// in the order the server returned them.
    ///
    /// An update naming no column to set is [`Error::NothingToSet`], as
    /// [`exec`](Self::exec) and
    /// [`exec_returning_models`](Self::exec_returning_models) hold.
    // [spec:pgorm:sem:exec.crud.versions]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    pub async fn exec_returning_changes<C>(mut self, db: &C) -> Result<Vec<Change<E::Model>>, Error>
    where
        C: ConnectionTrait,
    {
        if self.query.get_values().is_empty() {
            return Err(Error::NothingToSet);
        }
        self.query.returning(both_versions::<E>());
        let (sql, values) = self.query.build();
        rows_of(&sql, values, db)
            .await?
            .into_iter()
            .map(change_of)
            .collect()
    }
}

/// Every row an upsert wrote: inserted where its old row is absent,
/// updated where it is there.
// [spec:pgorm:sem:exec.crud.versions]
async fn exec_upserts<A, C>(
    mut statement: pgorm_query::InsertStatement,
    db: &C,
) -> Result<Vec<Upserted<<A::Entity as EntityTrait>::Model>>, Error>
where
    A: ActiveModelTrait,
    C: ConnectionTrait,
{
    statement.returning(both_versions::<A::Entity>());
    let (sql, values) = statement.build();
    rows_of(&sql, values, db)
        .await?
        .into_iter()
        .map(|row| {
            Ok(match versions_of(row)? {
                (None, new) => Upserted::Inserted(new),
                (Some(old), new) => Upserted::Updated(Change { old, new }),
            })
        })
        .collect()
}

impl<A> Insert<A, OneRow>
where
    A: ActiveModelTrait,
{
    /// Execute the insert and say what it did with the row: inserted it, or,
    /// under `ON CONFLICT DO UPDATE`, updated the row it conflicted with,
    /// which comes back before and after.
    ///
    /// `None` is a row the statement did not write: `ON CONFLICT DO NOTHING`
    /// skipped it, or the `DO UPDATE`'s `WHERE` left the row it conflicted
    /// with as it was.
    // [spec:pgorm:sem:exec.crud.versions]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    pub async fn exec_returning_upsert<C>(
        self,
        db: &C,
    ) -> Result<Option<Upserted<<A::Entity as EntityTrait>::Model>>, Error>
    where
        C: ConnectionTrait,
    {
        Ok(exec_upserts::<A, C>(self.query, db).await?.pop())
    }
}

impl<A> Insert<A, ManyRows>
where
    A: ActiveModelTrait,
{
    /// Execute the insert and say, for every row it wrote, whether it inserted
    /// it or updated the row it conflicted with, in the order the database
    /// wrote them. A row the conflict clause skipped is not among them.
    ///
    /// An insert to which no model was added writes nothing and answers
    /// nothing; a batch whose models do not all set the same columns
    /// ([`Insert::add`]) fails before anything is written.
    // [spec:pgorm:sem:exec.crud.versions]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    // [spec:pgorm:req:query.build.insert.uniform-columns+4]
    pub async fn exec_returning_upserts<C>(
        self,
        db: &C,
    ) -> Result<Vec<Upserted<<A::Entity as EntityTrait>::Model>>, Error>
    where
        C: ConnectionTrait,
    {
        self.ensure_uniform_columns()?;
        if self.has_no_models() {
            return Ok(Vec::new());
        }
        exec_upserts::<A, C>(self.query, db).await
    }
}
