#![allow(unused_imports, dead_code)]

use pgorm_query::ExprTrait;
pub mod common;

pub use common::{TestContext, bakery_chain::*, setup::*};
use pgorm::alias;
use pgorm::pgorm_query::{ConditionType, Expr, IntoCondition};
use pgorm::{DatabaseConnection, Error, RuntimeError, Schema, entity::*, query::*, set};

// [spec:pgorm:req:query.loader+1/test]    `load_one` over a `Vec<M>`, taking a
// bare entity through `EntityOrSelect`, returning `Vec<Option<R::Model>>`
// positionally aligned with the input, and rejecting a `HasMany` relation
// [spec:pgorm:sem:query.loader.regroup+6/test]    the belongs-to direction is
// where the join repeats one target row per matching input row: two bakers of
// one bakery put that bakery in its key's bucket twice, and counting by the
// target's own primary key is what collapses the repeat instead of reading it
// as a second row
#[pgorm_macros::test]
async fn loader_load_one() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_load_one").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;

    let bakery_0 = insert_bakery(db, "SeaSide Bakery").await?;

    let baker_1 = insert_baker(db, "Baker 1", bakery_0.id).await?;
    let baker_2 = insert_baker(db, "Baker 2", bakery_0.id).await?;
    let baker_3 = baker::ActiveModel {
        name: set("Baker 3"),
        contact_details: set(serde_json::json!({})),
        bakery_id: set(None),
        ..Default::default()
    }
    .insert(db)
    .await?;

    let bakers = baker::Entity::find().all(db).await?;
    let bakeries = bakers.load_one(bakery::Entity, db).await?;

    assert_eq!(bakers, [baker_1, baker_2, baker_3]);
    assert_eq!(bakeries, [Some(bakery_0.clone()), Some(bakery_0), None]);

    // has many find, should use load_many instead
    let bakeries = bakery::Entity::find().all(db).await?;
    let bakers = bakeries.load_one(baker::Entity, db).await;

    assert_eq!(
        bakers,
        Err(Error::Query(RuntimeError::Internal(
            "Relation is HasMany instead of HasOne".to_string()
        )))
    );

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:query.loader+1/test]    `load_many` returning `Vec<Vec<..>>`
// aligned with the input, driven from both a bare entity and a pre-filtered
// `Select<R>`
// [spec:pgorm:sem:query.loader.regroup+6/test]    a bucket per input key in
// result order, an empty `Vec` for an input nothing matched, and a clone of
// the same model for two inputs sharing a key
#[pgorm_macros::test]
async fn loader_load_many() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_load_many").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;

    let bakery_1 = insert_bakery(db, "SeaSide Bakery").await?;
    let bakery_2 = insert_bakery(db, "Offshore Bakery").await?;
    let bakery_3 = insert_bakery(db, "Rocky Bakery").await?;

    let baker_1 = insert_baker(db, "Baker 1", bakery_1.id).await?;
    let baker_2 = insert_baker(db, "Baker 2", bakery_1.id).await?;

    let baker_3 = insert_baker(db, "John", bakery_2.id).await?;
    let baker_4 = insert_baker(db, "Baker 4", bakery_2.id).await?;

    let bakeries = bakery::Entity::find().all(db).await?;
    let bakers = bakeries.load_many(baker::Entity, db).await?;

    assert_eq!(
        bakeries,
        [bakery_1.clone(), bakery_2.clone(), bakery_3.clone()]
    );
    assert_eq!(
        bakers,
        [
            vec![baker_1.clone(), baker_2.clone()],
            vec![baker_3.clone(), baker_4.clone()],
            vec![]
        ]
    );

    // load bakers again but with additional condition

    let bakers = bakeries
        .load_many(
            baker::Entity::find().filter(baker::Column::Name.like("Baker%")),
            db,
        )
        .await?;

    assert_eq!(
        bakers,
        [
            vec![baker_1.clone(), baker_2.clone()],
            vec![baker_4.clone()],
            vec![]
        ]
    );

    // now, start from baker

    let bakers = baker::Entity::find().all(db).await?;
    let bakeries = bakers.load_one(bakery::Entity::find(), db).await?;

    // note that two bakers share the same bakery
    assert_eq!(bakers, [baker_1, baker_2, baker_3, baker_4]);
    assert_eq!(
        bakeries,
        [
            Some(bakery_1.clone()),
            Some(bakery_1),
            Some(bakery_2.clone()),
            Some(bakery_2)
        ]
    );

    drop(conn);
    ctx.delete().await;

    Ok(())
}

