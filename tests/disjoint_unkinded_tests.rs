#![allow(unused_imports, dead_code)]

//! An unkinded disjoint entity's read on a live server, over tables written
//! by hand with no kind column: each base row decoded as the kind whose
//! table has a row for it, and as the unit kind where none does; the read
//! narrowed by key, by kind through presence, by base and kind columns,
//! ordered, sliced, counted, paged and read through a keyset cursor; and the
//! rows nothing in such a schema rules out — a base row with rows in two
//! kinds' tables, and one with a row in none where no unit kind is declared —
//! reported rather than read or dropped. A shape of one refuses every write
//! and its schema, which do not compile for the typed entity.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo nextest run --test disjoint_unkinded_tests`

pub mod common;
pub use common::{TestContext, setup::*};

use futures::StreamExt;
use pgorm::tests_cfg::{
    shop::{self, Shop, ShopKind},
    storefront, web_shop,
};
use pgorm::{
    ConnectionTrait, DatabaseConnection, Disjoint, DisjointShape, DisjointValues, Error,
    RuntimeError, SelectDisjoint, entity::prelude::*,
};
use pretty_assertions::assert_eq;
use std::num::NonZeroU64;

/// The shop's tables as an existing schema holds them: each kind's table
/// keyed by a foreign key from its primary key onto the shop's, and no kind
/// column anywhere.
const SHOPS: &str = "\
    CREATE TABLE shop (id serial PRIMARY KEY, name varchar NOT NULL); \
    CREATE TABLE web_shop (shop_id integer PRIMARY KEY REFERENCES shop (id), url varchar NOT NULL); \
    CREATE TABLE storefront (id integer PRIMARY KEY REFERENCES shop (id), street varchar NOT NULL); \
    INSERT INTO shop (name) VALUES ('alpha'), ('beta'), ('gamma'), ('delta'), ('epsilon'); \
    INSERT INTO web_shop VALUES (1, 'https://alpha.example'), (4, 'https://delta.example'); \
    INSERT INTO storefront VALUES (2, 'Main Street'), (5, 'High Street');";

type Row = (shop::Model, Shop);

fn base(id: i32, name: &str) -> shop::Model {
    shop::Model {
        id,
        name: name.to_owned(),
    }
}

fn online(id: i32, name: &str, url: &str) -> Row {
    (
        base(id, name),
        Shop::WebShop(web_shop::Model {
            shop_id: id,
            url: url.to_owned(),
        }),
    )
}

fn street(id: i32, name: &str, street: &str) -> Row {
    (
        base(id, name),
        Shop::Storefront(storefront::Model {
            id,
            street: street.to_owned(),
        }),
    )
}

/// The five shops [`SHOPS`] writes, in key order: two web shops, two
/// storefronts and a dormant shop.
fn shops() -> [Row; 5] {
    [
        online(1, "alpha", "https://alpha.example"),
        street(2, "beta", "Main Street"),
        (base(3, "gamma"), Shop::Dormant),
        online(4, "delta", "https://delta.example"),
        street(5, "epsilon", "High Street"),
    ]
}

fn ids(rows: &[Row]) -> Vec<i32> {
    rows.iter().map(|(base, _)| base.id).collect()
}

fn by_id(select: SelectDisjoint<Shop>) -> SelectDisjoint<Shop> {
    select.order_by_asc(shop::Column::Id)
}

async fn shop_tables(ctx: &TestContext) -> Result<DatabaseConnection, Error> {
    let db = ctx.db.get().await?;
    db.batch_execute(SHOPS).await?;
    Ok(db)
}

