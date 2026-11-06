//! An entity whose schema is computed at run time, a tenant's, chosen per
//! statement: each builder's `_in` constructor reads and writes the tenant of
//! the instance it is given, while the `Default` form reads the default
//! tenant's table.
#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, setup::*};
use pgorm::entity::prelude::*;
use pgorm::pipeline::{ExprOps, Pipeline};
use pretty_assertions::assert_eq;

mod tenant_cake {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Debug, EntityTrait)]
    pub struct Entity {
        pub tenant: &'static str,
    }

    impl Default for Entity {
        fn default() -> Self {
            Entity { tenant: "public" }
        }
    }

    impl EntityName for Entity {
        fn schema_name(&self) -> Option<&str> {
            Some(self.tenant)
        }

        fn table_name(&self) -> &str {
            "cake"
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}
use tenant_cake::{ActiveModel, Column, Entity, Model};

fn cake(id: i32, name: &str) -> Model {
    Model {
        id,
        name: name.to_owned(),
    }
}

// [spec:pgorm:req:entity.traits.crud+5/test]    the reads and the key delete
// take the instance whose schema they read
// [spec:pgorm:sem:query.build.insert+6/test]
// [spec:pgorm:sem:query.build.update+5/test]
// [spec:pgorm:sem:query.build.delete+4/test]
// [spec:pgorm:def:query.graph+2/test]    a graph rooted at an instance
// [spec:pgorm:sem:pipeline.qualify+4/test]    a pipeline over an instance, its
// model terminal and its source selection
// [spec:pgorm:req:entity.active-model.persistence+3/test]    the active model's `_in`
// writes
#[pgorm_macros::test]
async fn statements_take_the_tenant_from_the_instance() -> Result<(), Error> {
    let ctx = TestContext::new("entity_instance_tenants").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        "CREATE SCHEMA acme;
         CREATE SCHEMA globex;
         CREATE TABLE public.cake (id serial PRIMARY KEY, name text NOT NULL);
         CREATE TABLE acme.cake (id serial PRIMARY KEY, name text NOT NULL);
         CREATE TABLE globex.cake (id serial PRIMARY KEY, name text NOT NULL);
         INSERT INTO public.cake (name) VALUES ('Default Sponge');",
    )
    .await?;
    let acme = Entity { tenant: "acme" };
    let globex = Entity { tenant: "globex" };

    Insert::many_in(
        acme,
        [
            ActiveModel {
                name: set("Acme Tart"),
                ..Default::default()
            },
            ActiveModel {
                name: set("Acme Loaf"),
                ..Default::default()
            },
        ],
    )
    .exec(&db)
    .await?;
    let gateau = Insert::one_in(
        globex,
        ActiveModel {
            name: set("Globex Gateau"),
            ..Default::default()
        },
    )
    .exec_returning_model(&db)
    .await?;
    assert_eq!(gateau, cake(1, "Globex Gateau"));

    assert_eq!(
        Entity::find_in(acme)
            .order_by_asc(Column::Id)
            .all(&db)
            .await?,
        [cake(1, "Acme Tart"), cake(2, "Acme Loaf")]
    );
    assert_eq!(
        Entity::find().all(&db).await?,
        [cake(1, "Default Sponge")],
        "the static form reads the default's tenant"
    );
    assert_eq!(
        Entity::find_by_id_in(globex, 1).one(&db).await?,
        cake(1, "Globex Gateau")
    );
    assert_eq!(
        Entity::graph_in(globex).all(&db).await?,
        [cake(1, "Globex Gateau")]
    );

    let renamed = Update::one_in(
        acme,
        ActiveModel {
            id: set(2),
            name: set("Acme Bloomer"),
        },
    )?
    .exec_returning_model(&db)
    .await?;
    assert_eq!(renamed, cake(2, "Acme Bloomer"));
    assert_eq!(
        Update::many(acme)
            .col_expr(Column::Name, Expr::value("Acme Bun"))
            .filter(ColumnTrait::eq(&Column::Id, 1))
            .exec(&db)
            .await?,
        1
    );

    let piped: Vec<Model> = Pipeline::from(acme)
        .filter(Column::Id.gt(1))
        .all(&db)
        .await?;
    assert_eq!(piped, [cake(2, "Acme Bloomer")]);
    let selected = Pipeline::from(acme)
        .sort(Column::Id)
        .select_sources(acme)
        .all(&db)
        .await?;
    assert_eq!(
        selected,
        [Some(cake(1, "Acme Bun")), Some(cake(2, "Acme Bloomer"))]
    );

    assert_eq!(
        Delete::one_in(acme, cake(1, "Acme Bun"))?.exec(&db).await?,
        1
    );
    assert_eq!(Entity::delete_by_id_in(acme, 2).exec(&db).await?, 1);
    assert_eq!(
        Delete::many(globex).exec(&db).await?,
        1,
        "the many-row delete already took its instance"
    );
    assert!(Entity::find_in(acme).all(&db).await?.is_empty());
    assert!(Entity::find_in(globex).all(&db).await?.is_empty());
    assert_eq!(Entity::find().all(&db).await?.len(), 1);

    // The active model's own writes, hooks and all, take the instance too.
    let scone = ActiveModel {
        name: set("Globex Scone"),
        ..Default::default()
    }
    .insert_in(globex, &db)
    .await?;
    let mut renamed = scone.clone().into_active();
    renamed.name = set("Globex Bap");
    assert_eq!(renamed.update_in(globex, &db).await?.name, "Globex Bap");
    assert_eq!(scone.into_active().delete_in(globex, &db).await?, 1);
    assert!(Entity::find_in(globex).all(&db).await?.is_empty());

    drop(db);
    ctx.delete().await;
    Ok(())
}
