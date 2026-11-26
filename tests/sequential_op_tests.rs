#![allow(unused_imports, dead_code)]

use crate::common;

use common::{TestContext, bakery_chain::*, setup::*};
use jiff::{Timestamp, tz::Offset};
use pgorm::{DatabaseConnection, FromQueryResult, entity::*, query::*, set};
use rust_decimal::prelude::*;
use uuid::Uuid;

// Run the test locally:
// DATABASE_URL="postgres://postgres:postgres@localhost" cargo test --test it -- sequential_op_tests::
#[pgorm_macros::test]
pub async fn test_multiple_operations() {
    let ctx = TestContext::new("multiple_sequential_operations").await;

    create_tables(&ctx.db).await.unwrap();

    let db = ctx.db.get().await.unwrap();

    seed_data(&db).await;
    let baker_least_sales = find_baker_least_sales(&db).await.unwrap();
    assert_eq!(baker_least_sales.name, "Baker 2");

    let new_cake = create_cake(&db, baker_least_sales).await.unwrap();
    create_order(&db, new_cake).await;

    let baker_least_sales = find_baker_least_sales(&db).await.unwrap();
    assert_eq!(baker_least_sales.name, "Baker 1");

    drop(db);

    ctx.delete().await;
}

// [spec:pgorm:sem:exec.crud.insert+6/test]    a composite primary key comes back
// as the entity's `PrimaryKey::ValueType` tuple
async fn seed_data(db: &DatabaseConnection) {
    let bakery = bakery::ActiveModel {
        name: set("SeaSide Bakery"),
        profit_margin: set(10.4),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("could not insert bakery");

    let baker_1 = baker::ActiveModel {
        name: set("Baker 1"),
        contact_details: set(serde_json::json!({})),
        bakery_id: set(Some(bakery.id)),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("could not insert baker");

    let _baker_2 = baker::ActiveModel {
        name: set("Baker 2"),
        contact_details: set(serde_json::json!({})),
        bakery_id: set(Some(bakery.id)),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("could not insert baker");

    let mud_cake = cake::ActiveModel {
        name: set("Mud Cake"),
        price: set(rust_dec(10.25)),
        gluten_free: set(false),
        serial: set(Uuid::new_v4()),
        bakery_id: set(Some(bakery.id)),
        ..Default::default()
    };

    let cake_insert_res = Insert::one(mud_cake)
        .exec_returning_pk(db)
        .await
        .expect("could not insert cake");

    let cake_baker = cakes_bakers::ActiveModel {
        cake_id: set(cake_insert_res),
        baker_id: set(baker_1.id),
    };

    let cake_baker_res = Insert::one(cake_baker.clone())
        .exec_returning_pk(db)
        .await
        .expect("could not insert cake_baker");
    assert_eq!(
        cake_baker_res,
        (cake_baker.cake_id.unwrap(), cake_baker.baker_id.unwrap())
    );

    let customer_kate = customer::ActiveModel {
        name: set("Kate"),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("could not insert customer");

    let kate_order_1 = order::ActiveModel {
        bakery_id: set(bakery.id),
        customer_id: set(customer_kate.id),
        total: set(rust_dec(99.95)),
        placed_at: set(Offset::UTC.to_datetime(Timestamp::now())),

        ..Default::default()
    }
    .insert(db)
    .await
    .expect("could not insert order");

    let _lineitem = lineitem::ActiveModel {
        cake_id: set(cake_insert_res),
        price: set(rust_dec(10.00)),
        quantity: set(12),
        order_id: set(kate_order_1.id),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("could not insert order");

    let _lineitem2 = lineitem::ActiveModel {
        cake_id: set(cake_insert_res),
        price: set(rust_dec(50.00)),
        quantity: set(2),
        order_id: set(kate_order_1.id),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("could not insert order");
}

async fn find_baker_least_sales(db: &DatabaseConnection) -> Option<baker::Model> {
    #[derive(Debug, FromQueryResult)]
    struct SelectResult {
        id: i32,
        cakes_sold_opt: Option<i64>,
    }

    #[derive(Debug)]
    struct LeastSalesBakerResult {
        id: i32,
        cakes_sold: Decimal,
    }

    let rel: RelationDef = cakes_bakers::Entity::belongs_to(baker::Entity)
        .columns(cakes_bakers::Column::BakerId, baker::Column::Id)
        .into();

    let rel2: RelationDef = cakes_bakers::Entity::belongs_to(cake::Entity)
        .columns(cakes_bakers::Column::CakeId, cake::Column::Id)
        .into();

    let rel3: RelationDef = cake::Entity::has_many(lineitem::Entity)
        .columns(cake::Column::Id, lineitem::Column::CakeId)
        .into();

    let select = cakes_bakers::Entity::find()
        .join(JoinType::RightJoin, rel)
        .join(JoinType::LeftJoin, rel2)
        .join(JoinType::LeftJoin, rel3)
        .select(baker::Column::Id)
        .column_as(
            lineitem::Column::Quantity.sum(),
            Name::runtime("cakes_sold_opt"),
        )
        .group_by(baker::Column::Id);

    let mut results: Vec<LeastSalesBakerResult> = select
        .into_model::<SelectResult>()
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|b| LeastSalesBakerResult {
            id: b.id,
            cakes_sold: b.cakes_sold_opt.unwrap_or_default().into(),
        })
        .collect();

    results.sort_by_key(|r| std::cmp::Reverse(r.cakes_sold));

    Baker::find_by_id(results.last().unwrap().id)
        .one_opt(db)
        .await
        .unwrap()
}

async fn create_cake(db: &DatabaseConnection, baker: baker::Model) -> Option<cake::Model> {
    let new_cake = cake::ActiveModel {
        name: set("New Cake"),
        price: set(rust_dec(8.00)),
        gluten_free: set(false),
        serial: set(Uuid::new_v4()),
        bakery_id: set(Some(baker.bakery_id.unwrap())),
        ..Default::default()
    };

    let cake_insert_res = Insert::one(new_cake)
        .exec_returning_pk(db)
        .await
        .expect("could not insert cake");

    let cake_baker = cakes_bakers::ActiveModel {
        cake_id: set(cake_insert_res),
        baker_id: set(baker.id),
    };

    let cake_baker_res = Insert::one(cake_baker.clone())
        .exec_returning_pk(db)
        .await
        .expect("could not insert cake_baker");
    assert_eq!(
        cake_baker_res,
        (cake_baker.cake_id.unwrap(), cake_baker.baker_id.unwrap())
    );

    Cake::find_by_id(cake_insert_res).one_opt(db).await.unwrap()
}

async fn create_order(db: &DatabaseConnection, cake: cake::Model) {
    let another_customer = customer::ActiveModel {
        name: set("John"),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("could not insert customer");

    let order = order::ActiveModel {
        bakery_id: set(cake.bakery_id.unwrap()),
        customer_id: set(another_customer.id),
        total: set(rust_dec(200.00)),
        placed_at: set(Offset::UTC.to_datetime(Timestamp::now())),

        ..Default::default()
    }
    .insert(db)
    .await
    .expect("could not insert order");

    let _lineitem = lineitem::ActiveModel {
        cake_id: set(cake.id),
        price: set(rust_dec(10.00)),
        quantity: set(300),
        order_id: set(order.id),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("could not insert order");
}