// [spec:pgorm:sem:disjoint.unkinded/test]    `all`, `one` and `stream` read each base row as the kind whose table has its row, a shop with a row in neither table as the unit kind, and `find_by_id` reads one key, a key no row has being `RecordNotFound`
#[pgorm_macros::test]
async fn every_unkinded_row_reads_as_its_kind_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_unkinded_pairs").await;
    let db = shop_tables(&ctx).await?;

    assert_eq!(by_id(Shop::find()).all(&db).await?, shops());
    let streamed: Vec<Row> = by_id(Shop::find())
        .stream(&db)
        .await?
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<_, _>>()?;
    assert_eq!(streamed, shops());

    let [_, beta, gamma, ..] = shops();
    assert_eq!(Shop::find_by_id(2).one(&db).await?, beta);
    assert_eq!(Shop::find_by_id(3).one(&db).await?, gamma);
    assert_eq!(
        Shop::find_by_id(99).one(&db).await,
        Err(Error::RecordNotFound)
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.unkinded/test]    `kind` and `kinds` narrow by presence — a kind with a table to its rows, the unit kind to base rows with a row in no kind's table — and `filter_kind` to its kind's rows passing the condition, a NULL-accepting one included, beside an ordinary filter on a base column
#[pgorm_macros::test]
async fn unkinded_reads_narrow_by_presence_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_unkinded_kinds").await;
    let db = shop_tables(&ctx).await?;

    let web = by_id(Shop::find().kind(ShopKind::WebShop)).all(&db).await?;
    assert_eq!(ids(&web), [1, 4]);
    let dormant = Shop::find().kind(ShopKind::Dormant).all(&db).await?;
    assert_eq!(dormant, [(base(3, "gamma"), Shop::Dormant)]);
    let some = by_id(Shop::find().kinds([ShopKind::Storefront, ShopKind::Dormant]))
        .all(&db)
        .await?;
    assert_eq!(ids(&some), [2, 3, 5]);
    assert!(Shop::find().kinds([]).all(&db).await?.is_empty());

    let main = Shop::find()
        .filter_kind::<storefront::Model>(storefront::Column::Street.eq("Main Street"))
        .all(&db)
        .await?;
    assert_eq!(main, [street(2, "beta", "Main Street")]);
    let nameless = Shop::find()
        .filter_kind::<storefront::Model>(storefront::Column::Street.is_null())
        .all(&db)
        .await?;
    assert!(
        nameless.is_empty(),
        "no storefront has a null street, and no other kind is read: {nameless:?}"
    );
    let delta = Shop::find()
        .filter(shop::Column::Name.eq("delta"))
        .all(&db)
        .await?;
    assert_eq!(ids(&delta), [4]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.unkinded/test]    ordering, `limit`, `offset` and `count` read base rows, `paginate` pages them, and a keyset cursor over a base column reads forward and backward, a kind restriction kept
