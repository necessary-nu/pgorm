use crate::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, Insert, IntoActiveModel, Iterable,
    ManyRows, OneRow, PrimaryKeyToColumn, PrimaryKeyTrait, QueryResult, SelectModel, SelectorRaw,
    TryInsert, error::*,
};
use pgorm_query::{InsertStatement, Query, SqlName};
use tokio_postgres::types::ToSql;

use super::ValueHolder;

/// The primary key an insert reports back, typed as the entity's declared
/// `PrimaryKey::ValueType`.
pub type InsertedPrimaryKey<A> =
    <<<A as ActiveModelTrait>::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType;

/// The model an insert reports back.
type InsertedModel<A> = <<A as ActiveModelTrait>::Entity as EntityTrait>::Model;

/// The types of results for an INSERT operation
// [spec:pgorm:sem:exec.crud.try-insert+4]
#[derive(Debug)]
pub enum TryInsertResult<T> {
    /// The INSERT statement did not have any value to insert
    Empty,
    /// The INSERT operation did not insert any valid value
    Conflicted,
    /// Successfully inserted
    Inserted(T),
}

// [spec:pgorm:sem:exec.crud.try-insert+4]
impl<A, R> TryInsert<A, R>
where
    A: ActiveModelTrait,
{
    /// Whether the statement carries an `ON CONFLICT` clause, and so could have
    /// skipped the insert rather than failed it.
    fn has_conflict_clause(&self) -> bool {
        self.insert_struct.query.get_on_conflict().is_some()
    }

    /// Execute the insert and report how many rows it wrote.
    ///
    /// No `RETURNING` clause is emitted. See `exec_returning_pk` /
    /// `exec_returning_pks` for the inserted primary keys and
    /// `exec_returning_model` / `exec_returning_models` for the rows.
    // [spec:pgorm:req:query.build.insert.uniform-columns+4]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    pub async fn exec<C>(self, db: &C) -> Result<TryInsertResult<u64>, Error>
    where
        C: ConnectionTrait,
    {
        self.ensure_uniform_columns()?;
        if self.insert_struct.is_empty() {
            return Ok(TryInsertResult::Empty);
        }
        let conflict_clause = self.has_conflict_clause();
        let res = self.insert_struct.exec(db).await;
        match res {
            Ok(0) if conflict_clause => Ok(TryInsertResult::Conflicted),
            Ok(res) => Ok(TryInsertResult::Inserted(res)),
            Err(Error::RecordNotInserted) => Ok(TryInsertResult::Conflicted),
            Err(err) => Err(err),
        }
    }

    /// A batch's answer: `Conflicted` when the conflict clause skipped every
    /// row, otherwise the rows that were written.
    fn batch_result<T>(conflict_clause: bool, rows: Vec<T>) -> TryInsertResult<Vec<T>> {
        if rows.is_empty() && conflict_clause {
            TryInsertResult::Conflicted
        } else {
            TryInsertResult::Inserted(rows)
        }
    }
}

// [spec:pgorm:sem:exec.crud.try-insert+4]
impl<A> TryInsert<A, OneRow>
where
    A: ActiveModelTrait,
{
    /// Execute the insert and return the inserted row's primary key.
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    pub async fn exec_returning_pk<C>(
        self,
        db: &C,
    ) -> Result<TryInsertResult<InsertedPrimaryKey<A>>, Error>
    where
        C: ConnectionTrait,
    {
        if self.insert_struct.is_empty() {
            return Ok(TryInsertResult::Empty);
        }
        let res = self.insert_struct.exec_returning_pk(db).await;
        match res {
            Ok(res) => Ok(TryInsertResult::Inserted(res)),
            Err(Error::RecordNotInserted) => Ok(TryInsertResult::Conflicted),
            Err(err) => Err(err),
        }
    }

    /// Execute the insert and return the inserted row as a model.
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    pub async fn exec_returning_model<C>(
        self,
        db: &C,
    ) -> Result<TryInsertResult<InsertedModel<A>>, Error>
    where
        InsertedModel<A>: IntoActiveModel<A>,
        C: ConnectionTrait,
    {
        if self.insert_struct.is_empty() {
            return Ok(TryInsertResult::Empty);
        }
        let conflict_clause = self.has_conflict_clause();
        let res = exec_insert_returning_models::<A, C>(self.insert_struct.query, db).await;
        match res.map(|mut models| models.pop()) {
            Ok(Some(res)) => Ok(TryInsertResult::Inserted(res)),
            Ok(None) if conflict_clause => Ok(TryInsertResult::Conflicted),
            Ok(None) => Err(Error::RecordNotFound),
            Err(err) => Err(err),
        }
    }
}