#[pgorm_macros::test]
async fn loader_load_many_multi() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_load_many_multi").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;

    let bakery_1 = insert_bakery(db, "SeaSide Bakery").await?;
    let bakery_2 = insert_bakery(db, "Offshore Bakery").await?;

    let baker_1 = insert_baker(db, "John", bakery_1.id).await?;
    let baker_2 = insert_baker(db, "Jane", bakery_1.id).await?;
    let baker_3 = insert_baker(db, "Peter", bakery_2.id).await?;

    let cake_1 = insert_cake(db, "Cheesecake", Some(bakery_1.id)).await?;
    let cake_2 = insert_cake(db, "Chocolate", Some(bakery_2.id)).await?;
    let cake_3 = insert_cake(db, "Chiffon", Some(bakery_2.id)).await?;
    let _cake_4 = insert_cake(db, "Apple Pie", None).await?; // no one makes apple pie

    let bakeries = bakery::Entity::find().all(db).await?;
    let bakers = bakeries.load_many(baker::Entity, db).await?;
    let cakes = bakeries.load_many(cake::Entity, db).await?;

    assert_eq!(bakeries, [bakery_1, bakery_2]);
    assert_eq!(bakers, [vec![baker_1, baker_2], vec![baker_3]]);
    assert_eq!(cakes, [vec![cake_1], vec![cake_2, cake_3]]);

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:query.loader+1/test]    `load_many_via` across a junction,
// from a bare entity and from a pre-filtered `Select<R>` — the junction entity
// is never named by the caller
// [spec:pgorm:sem:query.loader.many-to-many+3/test]    one join carries the
// input key back with each target row: a shared target is cloned into every
// referencing input, a target the caller's `Select` filtered away is dropped
// from that input's list, and the caller's `order_by` orders every bucket
#[pgorm_macros::test]
async fn loader_load_many_via() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_load_many_via").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;

    let bakery_1 = insert_bakery(db, "SeaSide Bakery").await?;

    let baker_1 = insert_baker(db, "Jane", bakery_1.id).await?;
    let baker_2 = insert_baker(db, "Peter", bakery_1.id).await?;
    let baker_3 = insert_baker(db, "Fred", bakery_1.id).await?; // does not make cake

    let cake_1 = insert_cake(db, "Cheesecake", None).await?;
    let cake_2 = insert_cake(db, "Coffee", None).await?;
    let cake_3 = insert_cake(db, "Chiffon", None).await?;
    let cake_4 = insert_cake(db, "Apple Pie", None).await?; // no one makes apple pie

    insert_cake_baker(db, baker_1.id, cake_1.id).await?;
    insert_cake_baker(db, baker_1.id, cake_2.id).await?;
    insert_cake_baker(db, baker_2.id, cake_2.id).await?;
    insert_cake_baker(db, baker_2.id, cake_3.id).await?;

    let bakers = baker::Entity::find().all(db).await?;

    // A bare entity: the rows arrive in whatever order the join produces, so
    // the assertion sorts each bucket rather than pretending otherwise.
    let mut cakes = bakers.load_many_via(cake::Entity, db).await?;
    for bucket in &mut cakes {
        bucket.sort_by_key(|cake| cake.id);
    }

    assert_eq!(bakers, [baker_1.clone(), baker_2.clone(), baker_3.clone()]);
    assert_eq!(
        cakes,
        [
            vec![cake_1.clone(), cake_2.clone()],
            vec![cake_2.clone(), cake_3.clone()],
            vec![]
        ]
    );

    // same, but apply restrictions on cakes

    let cakes = bakers
        .load_many_via(
            cake::Entity::find().filter(cake::Column::Name.like("Ch%")),
            db,
        )
        .await?;
    assert_eq!(cakes, [vec![cake_1.clone()], vec![cake_3.clone()], vec![]]);

    // now, start again from cakes — and order the target selector, which
    // orders every bucket.

    let cakes = cake::Entity::find().all(db).await?;
    let bakers = cakes
        .load_many_via(baker::Entity::find().order_by_desc(baker::Column::Id), db)
        .await?;

    assert_eq!(cakes, [cake_1, cake_2, cake_3, cake_4]);
    assert_eq!(
        bakers,
        [
            vec![baker_1.clone()],
            vec![baker_2.clone(), baker_1.clone()],
            vec![baker_2.clone()],
            vec![]
        ]
    );

    drop(conn);
    ctx.delete().await;

    Ok(())
}

