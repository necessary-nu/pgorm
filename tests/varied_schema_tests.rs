//! First-class schema support, proven live: entities declaring
//! `schema_name` drive every read and write path against real non-`public`
//! schemas — the DDL that creates them, CRUD, finds, cursors, graph reads
//! within and across schemas, loaders, the pipeline, and a schema-qualified
//! enum — plus the same table name in two schemas queried side by side, so
//! nothing here can be riding `search_path`.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo test --test varied_schema_tests`

#![allow(unused_imports, dead_code)]

pub mod common;

use futures::TryStreamExt;
use pgorm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, Error, LoaderTrait,
    PaginatorTrait, QueryFilter, QueryOrder, RelationTrait, Schema, TryInsertResult,
    entity::prelude::*, entity::*, query::*,
};
use pretty_assertions::assert_eq;

pub use common::TestContext;

mod tenant_a {
    pub mod status {
        use pgorm::entity::prelude::*;

        #[derive(Debug, Clone, PartialEq, Eq, ActiveEnum, Copy)]
        #[pgorm(enum_name = "status", schema_name = "tenant_a")]
        pub enum Status {
            #[pgorm(string_value = "open")]
            Open,
            #[pgorm(string_value = "closed")]
            Closed,
        }
    }

    pub mod item {
        use pgorm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, Eq, Entity)]
        #[pgorm(schema_name = "tenant_a", table_name = "item")]
        pub struct Model {
            #[pgorm(primary_key, auto_increment = false)]
            pub id: i32,
            pub name: String,
            pub status: super::status::Status,
            pub owner_id: Option<i32>,
        }

        #[derive(Copy, Clone, Debug, Relation)]
        pub enum Relation {
            #[pgorm(
                belongs_to = crate::owner::Entity,
                from = Column::OwnerId,
                to = crate::owner::Column::Id
            )]
            Owner,
        }

        impl Related<super::tag::Entity> for Entity {
            fn to() -> RelationDef {
                super::item_tag::Relation::Tag.def()
            }

            fn via() -> Option<RelationDef> {
                Some(super::item_tag::Relation::Item.def().rev())
            }
        }
    }

    pub mod tag {
        use pgorm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, Eq, Entity)]
        #[pgorm(schema_name = "tenant_a", table_name = "tag")]
        pub struct Model {
            #[pgorm(primary_key, auto_increment = false)]
            pub id: i32,
            pub name: String,
        }

        #[derive(Copy, Clone, Debug, Relation)]
        pub enum Relation {}
    }

    pub mod item_tag {
        use pgorm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, Eq, Entity)]
        #[pgorm(schema_name = "tenant_a", table_name = "item_tag")]
        pub struct Model {
            #[pgorm(primary_key, auto_increment = false)]
            pub item_id: i32,
            #[pgorm(primary_key, auto_increment = false)]
            pub tag_id: i32,
        }

        #[derive(Copy, Clone, Debug, Relation)]
        pub enum Relation {
            #[pgorm(
                belongs_to = super::item::Entity,
                from = Column::ItemId,
                to = super::item::Column::Id
            )]
            Item,
            #[pgorm(
                belongs_to = super::tag::Entity,
                from = Column::TagId,
                to = super::tag::Column::Id
            )]
            Tag,
        }
    }
}

mod tenant_b {
    /// Deliberately the same table name as `tenant_a::item`: only the
    /// declared schema tells them apart.
    pub mod item {
        use pgorm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, Eq, Entity)]
        #[pgorm(schema_name = "tenant_b", table_name = "item")]
        pub struct Model {
            #[pgorm(primary_key, auto_increment = false)]
            pub id: i32,
            pub name: String,
        }

        #[derive(Copy, Clone, Debug, Relation)]
        pub enum Relation {}
    }
}

/// An unqualified entity, resolved through `search_path` as ever: the mixed
/// case, related across the schema boundary from both sides.
mod owner {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "owner")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(has_many = crate::tenant_a::item::Entity)]
        Item,
    }
}

use tenant_a::status::Status;

