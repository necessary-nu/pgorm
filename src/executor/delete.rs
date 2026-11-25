use crate::{ActiveModel, Connection, DeleteMany, DeleteOne, Entity, error::*};
use pgorm_driver::types::ToSql;
use pgorm_query::DeleteStatement;

use super::ValueHolder;

impl<A> DeleteOne<A>
where
    A: ActiveModel,
{
    /// Execute the delete and report how many rows it removed.
    // [spec:pgorm:sem:exec.crud.delete+1]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    #[doc(alias = "DeleteResult")]
    pub async fn exec<C>(self, db: &C) -> Result<u64, Error>
    where
        C: Connection,
    {
        exec_delete(self.query, db).await
    }
}

impl<E> DeleteMany<E>
where
    E: Entity,
{
    /// Execute the delete and report how many rows it removed.
    // [spec:pgorm:sem:exec.crud.delete+1]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    #[doc(alias = "DeleteResult")]
    pub async fn exec<C>(self, db: &C) -> Result<u64, Error>
    where
        C: Connection,
    {
        self.refusal()?;
        exec_delete(self.query, db).await
    }
}

// [spec:pgorm:sem:exec.crud.delete+1]
async fn exec_delete<C>(query: DeleteStatement, db: &C) -> Result<u64, Error>
where
    C: Connection,
{
    let (stmt, values) = query.build();
    let values = values.into_iter().map(ValueHolder).collect::<Vec<_>>();
    #[cfg(feature = "turso")]
    let stmt = crate::database::ValueSql::new(&stmt, &values);
    let values = values
        .iter()
        .map(|x| x as _)
        .collect::<Vec<&(dyn ToSql + Sync)>>();

    db.execute(&stmt, &values).await
}