pub async fn insert_bakery(db: &DatabaseConnection, name: &str) -> Result<bakery::Model, Error> {
    bakery::ActiveModel {
        name: set(name),
        profit_margin: set(1.0),
        ..Default::default()
    }
    .insert(db)
    .await
}

pub async fn insert_baker(
    db: &DatabaseConnection,
    name: &str,
    bakery_id: i32,
) -> Result<baker::Model, Error> {
    baker::ActiveModel {
        name: set(name),
        contact_details: set(serde_json::json!({})),
        bakery_id: set(Some(bakery_id)),
        ..Default::default()
    }
    .insert(db)
    .await
}

pub async fn insert_cake(
    db: &DatabaseConnection,
    name: &str,
    bakery_id: Option<i32>,
) -> Result<cake::Model, Error> {
    cake::ActiveModel {
        name: set(name),
        price: set(rust_decimal::Decimal::ONE),
        gluten_free: set(false),
        bakery_id: set(bakery_id),
        ..Default::default()
    }
    .insert(db)
    .await
}

pub async fn insert_cake_baker(
    db: &DatabaseConnection,
    baker_id: i32,
    cake_id: i32,
) -> Result<cakes_bakers::Model, Error> {
    cakes_bakers::ActiveModel {
        cake_id: set(cake_id),
        baker_id: set(baker_id),
    }
    .insert(db)
    .await
}

// A fixture purpose-built for the loader's key-mapping edge cases: it lives in
// an explicit schema (exercising the `TableName::SchemaTable` arm of the key
// predicate), and `owner_id` is deliberately not unique, so a relation declared
// `HasOne` over it can match several rows for one key.
mod ledger {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(schema_name = "public", table_name = "ledger")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub owner_id: i32,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// `HasOne` over a non-unique column, so one key can match several rows.
impl Related<ledger::Entity> for bakery::Entity {
    fn to() -> RelationDef {
        bakery::Entity::belongs_to(ledger::Entity)
            .columns(bakery::Column::Id, ledger::Column::OwnerId)
            .into()
    }
}

/// A junction-mediated relation whose target is `HasMany` — rejected by all
/// three loader entry points, each for its own reason.
impl Related<ledger::Entity> for cake::Entity {
    fn to() -> RelationDef {
        let mut def: RelationDef = cake::Entity::belongs_to(ledger::Entity)
            .columns(cake::Column::Id, ledger::Column::OwnerId)
            .into();
        def.rel_type = RelationType::HasMany;
        def
    }

    fn via() -> Option<RelationDef> {
        Some(cakes_bakers::Relation::Cake.def().rev())
    }
}

/// The same relation with an aliased target table, which the loader's key
/// predicate cannot qualify.
impl Related<ledger::Entity> for customer::Entity {
    fn to() -> RelationDef {
        let mut def: RelationDef = customer::Entity::belongs_to(ledger::Entity)
            .columns(customer::Column::Id, ledger::Column::OwnerId)
            .into();
        def.to_tbl = def.to_tbl.alias(alias("l"));
        def
    }
}

/// A relation naming a column the source model does not have, which the
/// loader's key extraction cannot resolve to a `Column` variant.
impl Related<ledger::Entity> for baker::Entity {
    fn to() -> RelationDef {
        let mut def: RelationDef = baker::Entity::belongs_to(ledger::Entity)
            .columns(baker::Column::Id, ledger::Column::OwnerId)
            .into();
        def.columns = ColumnPairs::new(alias("no_such_column"), ledger::Column::OwnerId);
        def
    }
}

// A second fixture whose key column is blank-padded `char(3)`: a key matches in
// SQL, because `bpchar` comparison ignores trailing blanks, but reads back
// padded, so an input model spelling that key unpadded is not equal to the row
// it was fetched for.
mod padded {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "padded")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(column_type = ColumnType::Char(Some(3)))]
        pub code: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// `HasMany` from an unpadded `text` column onto that padded one.