// [spec:pgorm:sem:exec.crud.try-insert+4]
impl<A> TryInsert<A, ManyRows>
where
    A: ActiveModelTrait,
{
    /// Execute the insert and return the primary key of every row written, in
    /// the order the database wrote them.
    // [spec:pgorm:req:query.build.insert.uniform-columns+4]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    pub async fn exec_returning_pks<C>(
        self,
        db: &C,
    ) -> Result<TryInsertResult<Vec<InsertedPrimaryKey<A>>>, Error>
    where
        C: ConnectionTrait,
    {
        self.ensure_uniform_columns()?;
        if self.insert_struct.is_empty() {
            return Ok(TryInsertResult::Empty);
        }
        let conflict_clause = self.has_conflict_clause();
        let keys = self.insert_struct.exec_returning_pks(db).await?;
        Ok(Self::batch_result(conflict_clause, keys))
    }

    /// Execute the insert and return every row written as a model, in the
    /// order the database wrote them.
    // [spec:pgorm:req:query.build.insert.uniform-columns+4]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    pub async fn exec_returning_models<C>(
        self,
        db: &C,
    ) -> Result<TryInsertResult<Vec<InsertedModel<A>>>, Error>
    where
        InsertedModel<A>: IntoActiveModel<A>,
        C: ConnectionTrait,
    {
        self.ensure_uniform_columns()?;
        if self.insert_struct.is_empty() {
            return Ok(TryInsertResult::Empty);
        }
        let conflict_clause = self.has_conflict_clause();
        let models = self.insert_struct.exec_returning_models(db).await?;
        Ok(Self::batch_result(conflict_clause, models))
    }
}

impl<A, R> Insert<A, R>
where
    A: ActiveModelTrait,
{
    /// Execute the insert and report how many rows it wrote.
    ///
    /// No `RETURNING` clause is emitted. See `exec_returning_pk` /
    /// `exec_returning_pks` for the inserted primary keys and
    /// `exec_returning_model` / `exec_returning_models` for the rows.
    ///
    /// An insert to which no model was added writes nothing and reports `0`; a
    /// model that leaves every column `NotSet` asks for a row of database
    /// defaults and still writes one.
    ///
    /// A batch whose models do not all set the same columns contributes no row
    /// for the ones that disagree ([`Insert::add`]), and that is reported here
    /// as an error rather than as a smaller count — so a `0` means nothing was
    /// asked for, never that something was asked for and dropped.
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    // [spec:pgorm:sem:query.build.insert+5]
    // [spec:pgorm:req:query.build.insert.uniform-columns+4]
    #[doc(alias = "exec_without_returning")]
    pub async fn exec<C>(self, db: &C) -> Result<u64, Error>
    where
        C: ConnectionTrait,
    {
        self.ensure_uniform_columns()?;
        if self.has_no_models() {
            return Ok(0);
        }
        exec_insert_without_returning(self.query, db).await
    }
}

impl<A> Insert<A, OneRow>
where
    A: ActiveModelTrait,
{
    /// Execute the insert and return the inserted row's primary key.
    ///
    /// The builder holds exactly one model, so there is one row to answer for;
    /// an `ON CONFLICT DO NOTHING` that skipped it fails with
    /// [`Error::RecordNotInserted`].
    // [spec:pgorm:sem:exec.crud.insert+6]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    // [spec:pgorm:sem:query.build.insert+5]
    #[doc(alias("last_insert_id", "InsertResult"))]
    pub async fn exec_returning_pk<C>(self, db: &C) -> Result<InsertedPrimaryKey<A>, Error>
    where
        C: ConnectionTrait,
    {
        exec_insert_returning_pks::<A, _>(self.query, db)
            .await?
            .pop()
            .ok_or(Error::RecordNotInserted)
    }

    /// Execute the insert and return the inserted row as a model.
    ///
    /// An `ON CONFLICT DO NOTHING` that skipped the row fails with
    /// [`Error::RecordNotFound`].
    // [spec:pgorm:sem:exec.crud.insert-returning+3]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    // [spec:pgorm:sem:query.build.insert+5]
    #[doc(alias = "exec_with_returning")]
    pub async fn exec_returning_model<C>(self, db: &C) -> Result<InsertedModel<A>, Error>
    where
        InsertedModel<A>: IntoActiveModel<A>,
        C: ConnectionTrait,
    {
        exec_insert_returning_models::<A, _>(self.query, db)
            .await?
            .pop()
            .ok_or(Error::RecordNotFound)
    }
}

