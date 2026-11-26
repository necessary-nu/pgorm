use pgorm::entity::prelude::*;
use pgorm::pgorm_query::Values;
use pgorm::tests_cfg::cake;
use pgorm::{SelectModel, SelectorRaw};

async fn read(db: &ReadOnlyConnection, sql: String) -> Result<Vec<cake::Model>, Error> {
    SelectorRaw::<SelectModel<cake::Model>>::from_statement(&sql, Values(vec![]))
        .all(db)
        .await
}

fn main() {
    let _ = read;
}