/// Both schemas, every table and the enum — created from the entities
/// themselves, so the generated DDL is what proves itself against a live
/// non-`public` schema.
async fn create_schemas(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute("CREATE SCHEMA tenant_a; CREATE SCHEMA tenant_b;")
        .await?;
    let schema = Schema::new();
    for stmt in schema.create_enum_from_entity(tenant_a::item::Entity) {
        db.execute(&stmt.to_string(), &[]).await?;
    }
    db.execute(
        &schema.create_table_from_entity(owner::Entity)?.to_string(),
        &[],
    )
    .await?;
    for table in [
        schema.create_table_from_entity(tenant_a::item::Entity)?,
        schema.create_table_from_entity(tenant_a::tag::Entity)?,
        schema.create_table_from_entity(tenant_a::item_tag::Entity)?,
        schema.create_table_from_entity(tenant_b::item::Entity)?,
    ] {
        db.execute(&table.to_string(), &[]).await?;
    }
    Ok(())
}

async fn seed(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(
        r#"
        INSERT INTO "owner" VALUES (1, 'Alice'), (2, 'Bo');
        INSERT INTO "tenant_a"."item" VALUES
            (10, 'anvil', 'open', 1),
            (11, 'bellows', 'open', 1),
            (12, 'crate', 'closed', 2),
            (13, 'drum', 'open', NULL);
        INSERT INTO "tenant_a"."tag" VALUES (100, 'heavy'), (101, 'wooden');
        INSERT INTO "tenant_a"."item_tag" VALUES (10, 100), (12, 100), (12, 101);
        INSERT INTO "tenant_b"."item" VALUES (10, 'impostor');
        "#,
    )
    .await
}

fn anvil() -> tenant_a::item::Model {
    tenant_a::item::Model {
        id: 10,
        name: "anvil".to_owned(),
        status: Status::Open,
        owner_id: Some(1),
    }
}

// [spec:pgorm:req:entity.traits.entity-name+6/test]    a declared schema
// qualifies the generated DDL and every CRUD statement: create, insert with
// RETURNING, a guarded no-op update, a TryInsert conflict, and delete all
// land on tenant_a.item
// [spec:pgorm:sem:exec.crud.update+7/test]
#[pgorm_macros::test]
async fn schema_ddl_and_crud_round_trip() -> Result<(), Error> {
    let ctx = TestContext::new("varied_schema_crud").await;
    let db = ctx.db.get().await?;
    create_schemas(&db).await?;

    let inserted = Insert::one(tenant_a::item::ActiveModel {
        id: set(10),
        name: set("anvil"),
        status: set(Status::Open),
        owner_id: set(None),
    })
    .exec_returning_model(&db)
    .await?;
    assert_eq!(inserted.name, "anvil");

    let conflicted = TryInsert::one(tenant_a::item::ActiveModel {
        id: set(10),
        name: set("anvil again"),
        status: set(Status::Open),
        owner_id: set(None),
    })
    .on_conflict(
        pgorm_query::OnConflict::column(tenant_a::item::Column::Id)
            .do_nothing()
            .to_owned(),
    )
    .exec(&db)
    .await?;
    assert!(matches!(conflicted, TryInsertResult::Conflicted));

    let updated = Update::one(tenant_a::item::ActiveModel {
        id: ActiveValue::Unchanged(10),
        name: set("re-forged anvil"),
        status: ActiveValue::Unchanged(Status::Open),
        owner_id: ActiveValue::Unchanged(None),
    })?
    .exec_returning_model(&db)
    .await?;
    assert_eq!(updated.name, "re-forged anvil");

    // The guarded no-op re-read runs under the full WHERE, schema included.
    let missed = Update::one(updated.clone().into_active())?
        .filter(tenant_a::item::Column::Name.eq("someone else's anvil"))
        .exec_returning_model(&db)
        .await;
    assert_eq!(missed.unwrap_err(), Error::RecordNotFound);

    let deleted = Delete::many(tenant_a::item::Entity)
        .filter(tenant_a::item::Column::Id.eq(10))
        .exec(&db)
        .await?;
    assert_eq!(deleted, 1);
    assert_eq!(tenant_a::item::Entity::find().count(&db).await?, 0);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:entity.traits.entity-name+6/test]    finds, filters,
