//! What the generated `schema_name` attribute is *for*: a live database
//! holding two `item` tables, one in `tenant_a` and one in `public` — which is
//! on the default `search_path` — and the entity pgorm-codegen writes for
//! `CREATE TABLE tenant_a.item` reaching its own.
//!
//! The two halves of the claim: `pgorm-codegen`'s
//! `schema_qualifier_tests::compact_entity_carries_the_source_schema` pins that
//! codegen emits `#[pgorm(schema_name = "tenant_a", table_name = "item")]` for
//! a qualified source table, and this pins that an entity written that way
//! reads and writes `tenant_a.item` while an unqualified one — what codegen
//! used to emit for the same DDL — silently gets `public.item` instead.
#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, setup::*};
use pgorm::entity::prelude::*;
use pretty_assertions::assert_eq;

const NO_PARAMS: [&(dyn tokio_postgres::types::ToSql + Sync); 0] = [];

/// The entity pgorm-codegen generates from `CREATE TABLE tenant_a.item`.
mod qualified {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(schema_name = "tenant_a", table_name = "item")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The same table read as if its schema had been discarded — what a generator
/// that drops the qualifier produces, and what `search_path` then resolves.
mod unqualified {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "item")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("schema_qualified_tests").await;
    let db = ctx.db.get().await?;

    for statement in [
        "CREATE SCHEMA tenant_a",
        "CREATE TABLE tenant_a.item (id int PRIMARY KEY, label text NOT NULL)",
        "CREATE TABLE public.item (id int PRIMARY KEY, label text NOT NULL)",
        "INSERT INTO tenant_a.item (id, label) VALUES (1, 'tenant_a')",
        "INSERT INTO public.item (id, label) VALUES (1, 'public')",
    ] {
        db.execute_raw(statement, NO_PARAMS).await?;
    }

    let mut result = generated_entity_targets_its_own_schema(&db).await;
    if result.is_ok() {
        result = raw_builders_target_the_entitys_schema(&db).await;
    }

    drop(db);
    ctx.delete().await;
    result
}

// [spec:pgorm:sem:codegen.entity.transform+18/test]    preserving the source
// table's schema qualifier is what keeps generated CRUD off a same-named table
// on the `search_path`
// [spec:pgorm:def:codegen.entity.compact+8/test]
async fn generated_entity_targets_its_own_schema(db: &DatabaseConnection) -> Result<(), Error> {
    // `public` is on the default `search_path` and `tenant_a` is not, so an
    // unqualified name resolves to the other table entirely.
    assert_eq!(
        unqualified::Entity::find().one(db).await?,
        unqualified::Model {
            id: 1,
            label: "public".to_owned(),
        }
    );

    assert_eq!(
        qualified::Entity::find().one(db).await?,
        qualified::Model {
            id: 1,
            label: "tenant_a".to_owned(),
        }
    );

    assert_eq!(
        qualified::Entity::find_by_id(1).one(db).await?,
        qualified::Model {
            id: 1,
            label: "tenant_a".to_owned(),
        }
    );

    // Writes land in the same place reads come from.
    qualified::ActiveModel {
        id: set(2),
        label: set("also tenant_a"),
    }
    .insert(db)
    .await?;

    assert_eq!(qualified::Entity::find().all(db).await?.len(), 2);
    assert_eq!(unqualified::Entity::find().all(db).await?.len(), 1);

    Ok(())
}

/// `statement` built, with its values bound, as a caller runs a raw write.
async fn run<S>(db: &DatabaseConnection, statement: &S) -> Result<u64, Error>
where
    S: pgorm::pgorm_query::QueryStatementBuilder,
{
    let (sql, values) = statement.build();
    db.execute_raw(&sql, values.into_iter().map(pgorm::ValueHolder))
        .await
}

// [spec:pgorm:req:entity.traits.entity-name+5/test]    an entity handed straight to a raw
// builder's table position is qualified as `table_ref` qualifies it, so a select, a join and
// each write reach `tenant_a.item` and never the `public.item` the `search_path` holds
// [spec:pgorm:def:sql.types.table-ref+7/test]
async fn raw_builders_target_the_entitys_schema(db: &DatabaseConnection) -> Result<(), Error> {
    use pgorm::pgorm_query::{Expr, Name, Order, Query};
    use qualified::{Column, Entity};

    let insert = Query::insert()
        .into_table(Entity)
        .columns([Column::Id, Column::Label])
        .values_panic([3.into(), "raw".into()])
        .to_owned();
    assert_eq!(
        insert.to_string(),
        r#"INSERT INTO "tenant_a"."item" ("id", "label") VALUES (3, 'raw')"#
    );
    assert_eq!(run(db, &insert).await?, 1);

    let update = Query::update()
        .table(Entity)
        .value(Column::Label, "raw, updated")
        .and_where(Expr::col(Column::Id).eq(3))
        .to_owned();
    assert_eq!(
        update.to_string(),
        r#"UPDATE "tenant_a"."item" SET "label" = 'raw, updated' WHERE "id" = 3"#
    );
    assert_eq!(run(db, &update).await?, 1);

    let labels = Query::select()
        .column((Entity, Column::Label))
        .from(Entity)
        .order_by((Entity, Column::Id), Order::Asc)
        .to_owned();
    assert_eq!(
        labels.to_string(),
        r#"SELECT "item"."label" FROM "tenant_a"."item" ORDER BY "item"."id" ASC"#
    );
    assert_eq!(
        labels.into_tuple::<String>().all(db).await?,
        ["tenant_a", "also tenant_a", "raw, updated"]
    );

    // Both tables are named `item`, so the one read through `search_path`
    // takes an alias to be told apart from the joined one.
    let public = Name::runtime("p");
    let joined = Query::select()
        .column((public.clone(), Column::Label))
        .column((Entity, Column::Label))
        .from_as(unqualified::Entity, public.clone())
        .inner_join(
            Entity,
            Expr::col((Entity, Column::Id)).equals((public, Column::Id)),
        )
        .to_owned();
    assert_eq!(
        joined.to_string(),
        r#"SELECT "p"."label", "item"."label" FROM "item" AS "p" INNER JOIN "tenant_a"."item" ON "item"."id" = "p"."id""#
    );
    assert_eq!(
        joined.into_tuple::<(String, String)>().all(db).await?,
        [("public".to_owned(), "tenant_a".to_owned())]
    );

    let delete = Query::delete()
        .from_table(Entity)
        .and_where(Expr::col(Column::Id).eq(3))
        .to_owned();
    assert_eq!(
        delete.to_string(),
        r#"DELETE FROM "tenant_a"."item" WHERE "id" = 3"#
    );
    assert_eq!(run(db, &delete).await?, 1);
    assert_eq!(qualified::Entity::find().all(db).await?.len(), 2);
    assert_eq!(unqualified::Entity::find().all(db).await?.len(), 1);

    Ok(())
}
