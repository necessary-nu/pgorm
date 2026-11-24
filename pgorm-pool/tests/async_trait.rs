// This code must compile even without the async_trait crate
// See: https://github.com/bikeshedder/deadpool/issues/323

use futures_util::{Stream, StreamExt};
use pgorm_driver::types::ToSql;
use pgorm_pool::{GenericClient, pgorm_driver::Row};
use std::future::Future;

// this function borrowed from pgorm_driver source code
fn slice_iter<'a>(
    s: &'a [&'a (dyn ToSql + Sync)],
) -> impl ExactSizeIterator<Item = &'a dyn ToSql> + 'a {
    s.iter().map(|s| *s as _)
}

pub trait PgQuery {
    fn query_raw(
        db: &impl GenericClient,
        params: &[&(dyn ToSql + Sync)],
    ) -> impl Future<Output = impl Stream<Item = Row>> + Send {
        async {
            let rows = db.query_raw("SELECT 1", slice_iter(params)).await.unwrap();
            rows.map(|row| row.unwrap())
        }
    }
}