#[pgorm_macros::test]
async fn unkinded_reads_slice_page_and_cursor_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_unkinded_pages").await;
    let db = shop_tables(&ctx).await?;
    const PAGE: NonZeroU64 = NonZeroU64::new(2).unwrap();

    let window = Shop::find()
        .order_by_desc(shop::Column::Name)
        .limit(2)
        .offset(1)
        .all(&db)
        .await?;
    assert_eq!(ids(&window), [5, 4]);
    assert_eq!(Shop::find().count(&db).await?, 5);
    assert_eq!(Shop::find().kind(ShopKind::WebShop).count(&db).await?, 2);

    let mut pages = by_id(Shop::find()).paginate(&db, PAGE)?;
    assert_eq!(pages.num_items().await?, 5);
    let mut read = Vec::new();
    while let Some(page) = pages.fetch_and_next().await? {
        read.push(ids(&page));
    }
    assert_eq!(read, [vec![1, 2], vec![3, 4], vec![5]]);

    let mut cursor = Shop::find().cursor_by(shop::Column::Id)?;
    assert_eq!(ids(&cursor.after(1).first(2).all(&db).await?), [2, 3]);
    assert_eq!(
        ids(&cursor.after(i32::MIN).before(5).last(2).all(&db).await?),
        [3, 4]
    );
    let mut fronts = Shop::find()
        .kind(ShopKind::Storefront)
        .cursor_by(shop::Column::Name)?;
    assert_eq!(ids(&fronts.desc().first(5).all(&db).await?), [5, 2]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The shop's kinds with no unit kind: a shop with a row in neither table is
/// a row this declaration does not describe.
#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = shop::Entity, unkinded)]
pub enum Outlet {
    WebShop(web_shop::Model),
    Storefront(storefront::Model),
}

fn unexplained<T: std::fmt::Debug>(outcome: Result<T, Error>) -> String {
    match outcome {
        Err(Error::Query(RuntimeError::Internal(message))) => message,
        other => panic!("expected the row reported as unexplained, got {other:?}"),
    }
}

// [spec:pgorm:sem:disjoint.unkinded/test]    a base row with rows in two kinds' tables is an error naming the base table, the key and both kinds, from `all` and from a restriction to either kind, and `stream` yields it among the rows read; a base row with a row in no kind's table is one where the entity declares no unit kind
#[pgorm_macros::test]
async fn unexplained_unkinded_rows_are_errors_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_unkinded_lawless").await;
    let db = shop_tables(&ctx).await?;
    db.execute("INSERT INTO storefront VALUES (4, 'Side Street')", &[])
        .await?;

    let two = "the `shop` row whose key is (`id` = 4) of an unkinded disjoint entity has a row \
               in `web_shop`, the table of the kind `web_shop` and a row in `storefront`, the \
               table of the kind `storefront`: nothing in an unkinded entity's schema holds a \
               base row to one kind's table, so its kind cannot be told";
    let read = by_id(Shop::find())
        .stream(&db)
        .await?
        .collect::<Vec<_>>()
        .await;
    assert_eq!(read.len(), 5, "no row is dropped");
    let mut read = read.into_iter();
    let mut next = || read.next().expect("five rows were read");
    let [alpha, beta, gamma, _, epsilon] = shops();
    assert_eq!(next()?, alpha);
    assert_eq!(next()?, beta);
    assert_eq!(next()?, gamma);
    assert_eq!(unexplained(next()), two);
    assert_eq!(next()?, epsilon);

    assert_eq!(unexplained(by_id(Shop::find()).all(&db).await), two);
    assert_eq!(unexplained(Shop::find_by_id(4).one(&db).await), two);
    assert_eq!(
        unexplained(Shop::find().kind(ShopKind::WebShop).all(&db).await),
        two
    );
    assert_eq!(
        unexplained(Shop::find().kind(ShopKind::Storefront).all(&db).await),
        two
    );

    assert_eq!(
        unexplained(Outlet::find_by_id(3).one(&db).await),
        "the `shop` row whose key is (`id` = 3) of an unkinded disjoint entity has a row in \
         none of its kinds' tables, `web_shop`, `storefront`, and the entity declares no unit \
         kind for such a row"
    );
    assert_eq!(
        Outlet::find_by_id(1).one(&db).await?.1,
        Outlet::WebShop(web_shop::Model {
            shop_id: 1,
            url: "https://alpha.example".to_owned(),
        })
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A composite key in a schema of its own, each kind's key named otherwise
/// than the base's.
pub mod account {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(schema_name = "crm", table_name = "account")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub tenant: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i64,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

pub mod person {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(schema_name = "crm", table_name = "person")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub tenant_id: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub account_id: i64,
        pub given_name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

pub mod company {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(schema_name = "crm", table_name = "company")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub tenant_id: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub account_id: i64,
        pub vat: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(base = account::Entity, unkinded)]
pub enum Account {
    Person(person::Model),
    Company(company::Model),
}

// [spec:pgorm:sem:disjoint.unkinded/test]    a composite key in a schema of its own joins each kind's table column for column and narrows by presence on it
#[pgorm_macros::test]
async fn a_composite_unkinded_key_reads_live() -> Result<(), Error> {
    let ctx = TestContext::new("disjoint_unkinded_composite").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        "CREATE SCHEMA crm; \
         CREATE TABLE crm.account (tenant integer, id bigint, name varchar NOT NULL, \
             PRIMARY KEY (tenant, id)); \
         CREATE TABLE crm.person (tenant_id integer, account_id bigint, given_name varchar NOT NULL, \
             PRIMARY KEY (tenant_id, account_id), \
             FOREIGN KEY (tenant_id, account_id) REFERENCES crm.account (tenant, id)); \
         CREATE TABLE crm.company (tenant_id integer, account_id bigint, vat varchar NOT NULL, \
             PRIMARY KEY (tenant_id, account_id), \
             FOREIGN KEY (tenant_id, account_id) REFERENCES crm.account (tenant, id)); \
         INSERT INTO crm.account VALUES (7, 1, 'ada'), (7, 2, 'acme'), (8, 1, 'initech'); \
         INSERT INTO crm.person VALUES (7, 1, 'Ada'); \
         INSERT INTO crm.company VALUES (7, 2, 'GB1'), (8, 1, 'US2');",
    )
    .await?;

    let ada = Account::find_by_id((7, 1_i64)).one(&db).await?;
    assert_eq!(
        ada.1,
        Account::Person(person::Model {
            tenant_id: 7,
            account_id: 1,
            given_name: "Ada".to_owned(),
        })
    );
    let companies = Account::find()
        .kind(AccountKind::Company)
        .order_by_asc(account::Column::Tenant)
        .all(&db)
        .await?;
    assert_eq!(
        companies
            .iter()
            .map(|(base, _)| base.name.as_str())
            .collect::<Vec<_>>(),
        ["acme", "initech"]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:disjoint.unkinded/test]    a shape of an unkinded entity has no kind column, its check refuses nothing, and each write and its schema are refused as read-only before any statement is built
#[test]
fn an_unkinded_shape_refuses_writes_and_schema() {
    let shape = DisjointShape::of::<Shop>();
    assert!(shape.kind_column().is_none());
    assert!(shape.kind_type_name().is_none());
    assert_eq!(shape.check(), Ok(()));
    let read_only = "the disjoint entity over `shop` is unkinded: its base has no kind column, \
                     and nothing in its schema holds a base row to one kind's table, so it is \
                     read-only";
    let refused = |outcome: Option<Error>| match outcome {
        Some(Error::Type(message)) => message,
        other => panic!("expected the shape refused, got {other:?}"),
    };
    let values = shape
        .base()
        .values([("name", ActiveValue::Set(Value::from("omega")))])
        .expect("the base has a name");
    assert_eq!(
        refused(shape.insert(&values, "dormant", None).err()),
        read_only
    );
    assert_eq!(
        refused(shape.update(&values, "dormant", None).err()),
        read_only
    );
    assert_eq!(
        refused(
            shape
                .change_kind(vec![Value::from(1)], "dormant", None)
                .err()
        ),
        read_only
    );
    assert_eq!(refused(shape.create().err()), read_only);
    assert_eq!(
        shape
            .kinds()
            .iter()
            .map(|kind| kind.value())
            .collect::<Vec<_>>(),
        ["web_shop", "storefront", "dormant"]
    );
}
