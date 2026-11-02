use crate::{ActiveModelTrait, ConnectionTrait, DeleteMany, DeleteOne, EntityTrait, error::*};
use pgorm_query::DeleteStatement;
use tokio_postgres::types::ToSql;

use super::ValueHolder;

impl<A> DeleteOne<A>
where
    A: ActiveModelTrait,
{
    /// Execute the delete and report how many rows it removed.
    // [spec:pgorm:sem:exec.crud.delete+1]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    #[doc(alias = "DeleteResult")]
    pub async fn exec<C>(self, db: &C) -> Result<u64, Error>
    where
        C: ConnectionTrait,
    {
        exec_delete(self.query, db).await
    }
}

impl<E> DeleteMany<E>
where
    E: EntityTrait,
{
    /// Execute the delete and report how many rows it removed.
    // [spec:pgorm:sem:exec.crud.delete+1]
    // [spec:pgorm:req:exec.crud.exec-vocabulary+2]
    #[doc(alias = "DeleteResult")]
    pub async fn exec<C>(self, db: &C) -> Result<u64, Error>
    where
        C: ConnectionTrait,
    {
        exec_delete(self.query, db).await
    }
}

// [spec:pgorm:sem:exec.crud.delete+1]
async fn exec_delete<C>(query: DeleteStatement, db: &C) -> Result<u64, Error>
where
    C: ConnectionTrait,
{
    let (stmt, values) = query.build();
    let values = values.into_iter().map(ValueHolder).collect::<Vec<_>>();
    let values = values
        .iter()
        .map(|x| x as _)
        .collect::<Vec<&(dyn ToSql + Sync)>>();

    db.execute(&stmt, &values).await
}