// ordering, pagination, keyset cursors and the schema-qualified enum's value
// predicates all address tenant_a.item
// [spec:pgorm:sem:entity.traits.column.enum-cast+17/test]
// [spec:pgorm:sem:exec.cursor.keyset+8/test]
#[pgorm_macros::test]
async fn qualified_finds_filters_and_cursors() -> Result<(), Error> {
    let ctx = TestContext::new("varied_schema_finds").await;
    let db = ctx.db.get().await?;
    create_schemas(&db).await?;
    seed(&db).await?;

    assert_eq!(
        tenant_a::item::Entity::find()
            .filter(tenant_a::item::Column::Status.eq(Status::Open))
            .order_by_asc(tenant_a::item::Column::Id)
            .all(&db)
            .await?
            .len(),
        3
    );
    assert_eq!(
        tenant_a::item::Entity::find()
            .filter(tenant_a::item::Column::Status.is_in([Status::Closed]))
            .one_opt(&db)
            .await?
            .map(|item| item.id),
        Some(12)
    );

    let page = tenant_a::item::Entity::find()
        .order_by_asc(tenant_a::item::Column::Id)
        .paginate(&db, std::num::NonZeroU64::new(2).expect("non-zero"));
    assert_eq!(page.num_items().await?, 4);

    let mut cursor = tenant_a::item::Entity::find().cursor_by(tenant_a::item::Column::Id);
    let first = cursor.first(2).all(&db).await?;
    assert_eq!(
        first.iter().map(|item| item.id).collect::<Vec<_>>(),
        [10, 11]
    );
    let rest = cursor.after(11).all(&db).await?;
    assert_eq!(
        rest.iter().map(|item| item.id).collect::<Vec<_>>(),
        [12, 13]
    );

    // The enum-keyed cursor binds its boundary under the qualified cast;
    // enum comparison order is label-declaration order, so 'open' sorts
    // before 'closed' and resuming past the Open run reaches the Closed one.
    let mut by_status = tenant_a::item::Entity::find()
        .cursor_by(tenant_a::item::Column::Status)
        .into_model::<tenant_a::item::Model>();
    let open_first = by_status.first(1).all(&db).await?;
    assert_eq!(open_first[0].status, Status::Open);
    let closed = by_status.after(Status::Open).all(&db).await?;
    assert_eq!(
        closed.iter().map(|item| item.status).collect::<Vec<_>>(),
        [Status::Closed]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.slots+2/test]    graph reads span schemas: a
// junction fold inside tenant_a, an INNER join from tenant_a.item to the
// unqualified owner, grouped rows and a graph cursor — every source
// qualified by its own declaration
// [spec:pgorm:sem:query.graph.cursor+4/test]
#[pgorm_macros::test]
async fn graph_reads_span_schemas() -> Result<(), Error> {
    let ctx = TestContext::new("varied_schema_graph").await;
    let db = ctx.db.get().await?;
    create_schemas(&db).await?;
    seed(&db).await?;

    // Junction fold, wholly inside tenant_a.
    let tagged = tenant_a::item::Entity::graph()
        .related_maybe::<tenant_a::tag::Entity>()
        .filter(tenant_a::item::Column::Id.eq(12))
        .order_by_asc(tenant_a::tag::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(
        tagged
            .iter()
            .map(|(item, tag)| (item.id, tag.as_ref().map(|t| t.name.as_str())))
            .collect::<Vec<_>>(),
        [(12, Some("heavy")), (12, Some("wooden"))]
    );

    // INNER join across the schema boundary.
    let owned = tenant_a::item::Entity::graph()
        .join_one::<owner::Entity>(tenant_a::item::Relation::Owner.def())
        .order_by_asc(tenant_a::item::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(
        owned
            .iter()
            .map(|(item, owner)| (item.id, owner.name.as_str()))
            .collect::<Vec<_>>(),
        [(10, "Alice"), (11, "Alice"), (12, "Bo")]
    );

    // Grouped from the other side: the unqualified root gathers its
    // qualified children.
    let grouped = owner::Entity::graph()
        .related_maybe::<tenant_a::item::Entity>()
        .order_by_asc(owner::Column::Id)
        .all_grouped(&db)
        .await?;
    assert_eq!(
        grouped
            .iter()
            .map(|(owner, items)| (owner.name.as_str(), items.len()))
            .collect::<Vec<_>>(),
        [("Alice", 2), ("Bo", 1)]
    );

    // The graph cursor's keyset qualifies by each source's own identifier.
    let mut cursor = owner::Entity::graph()
        .related_maybe::<tenant_a::item::Entity>()
        .cursor_by(owner::Column::Id);
    let first = cursor.first(2).all(&db).await?;
    assert_eq!(first.len(), 2);
    let resumed = cursor.after_with((1, 11)).first(2).all(&db).await?;
    assert_eq!(
        resumed
            .iter()
            .map(|(owner, item)| (owner.id, item.as_ref().map(|i| i.id)))
            .collect::<Vec<_>>(),
        [(2, Some(12))]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.loader.batching+8/test]    loaders ride the graph
// across the schema boundary and through a qualified junction
#[pgorm_macros::test]
async fn loaders_work_across_qualified_schemas() -> Result<(), Error> {
    let ctx = TestContext::new("varied_schema_loaders").await;
    let db = ctx.db.get().await?;
    create_schemas(&db).await?;
    seed(&db).await?;

    let owners = owner::Entity::find()
        .order_by_asc(owner::Column::Id)
        .all(&db)
        .await?;
    let items = owners.load_many(tenant_a::item::Entity, &db).await?;
    assert_eq!(
        items
            .iter()
            .map(|per_owner| per_owner.len())
            .collect::<Vec<_>>(),
        [2, 1]
    );

    let some_items = tenant_a::item::Entity::find()
        .filter(tenant_a::item::Column::Id.is_in([10, 12]))
        .order_by_asc(tenant_a::item::Column::Id)
        .all(&db)
        .await?;
    let tags = some_items.load_many_via(tenant_a::tag::Entity, &db).await?;
    assert_eq!(
        tags.iter()
            .map(|per_item| per_item.len())
            .collect::<Vec<_>>(),
        [1, 2]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:entity.traits.entity-name+6/test]    the same table name in
// two schemas: both entities read side by side and return their own rows, so
// resolution is by declared schema, never by search_path
#[pgorm_macros::test]
async fn same_table_name_in_two_schemas() -> Result<(), Error> {
    let ctx = TestContext::new("varied_schema_same_name").await;
    let db = ctx.db.get().await?;
    create_schemas(&db).await?;
    seed(&db).await?;

    let a = tenant_a::item::Entity::find_by_id(10).one(&db).await?;
    let b = tenant_b::item::Entity::find_by_id(10).one(&db).await?;
    assert_eq!(a.name, "anvil");
    assert_eq!(b.name, "impostor");

    assert_eq!(tenant_a::item::Entity::find().count(&db).await?, 4);
    assert_eq!(tenant_b::item::Entity::find().count(&db).await?, 1);

    assert_eq!(tenant_a::item::Entity::graph().all(&db).await?.len(), 4);
    assert_eq!(tenant_b::item::Entity::graph().all(&db).await?.len(), 1);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:pipeline.select-sources+6/test]    the pipeline reads
// qualified sources: a cross-schema join lands whole models per source, and
// streaming decodes the qualified projection row by row
// [spec:pgorm:sem:exec.stream.decode+1/test]
#[pgorm_macros::test]
async fn pipeline_and_stream_read_qualified_sources() -> Result<(), Error> {
    use pgorm::pipeline::{JoinSide, Pipeline};

    let ctx = TestContext::new("varied_schema_pipeline").await;
    let db = ctx.db.get().await?;
    create_schemas(&db).await?;
    seed(&db).await?;

    let rows = Pipeline::from(tenant_a::item::Entity)
        .join(
            JoinSide::Inner,
            owner::Entity,
            pgorm::pipeline::ExprOps::eq(tenant_a::item::Column::OwnerId, owner::Column::Id),
        )
        .sort(tenant_a::item::Column::Id)
        .select_sources((tenant_a::item::Entity, owner::Entity))
        .all(&db)
        .await?;
    assert_eq!(rows.len(), 3);
    let (item, owner) = (&rows[0].0, &rows[0].1);
    assert_eq!(item.as_ref().map(|i| i.id), Some(10));
    assert_eq!(owner.as_ref().map(|o| o.name.as_str()), Some("Alice"));

    let streamed: Vec<tenant_a::item::Model> = tenant_a::item::Entity::find()
        .order_by_asc(tenant_a::item::Column::Id)
        .stream(&db)
        .await?
        .try_collect()
        .await?;
    assert_eq!(streamed.len(), 4);
    assert_eq!(streamed[0], anvil());

    drop(db);
    ctx.delete().await;
    Ok(())
}
