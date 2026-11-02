use crate::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, Iterable,
    QueryFilter, SelectModel, SelectorRaw, UpdateMany, UpdateOne, error::*,
};
use pgorm_query::{Query, UpdateStatement};
use tokio_postgres::types::ToSql;

use super::ValueHolder;

impl<A> UpdateOne<A>
where
    A: ActiveModelTrait,
{
    /// Execute the update and return the updated row as a model.
    ///
    /// `UpdateOne` has no bare `exec`: updating a single model by primary key
    /// always reads the row back. Use [`Update::many`](crate::Update::many)
    /// filtered to the key when a rows-affected count is all that is wanted.
    ///
    /// Nothing to set is not an error here: this terminal promises the model,
    /// the statement names one row by its primary key, and so the model can
    /// still be read under the statement's own `WHERE`. The many-row terminals
    /// have no such row to fall back to and report
    /// [`Error::NothingToSet`](crate::Error::NothingToSet) instead.
    // [spec:pgorm:sem:exec.crud.update+7]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    pub async fn exec_returning_model<C>(
        mut self,
        db: &C,
    ) -> Result<<A::Entity as EntityTrait>::Model, Error>
    where
        <A::Entity as EntityTrait>::Model: IntoActiveModel<A>,
        C: ConnectionTrait,
    {
        type Entity<A> = <A as ActiveModelTrait>::Entity;
        type Model<A> = <Entity<A> as EntityTrait>::Model;
        type Column<A> = <Entity<A> as EntityTrait>::Column;

        if self.query.get_values().is_empty() {
            return find_unchanged_model::<A, C>(&self.query, db).await;
        }

        let returning = Query::returning()
            .exprs(Column::<A>::iter().map(|c| c.select_as(c.into_returning_expr())));
        self.query.returning(returning);

        let (stmt, values) = self.query.build();

        let found: Model<A> = SelectorRaw::<SelectModel<Model<A>>>::from_statement(stmt, values)
            .one(db)
            .await?;

        Ok(found)
    }
}

impl<E> UpdateMany<E>
where
    E: EntityTrait,
{
    /// Execute the update and report how many rows it changed.
    ///
    /// No `RETURNING` clause is emitted. See [`Self::exec_returning_models`] for
    /// the updated rows.
    ///
    /// An update naming no column to set is [`Error::NothingToSet`], not
    /// `Ok(0)`: a zero count is the answer to "how many rows did the `WHERE`
    /// match", and an update that was never sent has no such answer. The case
    /// is easy to reach —
    /// [`UpdateMany::set`](crate::UpdateMany::set) skips `Unchanged` and
    /// `NotSet` fields, so a model read back from the database and handed
    /// straight to `set` contributes nothing.
    // [spec:pgorm:sem:exec.crud.update+7]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    #[doc(alias = "UpdateResult")]
    pub async fn exec<C>(self, db: &C) -> Result<u64, Error>
    where
        C: ConnectionTrait,
    {
        if self.query.get_values().is_empty() {
            return Err(Error::NothingToSet);
        }
        let (stmt, values) = self.query.build();
        let values = values.into_iter().map(ValueHolder).collect::<Vec<_>>();
        let values = values
            .iter()
            .map(|x| x as _)
            .collect::<Vec<&(dyn ToSql + Sync)>>();

        db.execute(&stmt, &values).await
    }

    /// Execute the update and return every updated row as a model.
    ///
    /// An update naming no column to set is [`Error::NothingToSet`], on the
    /// same terms as [`Self::exec`]: the two terminals of one builder answer
    /// that input identically, so which terminal you reach for cannot change
    /// whether the statement was sent.
    // [spec:pgorm:sem:exec.crud.update+7]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    #[doc(alias = "exec_with_returning")]
    pub async fn exec_returning_models<C>(mut self, db: &C) -> Result<Vec<E::Model>, Error>
    where
        C: ConnectionTrait,
    {
        if self.query.get_values().is_empty() {
            return Err(Error::NothingToSet);
        }

        let returning = Query::returning()
            .exprs(E::Column::iter().map(|c| c.select_as(c.into_returning_expr())));

        self.query.returning(returning);

        let (stmt, values) = self.query.build();

        let models: Vec<E::Model> =
            SelectorRaw::<SelectModel<E::Model>>::from_statement(stmt, values)
                .all(db)
                .await?;

        Ok(models)
    }
}

// [spec:pgorm:sem:exec.crud.update+7]
pub(super) async fn find_unchanged_model<A, C>(
    query: &UpdateStatement,
    db: &C,
) -> Result<<A::Entity as EntityTrait>::Model, Error>
where
    A: ActiveModelTrait,
    C: ConnectionTrait,
{
    let condition = query
        .where_condition()
        .cloned()
        .ok_or(Error::PrimaryKeyNotSet)?;

    <A::Entity as EntityTrait>::find()
        .filter(condition)
        .one(db)
        .await
}
