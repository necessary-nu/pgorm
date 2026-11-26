#![allow(unused_imports, dead_code)]

use crate::common;

use common::{TestContext, features::*, setup::*};
use pgorm::{entity::prelude::*, entity::*};
use pretty_assertions::assert_eq;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("pi_tests").await;
    create_tables(&ctx.db).await?;

    let db = ctx.db.get().await?;
    create_and_update_pi(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

// The literals are the fixture: this exercises decimal round-tripping of pi.
#[allow(clippy::approx_constant)]
pub async fn create_and_update_pi(db: &DatabaseConnection) -> Result<(), Error> {
    let pi = pi::Model {
        id: 1,
        decimal: rust_dec(3.1415926536),
        decimal_opt: None,
    };

    let res = pi.clone().into_active_model().insert(db).await?;

    let model = Pi::find().one_opt(db).await?;
    assert_eq!(model, Some(res));
    assert_eq!(model, Some(pi.clone()));

    let res = pi::ActiveModel {
        decimal_opt: set(Some(rust_dec(3.1415926536))),
        ..pi.clone().into_active_model()
    }
    .update(db)
    .await?;

    let model = Pi::find().one_opt(db).await?;
    assert_eq!(model, Some(res));
    assert_eq!(
        model,
        Some(pi::Model {
            id: 1,
            decimal: rust_dec(3.1415926536),
            decimal_opt: Some(rust_dec(3.1415926536)),
        })
    );

    Ok(())
}