impl<A> Insert<A, ManyRows>
where
    A: ActiveModelTrait,
{
    /// Execute the insert and return the primary key of every row written, in
    /// the order the database wrote them — for `INSERT ... VALUES`, the order
    /// the models were added.
    ///
    /// An insert to which no model was added writes nothing and returns no
    /// key; a row an `ON CONFLICT DO NOTHING` skipped has no key to return. A
    /// batch whose models do not all set the same columns ([`Insert::add`])
    /// fails before anything is written.
    // [spec:pgorm:sem:exec.crud.insert+6]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    // [spec:pgorm:sem:query.build.insert+5]
    // [spec:pgorm:req:query.build.insert.uniform-columns+4]
    pub async fn exec_returning_pks<C>(self, db: &C) -> Result<Vec<InsertedPrimaryKey<A>>, Error>
    where
        C: ConnectionTrait,
    {
        self.ensure_uniform_columns()?;
        if self.has_no_models() {
            return Ok(Vec::new());
        }
        exec_insert_returning_pks::<A, _>(self.query, db).await
    }

    /// Execute the insert and return every row written as a model, in the
    /// order the database wrote them.
    ///
    /// An insert to which no model was added writes nothing and returns no
    /// model. A batch whose models do not all set the same columns
    /// ([`Insert::add`]) fails before anything is written.
    // [spec:pgorm:sem:exec.crud.insert-returning+3]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    // [spec:pgorm:sem:query.build.insert+5]
    // [spec:pgorm:req:query.build.insert.uniform-columns+4]
    #[doc(alias = "exec_with_returning")]
    pub async fn exec_returning_models<C>(self, db: &C) -> Result<Vec<InsertedModel<A>>, Error>
    where
        InsertedModel<A>: IntoActiveModel<A>,
        C: ConnectionTrait,
    {
        self.ensure_uniform_columns()?;
        if self.has_no_models() {
            return Ok(Vec::new());
        }
        exec_insert_returning_models::<A, _>(self.query, db).await
    }
}

/// The keys come from the rows the database wrote, never from the models that
/// asked for them: a manually assigned key that an `ON CONFLICT DO UPDATE` did
/// not land on names a row that does not exist. They are read in the order the
/// rows came back, which is the order they were written.
// [spec:pgorm:sem:exec.crud.insert+6]
async fn exec_insert_returning_pks<A, C>(
    mut statement: InsertStatement,
    db: &C,
) -> Result<Vec<InsertedPrimaryKey<A>>, Error>
where
    C: ConnectionTrait,
    A: ActiveModelTrait,
{
    type PrimaryKey<A> = <<A as ActiveModelTrait>::Entity as EntityTrait>::PrimaryKey;

    statement.returning(Query::returning().exprs(PrimaryKey::<A>::iter().map(|c| {
        c.into_column()
            .select_as(c.into_column().into_returning_expr())
    })));
    let (stmt, values) = statement.build();
    let values = values.into_iter().map(ValueHolder).collect::<Vec<_>>();
    let values = values
        .iter()
        .map(|x| x as _)
        .collect::<Vec<&(dyn ToSql + Sync)>>();

    let cols = PrimaryKey::<A>::iter()
        .map(|col| col.to_string())
        .collect::<Vec<_>>();
    db.query_all(&stmt, &values)
        .await?
        .into_iter()
        .map(|row| {
            QueryResult { row }
                .try_get_many("", cols.as_ref())
                .map_err(|_| Error::UnpackInsertId)
        })
        .collect()
}

// [spec:pgorm:sem:exec.crud.insert-returning+3]
async fn exec_insert_without_returning<C>(
    insert_statement: InsertStatement,
    db: &C,
) -> Result<u64, Error>
where
    C: ConnectionTrait,
{
    let (stmt, values) = insert_statement.build();
    let values = values.into_iter().map(ValueHolder).collect::<Vec<_>>();
    let values = values
        .iter()
        .map(|x| x as _)
        .collect::<Vec<&(dyn ToSql + Sync)>>();

    let exec_result = db.execute(&stmt, &values).await?;
    Ok(exec_result)
}

/// Every row the insert wrote, decoded as a model in the order the rows came
/// back; none when the conflict clause skipped them all, so callers that can
/// tell an `ON CONFLICT` skip from a genuine miss decide which it was.
// [spec:pgorm:sem:exec.crud.insert-returning+3]
async fn exec_insert_returning_models<A, C>(
    mut insert_statement: InsertStatement,
    db: &C,
) -> Result<Vec<InsertedModel<A>>, Error>
where
    InsertedModel<A>: IntoActiveModel<A>,
    C: ConnectionTrait,
    A: ActiveModelTrait,
{
    let returning = Query::returning().exprs(
        <A::Entity as EntityTrait>::Column::iter().map(|c| c.select_as(c.into_returning_expr())),
    );
    insert_statement.returning(returning);
    let (stmt, values) = insert_statement.build();

    SelectorRaw::<SelectModel<InsertedModel<A>>>::from_statement(&stmt, values)
        .all(db)
        .await
}