impl Related<padded::Entity> for bakery::Entity {
    fn to() -> RelationDef {
        let mut def: RelationDef = bakery::Entity::belongs_to(padded::Entity)
            .columns(bakery::Column::Name, padded::Column::Code)
            .into();
        def.rel_type = RelationType::HasMany;
        def
    }
}

/// The same pairing walked the other way, so the padded column is the one the
/// input models are keyed by.
impl Related<bakery::Entity> for padded::Entity {
    fn to() -> RelationDef {
        let mut def: RelationDef = padded::Entity::belongs_to(bakery::Entity)
            .columns(padded::Column::Code, bakery::Column::Name)
            .into();
        def.rel_type = RelationType::HasMany;
        def
    }
}

// A fixture whose relations carry a predicate of their own: `visible` is what
// an authored `on_condition` narrows on, so a load that dropped the relation's
// predicate would hand back the hidden row.
mod tagged {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "tagged")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub owner_id: i32,
        pub visible: bool,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The predicate an authored relation adds to its key equality. It qualifies
/// from the identifier it is handed, so it survives the loader's aliasing.
fn only_visible(_left: Name, right: Name) -> Condition {
    Expr::col((right, tagged::Column::Visible))
        .eq(true)
        .into_condition()
}

/// `HasMany` narrowed by the relation's own predicate.
impl Related<tagged::Entity> for bakery::Entity {
    fn to() -> RelationDef {
        let mut def: RelationDef = bakery::Entity::belongs_to(tagged::Entity)
            .columns(bakery::Column::Id, tagged::Column::OwnerId)
            .into();
        def.rel_type = RelationType::HasMany;
        def.on_condition(only_visible)
    }
}

/// A self-referencing `HasOne` — target and input are one table — resolving
/// each row's owner, and only while that owner is visible.
impl Related<tagged::Entity> for tagged::Entity {
    fn to() -> RelationDef {
        let def: RelationDef = tagged::Entity::belongs_to(tagged::Entity)
            .columns(tagged::Column::OwnerId, tagged::Column::Id)
            .into();
        def.on_condition(only_visible)
    }
}

/// The same predicate composed with `Any`, so a row satisfying it belongs to
/// every input rather than only to the one its key names.
impl Related<tagged::Entity> for customer::Entity {
    fn to() -> RelationDef {
        let mut def: RelationDef = customer::Entity::belongs_to(tagged::Entity)
            .columns(customer::Column::Id, tagged::Column::OwnerId)
            .condition_type(ConditionType::Any)
            .into();
        def.rel_type = RelationType::HasMany;
        def.on_condition(only_visible)
    }
}

/// A self-relation on a composite primary key: every row matches exactly
/// itself, so the batch predicate has to be a tuple `IN` list.
impl Related<cakes_bakers::Entity> for cakes_bakers::Entity {
    fn to() -> RelationDef {
        let mut def: RelationDef = cakes_bakers::Entity::belongs_to(cakes_bakers::Entity)
            .columns(cakes_bakers::Column::CakeId, cakes_bakers::Column::CakeId)
            .and_columns(cakes_bakers::Column::BakerId, cakes_bakers::Column::BakerId)
            .into();
        def.rel_type = RelationType::HasMany;
        def
    }
}

fn internal_err<T>(message: &str) -> Result<T, Error> {
    Err(Error::Query(RuntimeError::Internal(message.to_owned())))
}

fn internal_message<T: std::fmt::Debug>(outcome: Result<T, Error>, context: &str) -> String {
    match outcome.expect_err(context) {
        Error::Query(RuntimeError::Internal(message)) => message,
        other => panic!("{context}, got: {other:?}"),
    }
}

async fn create_ledger_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionTrait,
{
    let stmt = Schema::new().create_table_from_entity(ledger::Entity)?;
    create_table_without_asserts(db, &stmt).await
}

async fn create_padded_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionTrait,
{
    let stmt = Schema::new().create_table_from_entity(padded::Entity)?;
    create_table_without_asserts(db, &stmt).await
}

