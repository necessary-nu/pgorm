#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, bakery_chain::*, setup::*};
use entity::{Column, Entity};
use pgorm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, Error, FromQueryResult, ModelTrait, PartialModel,
    QueryOrder, set,
};
use pgorm_query::{Expr, ExprTrait, Func, Name};

mod entity {
    use pgorm::prelude::*;

    #[derive(Debug, Clone, Entity)]
    #[pgorm(table_name = "foo_table")]
    pub struct Model {
        #[pgorm(primary_key)]
        id: i32,
        foo: i32,
        bar: String,
        foo2: bool,
        bar2: f64,
    }

    #[derive(Debug, Relation)]
    pub enum Relation {}
}

#[derive(PartialModel)]
#[pgorm(entity = Entity)]
struct SimpleTest {
    _foo: i32,
    _bar: String,
}

#[derive(PartialModel)]
#[pgorm(entity = <entity::Model as ModelTrait>::Entity)]
struct EntityNameNotAIdent {
    #[pgorm(from_col = <<entity::Model as ModelTrait>::Entity as EntityTrait>::Column::Foo2)]
    _foo: i32,
    #[pgorm(from_col = Column::Bar2)]
    _bar: String,
}

#[derive(PartialModel)]
#[pgorm(entity = Entity)]
struct FieldFromDiffNameColumnTest {
    #[pgorm(from_col = Column::Foo2)]
    _foo: i32,
    #[pgorm(from_col = Column::Bar2)]
    _bar: String,
}

#[derive(PartialModel)]
struct FieldFromExpr {
    #[pgorm(from_expr = Column::Bar2.sum())]
    _foo: f64,
    #[pgorm(from_expr = Expr::col(Column::Id).equals(Column::Foo))]
    _bar: bool,
}

#[derive(Debug, PartialEq, PartialModel)]
#[pgorm(entity = bakery::Entity)]
struct PartialBakery {
    id: i32,
    #[pgorm(from_col = bakery::Column::Name)]
    title: String,
    #[pgorm(from_expr = Expr::col(bakery::Column::ProfitMargin).mul(2.0))]
    double_margin: f64,
    #[pgorm(from_expr = Func::upper(Expr::col(bakery::Column::Name)))]
    shout: String,
}

#[derive(Debug, PartialEq, PartialModel)]
#[pgorm(entity = bakery::Entity)]
struct MarginTotal {
    #[pgorm(from_expr = bakery::Column::ProfitMargin.sum())]
    total: f64,
}

#[derive(Debug, PartialEq, PartialModel)]
#[pgorm(entity = bakery::Entity)]
struct IntScaledBakery {
    id: i32,
    #[pgorm(from_expr = Expr::col(bakery::Column::ProfitMargin).mul(2))]
    double_margin: f64,
}

// [spec:pgorm:def:exec.crud+1/test]    `Select::into_partial_model` re-targets
// the decoded type, and `one` / `all` run through the same selector
// [spec:pgorm:def:query.build.query-trait+2/test]    a field read from a bare
// function call, `from_expr = Func::upper(..)`
#[pgorm_macros::test]
async fn partial_model_select() -> Result<(), Error> {
    let ctx = TestContext::new("partial_model_select").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    bakery::ActiveModel {
        name: set("SeaSide Bakery"),
        profit_margin: set(10.5),
        ..Default::default()
    }
    .insert(&db)
    .await?;

    bakery::ActiveModel {
        name: set("Top Bakery"),
        profit_margin: set(4.5),
        ..Default::default()
    }
    .insert(&db)
    .await?;

    let bakeries = bakery::Entity::find()
        .order_by_asc(bakery::Column::Id)
        .into_partial_model::<PartialBakery>()
        .all(&db)
        .await?;

    assert_eq!(
        bakeries,
        [
            PartialBakery {
                id: 1,
                title: "SeaSide Bakery".to_owned(),
                double_margin: 21.0,
                shout: "SEASIDE BAKERY".to_owned(),
            },
            PartialBakery {
                id: 2,
                title: "Top Bakery".to_owned(),
                double_margin: 9.0,
                shout: "TOP BAKERY".to_owned(),
            },
        ]
    );

    let total = bakery::Entity::find()
        .into_partial_model::<MarginTotal>()
        .one(&db)
        .await?;

    assert_eq!(total, MarginTotal { total: 15.0 });

    drop(db);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:exec.cursor.binding-coerce+2/test]
#[pgorm_macros::test]
async fn integer_operand_against_float_column() -> Result<(), Error> {
    let ctx = TestContext::new("partial_model_int_operand_bindtypes").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    bakery::ActiveModel {
        name: set("SeaSide Bakery"),
        profit_margin: set(10.5),
        ..Default::default()
    }
    .insert(&db)
    .await?;

    let bakeries = bakery::Entity::find()
        .order_by_asc(bakery::Column::Id)
        .into_partial_model::<IntScaledBakery>()
        .all(&db)
        .await?;

    assert_eq!(
        bakeries,
        [IntScaledBakery {
            id: 1,
            double_margin: 21.0,
        }]
    );

    drop(db);
    ctx.delete().await;

    Ok(())
}
