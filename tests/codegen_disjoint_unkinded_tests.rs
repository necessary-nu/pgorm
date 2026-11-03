#![allow(unused_imports, dead_code)]

//! The unkinded disjoint entities pgorm-codegen generates from tables keyed
//! by a foreign key from their primary key onto a base's, compiled, and read
//! through against a live server.
//!
//! pgorm-codegen's own tests hold the writer's output to the files of
//! `codegen_disjoint_unkinded`, so what is compiled and run here is what the
//! generator writes. The round trip closes here: the schema the files were
//! generated from runs as written, and the generated enums read its rows by
//! which kind's table has a row for them, the generated relations joining
//! each kind's row to its base row.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo nextest run --test codegen_disjoint_unkinded_tests`

pub mod common;
pub use common::{TestContext, setup::*};

use pgorm::entity::prelude::*;
use pgorm::{ConnectionTrait, Disjoint, check_disjoint};
use pretty_assertions::assert_eq;

mod codegen_disjoint_unkinded;

use codegen_disjoint_unkinded::account::{Account, AccountKind};
use codegen_disjoint_unkinded::shop::{Shop, ShopKind};
use codegen_disjoint_unkinded::{account, company, person, shop, storefront, website};

const SCHEMA: &str = include_str!("codegen_disjoint_unkinded/schema.sql");

// [spec:pgorm:sem:codegen.entity.disjoint.unkinded/test]    the round trip: the schema the fixtures were generated from runs as written, the generated unkinded enums read each base row as the kind whose table has its row, the shop's dormant row as its unit kind and the account's composite rows by their keys, narrowed by kind, and each kind's row reaches its base row through the generated relations
#[pgorm_macros::test]
async fn generated_unkinded_entities_read_live() -> Result<(), Error> {
    let ctx = TestContext::new("codegen_disjoint_unkinded").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SCHEMA).await?;
    db.batch_execute(
        "INSERT INTO shop (name) VALUES ('alpha'), ('beta'), ('gamma'); \
         INSERT INTO website VALUES (1, 'https://alpha.example'); \
         INSERT INTO storefront VALUES (2, 'Main Street'); \
         INSERT INTO crm.account VALUES (7, 1, 'ada'), (7, 2, 'acme'); \
         INSERT INTO crm.person VALUES (7, 1, 'Ada'); \
         INSERT INTO crm.company VALUES (7, 2, 'GB1');",
    )
    .await?;
    check_disjoint::<Shop>().expect("an unkinded entity refuses nothing");
    check_disjoint::<Account>().expect("an unkinded entity refuses nothing");

    let shops = Shop::find().order_by_asc(shop::Column::Id).all(&db).await?;
    assert_eq!(
        shops.into_iter().map(|(_, kind)| kind).collect::<Vec<_>>(),
        [
            Shop::Website(website::Model {
                shop_id: 1,
                url: "https://alpha.example".to_owned(),
            }),
            Shop::Storefront(storefront::Model {
                id: 2,
                street: "Main Street".to_owned(),
            }),
            Shop::Dormant,
        ]
    );
    let dormant = Shop::find().kind(ShopKind::Dormant).one(&db).await?;
    assert_eq!(dormant.0.name, "gamma");

    let acme = Account::find_by_id((7, 2_i64)).one(&db).await?;
    assert_eq!(
        acme.1,
        Account::Company(company::Model {
            tenant_id: 7,
            account_id: 2,
            vat: "GB1".to_owned(),
        })
    );
    let people = Account::find().kind(AccountKind::Person).all(&db).await?;
    assert_eq!(
        people
            .iter()
            .map(|(base, _)| base.name.as_str())
            .collect::<Vec<_>>(),
        ["ada"]
    );

    let alpha = shop::Entity::find_by_id(1).one(&db).await?;
    let site = alpha.find_related(website::Entity).one(&db).await?;
    assert_eq!(site.find_related(shop::Entity).one(&db).await?, alpha);
    let ada = person::Entity::find_by_id((7, 1_i64)).one(&db).await?;
    assert_eq!(
        ada.find_related(account::Entity).one(&db).await?.name,
        "ada"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
