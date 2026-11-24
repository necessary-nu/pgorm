use crate::client::InnerClient;
use crate::native::{self, codecs};
use crate::types::{BorrowToSql, Type};
use crate::{Error, RowStream, Statement, prepare};
use std::sync::Arc;

fn parameters<P, I>(
    client: &InnerClient,
    actor: &native::Client,
    statement: &Statement,
    params: I,
) -> Result<Vec<turso_core::Value>, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = P>,
    I::IntoIter: ExactSizeIterator,
{
    statement.native_id(actor)?;
    let codecs = statement
        .native_codecs()
        .ok_or_else(|| Error::turso_unsupported("a statement without native parameter codecs"))?;
    client.with_buf(|buffer| codecs::encode_params(statement.params(), codecs, params, buffer))
}

pub(crate) async fn query<P, I>(
    client: &InnerClient,
    actor: &native::Client,
    statement: Statement,
    params: I,
) -> Result<RowStream, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = P>,
    I::IntoIter: ExactSizeIterator,
{
    let values = parameters(client, actor, &statement, params)?;
    let rows = actor.query_prepared(statement.clone(), values).await?;
    Ok(RowStream::from_native(statement, rows))
}

pub(crate) async fn execute<P, I>(
    client: &InnerClient,
    actor: &native::Client,
    statement: Statement,
    params: I,
) -> Result<u64, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = P>,
    I::IntoIter: ExactSizeIterator,
{
    let values = parameters(client, actor, &statement, params)?;
    actor.execute_prepared(statement, values).await
}

pub(crate) async fn query_typed<P, I>(
    client: &Arc<InnerClient>,
    actor: &native::Client,
    sql: &str,
    params: I,
) -> Result<RowStream, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = (P, Type)>,
{
    let (types, values) = typed_parameters(client, params)?;
    let statement = prepare::prepare(client, sql, &types).await?;
    let rows = actor.query_prepared(statement.clone(), values).await?;
    Ok(RowStream::from_native(statement, rows))
}

pub(crate) async fn execute_typed<P, I>(
    client: &Arc<InnerClient>,
    actor: &native::Client,
    sql: &str,
    params: I,
) -> Result<u64, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = (P, Type)>,
{
    let (types, values) = typed_parameters(client, params)?;
    let statement = prepare::prepare(client, sql, &types).await?;
    actor.execute_prepared(statement, values).await
}

// BorrowToSql values need not be Send. Encode them before the first await so
// native execution retains the generic client's existing iterator bounds.
fn typed_parameters<P, I>(
    client: &InnerClient,
    params: I,
) -> Result<(Vec<Type>, Vec<turso_core::Value>), Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = (P, Type)>,
{
    let (params, types): (Vec<_>, Vec<_>) = params.into_iter().unzip();
    let codecs = codecs::parameter_codecs(&types)?;
    let values =
        client.with_buf(|buffer| codecs::encode_params(&types, &codecs, params, buffer))?;
    Ok((types, values))
}