async fn create_tagged_table<C>(db: &C) -> Result<u64, Error>
where
    C: ConnectionTrait,
{
    let stmt = Schema::new().create_table_from_entity(tagged::Entity)?;
    create_table_without_asserts(db, &stmt).await
}

async fn insert_tagged(
    db: &DatabaseConnection,
    owner_id: i32,
    visible: bool,
) -> Result<tagged::Model, Error> {
    tagged::ActiveModel {
        owner_id: set(owner_id),
        visible: set(visible),
        ..Default::default()
    }
    .insert(db)
    .await
}

async fn insert_customer(db: &DatabaseConnection, name: &str) -> Result<customer::Model, Error> {
    customer::ActiveModel {
        name: set(name),
        ..Default::default()
    }
    .insert(db)
    .await
}

async fn insert_ledger(
    db: &DatabaseConnection,
    owner_id: i32,
    label: &str,
) -> Result<ledger::Model, Error> {
    ledger::ActiveModel {
        owner_id: set(owner_id),
        label: set(label),
        ..Default::default()
    }
    .insert(db)
    .await
}

// [spec:pgorm:req:query.loader+1/test]    relation shape is validated up front:
// `load_one` rejects a junction, `load_many` rejects a junction and a HasOne
// target, and `load_many_via` rejects a missing junction and a non-HasOne
// target. There is no longer a mismatched-junction case to reject: the
// junction is the relation's own and the caller cannot name a different one.
// The `&[M]` impl the `Vec<M>` one delegates to is public API in its own right.
#[pgorm_macros::test]
async fn loader_rejects_wrong_relation_shapes() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_relation_shapes").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;

    let bakery_1 = insert_bakery(db, "SeaSide Bakery").await?;
    let baker_1 = insert_baker(db, "Jane", bakery_1.id).await?;
    let cake_1 = insert_cake(db, "Cheesecake", None).await?;
    insert_cake_baker(db, baker_1.id, cake_1.id).await?;

    let bakers = baker::Entity::find().all(db).await?;
    let cakes = cake::Entity::find().all(db).await?;

    assert_eq!(
        cakes.load_one(ledger::Entity, db).await,
        internal_err("Relation is ManytoMany instead of HasOne")
    );
    assert_eq!(
        cakes.load_many(ledger::Entity, db).await,
        internal_err("Relation is ManyToMany instead of HasMany")
    );
    assert_eq!(
        bakers.load_many(bakery::Entity, db).await,
        internal_err("Relation is HasOne instead of HasMany")
    );
    assert_eq!(
        bakers.load_many_via(bakery::Entity, db).await,
        internal_err("Relation is not ManyToMany")
    );
    assert_eq!(
        cakes.load_many_via(ledger::Entity, db).await,
        internal_err("Relation to is not HasOne")
    );

    // The slice impl carries the actual implementation.
    let slice: &[baker::Model] = bakers.as_slice();
    assert_eq!(slice.load_one(bakery::Entity, db).await?, [Some(bakery_1)]);

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:query.loader+1/test]    an empty input short-circuits to an
// empty result without querying: every selector passed here would raise a
// database error if it were ever sent
#[pgorm_macros::test]
async fn loader_empty_input_skips_the_query() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_empty_input").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;

    let poisoned_bakery = || bakery::Entity::find().filter(Expr::raw("no_such_column IS NULL"));
    let poisoned_baker = || baker::Entity::find().filter(Expr::raw("no_such_column IS NULL"));
    let poisoned_cake = || cake::Entity::find().filter(Expr::raw("no_such_column IS NULL"));

    // The selectors really are unusable against this schema.
    assert!(poisoned_bakery().all(db).await.is_err());

    let no_bakers: Vec<baker::Model> = Vec::new();
    assert_eq!(
        no_bakers.load_one(poisoned_bakery(), db).await?,
        Vec::<Option<bakery::Model>>::new()
    );

    let no_bakeries: &[bakery::Model] = &[];
    assert_eq!(
        no_bakeries.load_many(poisoned_baker(), db).await?,
        Vec::<Vec<baker::Model>>::new()
    );

    let no_bakers_slice: &[baker::Model] = &[];
    assert_eq!(
        no_bakers_slice.load_many_via(poisoned_cake(), db).await?,
        Vec::<Vec<cake::Model>>::new()
    );

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:query.loader.batching+8/test]    keys are collected in input
// order and become a single IN predicate on the relation's from side, over a
// relation that is also self-referencing: a composite key renders as a tuple
// `IN` list through `in_tuples` (the unary `col IN (..)` form is what every
// other loader test here exercises). The predicate is AND-ed onto the caller's
// `Select`, so a user filter composes with it. The rule's note that duplicate
// keys are repeated rather than deduplicated concerns the emitted SQL text,
// which is not observable through this API.
// [spec:pgorm:sem:query.loader.regroup+6/test]    two inputs sharing a key each
// receive their own clone of that key's bucket
#[pgorm_macros::test]
async fn loader_batches_composite_keys_as_tuples() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_composite_keys").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;

    let bakery_1 = insert_bakery(db, "SeaSide Bakery").await?;
    let baker_1 = insert_baker(db, "Jane", bakery_1.id).await?;
    let baker_2 = insert_baker(db, "Peter", bakery_1.id).await?;
    let cake_1 = insert_cake(db, "Cheesecake", None).await?;
    let cake_2 = insert_cake(db, "Chiffon", None).await?;

    insert_cake_baker(db, baker_1.id, cake_1.id).await?;
    insert_cake_baker(db, baker_2.id, cake_1.id).await?;
    insert_cake_baker(db, baker_1.id, cake_2.id).await?;

    let rows = cakes_bakers::Entity::find()
        .order_by_asc(cakes_bakers::Column::CakeId)
        .order_by_asc(cakes_bakers::Column::BakerId)
        .all(db)
        .await?;
    assert_eq!(rows.len(), 3);

    // Each composite key matches exactly its own row, positionally.
    let matched = rows.load_many(cakes_bakers::Entity, db).await?;
    assert_eq!(
        matched,
        rows.iter().map(|row| vec![row.clone()]).collect::<Vec<_>>()
    );

    // A caller-supplied filter is AND-ed onto the same tuple predicate.
    let filtered = rows
        .load_many(
            cakes_bakers::Entity::find().filter(cakes_bakers::Column::CakeId.eq(cake_1.id)),
            db,
        )
        .await?;
    assert_eq!(
        filtered,
        rows.iter()
            .map(|row| if row.cake_id == cake_1.id {
                vec![row.clone()]
            } else {
                Vec::new()
            })
            .collect::<Vec<_>>()
    );

    // Repeating an input repeats its bucket, in input order.
    let repeated = vec![rows[2].clone(), rows[0].clone(), rows[0].clone()];
    assert_eq!(
        repeated.load_many(cakes_bakers::Entity, db).await?,
        [
            vec![rows[2].clone()],
            vec![rows[0].clone()],
            vec![rows[0].clone()],
        ]
    );

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:query.loader.regroup+6/test]    `load_one` yields the one row
// its key's bucket holds, `None` where nothing matched, and a clone each to
// inputs sharing a key — and when a relation declared `HasOne` matches two
// distinct rows for one key it aborts with an `Err` naming how many rows, the
// target table, the key and the key's column list, rather than discarding a row
// [spec:pgorm:req:query.loader.table-ref-limitation+3/test]    the supported
// `TableName::SchemaTable` target: it names the root and the load runs
#[pgorm_macros::test]
async fn loader_load_one_refuses_a_second_row() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_one_refuses_many").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;
    create_ledger_table(db).await?;

    let bakery_1 = insert_bakery(db, "SeaSide Bakery").await?;
    let bakery_2 = insert_bakery(db, "Offshore Bakery").await?;

    let first = insert_ledger(db, bakery_1.id, "first").await?;

    // One row per key is what the relation declares, and here what it gets.
    let bakeries = vec![bakery_1.clone(), bakery_2, bakery_1.clone()];
    assert_eq!(
        bakeries.load_one(ledger::Entity, db).await?,
        [Some(first.clone()), None, Some(first.clone())]
    );

    // A second row under that key is the missing UNIQUE the relation's `HasOne`
    // assumes away, so the load reports it instead of resolving it — the
    // caller's own ordering does not make the discard any less arbitrary.
    let second = insert_ledger(db, bakery_1.id, "second").await?;
    assert!(first.id < second.id);

    let message = internal_message(
        bakeries
            .load_one(ledger::Entity::find().order_by_asc(ledger::Column::Id), db)
            .await,
        "two rows under one HasOne key must abort the load",
    );
    assert!(
        message.starts_with(&format!(
            "load_one matched 2 distinct `ledger` rows against the key \
             ValueTuple([Int(Some({}))]) read from `id`, but the relation declares HasOne",
            bakery_1.id
        )),
        "{message}"
    );
    assert!(message.contains("UNIQUE constraint"), "{message}");

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:req:query.loader.table-ref-limitation+3/test]    a relation whose
// target resolves to any other from item — an aliased table here — cannot have
// its key column qualified, so the load returns an `Err` naming the column and
// the offending from item
#[pgorm_macros::test]
async fn loader_errors_on_aliased_from_item() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_table_ref_limit").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;
    create_ledger_table(db).await?;

    let customers = vec![
        customer::ActiveModel {
            name: set("Alice"),
            ..Default::default()
        }
        .insert(db)
        .await?,
    ];

    let message = internal_message(
        customers.load_one(ledger::Entity, db).await,
        "an aliased from item must abort the load",
    );
    assert!(
        message.starts_with("Loader cannot qualify key column `owner_id`"),
        "{message}"
    );
    assert!(message.contains("SchemaTable"), "{message}");
    assert!(message.contains("ledger"), "{message}");

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:query.loader.batching+8/test]    a relation naming a column
// its source model does not have is reported as an `Err` naming that column and
// the model's table, not a panic
#[pgorm_macros::test]
async fn loader_errors_on_unknown_relation_column() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_unknown_column").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;
    create_ledger_table(db).await?;

    let bakery_1 = insert_bakery(db, "SeaSide Bakery").await?;
    let bakers = vec![insert_baker(db, "Jane", bakery_1.id).await?];

    let message = internal_message(
        bakers.load_one(ledger::Entity, db).await,
        "an unresolvable relation column must abort the load",
    );
    assert_eq!(
        message,
        "Relation names column `no_such_column`, which is not a column of `baker`"
    );

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:query.loader.regroup+6/test]    the key is read back from the
// input entity's own row rather than re-derived from the target, so a target
// whose `char(3)` column pads the key back out regroups instead of aborting
#[pgorm_macros::test]
async fn loader_regroups_across_a_padded_target() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_padded_target").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;
    create_padded_table(db).await?;

    let bakery_1 = insert_bakery(db, "ab").await?;
    let stored = padded::ActiveModel {
        code: set("ab"),
        ..Default::default()
    }
    .insert(db)
    .await?;
    assert_eq!(stored.code, "ab ");

    let bakeries = vec![bakery_1];
    assert_eq!(
        bakeries.load_many(padded::Entity, db).await?,
        [vec![stored]]
    );

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:query.loader.regroup+6/test]    a returned row whose key is
// absent from the seeded buckets — here an input model spelling a `char(3)` key
// the way the caller wrote it rather than the way the column stores it — is an
// `Err` naming the unmatched key, a sample input key and the key's column list,
// rather than a panic
#[pgorm_macros::test]
async fn loader_errors_on_unmatched_returned_key() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_unmatched_key").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;
    create_padded_table(db).await?;

    insert_bakery(db, "ab").await?;
    let stored = padded::ActiveModel {
        code: set("ab"),
        ..Default::default()
    }
    .insert(db)
    .await?;

    // The row really is reachable through the key predicate; it is only the
    // regrouping that cannot match it back.
    let inputs = vec![padded::Model {
        id: stored.id,
        code: "ab".to_owned(),
    }];
    let message = internal_message(
        inputs.load_many(bakery::Entity, db).await,
        "an unpadded input key must abort the regrouping",
    );
    assert!(
        message.starts_with(
            "Loader cannot regroup a returned row: the key ValueTuple([String(Some(\"ab \"))]) \
             read back from `code` equals none of the keys read from the input models (an input \
             key reads as ValueTuple([String(Some(\"ab\"))]))."
        ),
        "{message}"
    );

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:query.loader.batching+8/test]    the relation reaches SQL
// whole: a `HasMany` narrowed by its own `on_condition` loads exactly what the
// equivalent graph read returns, the excluded row included in neither, and a
// caller filter still composes onto the same statement
#[pgorm_macros::test]
async fn loader_load_many_applies_relation_predicate() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_load_many_predicate").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;
    create_tagged_table(db).await?;

    let bakery_1 = insert_bakery(db, "SeaSide Bakery").await?;
    let bakery_2 = insert_bakery(db, "Offshore Bakery").await?;
    let shown_1 = insert_tagged(db, bakery_1.id, true).await?;
    let shown_2 = insert_tagged(db, bakery_1.id, true).await?;
    let hidden = insert_tagged(db, bakery_1.id, false).await?;

    let bakeries = vec![bakery_1.clone(), bakery_2.clone()];
    let loaded = bakeries.load_many(tagged::Entity, db).await?;
    let joined = bakery::Entity::graph()
        .related_maybe::<tagged::Entity>()
        .all_grouped(db)
        .await?;

    assert_eq!(loaded, [vec![shown_1.clone(), shown_2.clone()], vec![]]);
    assert_eq!(
        joined,
        [
            (bakery_1, vec![shown_1, shown_2.clone()]),
            (bakery_2, vec![])
        ]
    );
    assert!(!loaded.concat().contains(&hidden));

    let filtered = bakeries
        .load_many(
            tagged::Entity::find().filter(tagged::Column::Id.eq(shown_2.id)),
            db,
        )
        .await?;
    assert_eq!(filtered, [vec![shown_2], vec![]]);

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:query.loader.batching+8/test]    a self-referencing `HasOne`
// — the input entity joined back under the loader's alias against its own
// table — resolves to what the equivalent aliased graph read resolves to, the
// relation's predicate excluding the same rows in both
#[pgorm_macros::test]
async fn loader_load_one_applies_relation_predicate() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_load_one_predicate").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;
    create_tagged_table(db).await?;

    let visible_owner = insert_tagged(db, 0, true).await?;
    let hidden_owner = insert_tagged(db, 0, false).await?;
    insert_tagged(db, visible_owner.id, true).await?;
    insert_tagged(db, hidden_owner.id, true).await?;

    let rows = tagged::Entity::find()
        .order_by_asc(tagged::Column::Id)
        .all(db)
        .await?;
    let owners = rows.load_one(tagged::Entity, db).await?;
    let joined = tagged::Entity::graph()
        .join_maybe_as::<tagged::Entity>(
            <tagged::Entity as Related<tagged::Entity>>::to(),
            alias("owner"),
        )
        .order_by_asc(tagged::Column::Id)
        .all(db)
        .await?;

    assert_eq!(owners, [None, None, Some(visible_owner), None]);
    assert_eq!(
        owners,
        joined
            .into_iter()
            .map(|(_, owner)| owner)
            .collect::<Vec<_>>()
    );

    drop(conn);
    ctx.delete().await;

    Ok(())
}

// [spec:pgorm:sem:query.loader.regroup+6/test]    under an `Any` composition a
// target satisfying the relation's predicate belongs to every input, not only
// to the one its key names — which is why the bucket is keyed on the input row
// the read carries back rather than on the target's own columns
#[pgorm_macros::test]
async fn loader_any_composition_shares_a_target() -> Result<(), Error> {
    let ctx = TestContext::new("loader_test_any_composition").await;
    create_tables(&ctx.db).await?;
    let conn = ctx.db.get().await?;
    let db = &conn;
    create_tagged_table(db).await?;

    let customer_1 = insert_customer(db, "Alice").await?;
    let customer_2 = insert_customer(db, "Bob").await?;
    let shown = insert_tagged(db, customer_1.id, true).await?;
    let hidden = insert_tagged(db, customer_2.id, false).await?;

    let customers = vec![customer_1, customer_2];
    let mut loaded = customers.load_many(tagged::Entity, db).await?;
    for bucket in &mut loaded {
        bucket.sort_by_key(|row| row.id);
    }

    // The visible row matches both inputs; the hidden one only the input its
    // key names.
    assert_eq!(loaded, [vec![shown.clone()], vec![shown, hidden]]);

    drop(conn);
    ctx.delete().await;

    Ok(())
}
