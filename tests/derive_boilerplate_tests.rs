#![allow(unused_imports, dead_code)]

//! Entities written with nothing but their derives, against a live server: the
//! `Related` impl the relation derive writes is the join `find_related` walks,
//! a model saying `hooks` has its own behaviour run around a write, and an
//! enum of names is the PostgreSQL enum a schema created in snake_case.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo nextest run --test derive_boilerplate_tests`

pub mod common;
pub use common::TestContext;

use pgorm::{ConnectionTrait, Error, entity::prelude::*};
use pretty_assertions::assert_eq;

/// The schema the entities read, written by hand so the derives' defaults
/// are what has to match it: the type and its values in snake_case.
const SCHEMA: &str = r#"
    CREATE TYPE brew_kind AS ENUM ('everyday_tea', 'breakfast_tea', 'Earl Grey');
    CREATE TABLE teapot (id serial PRIMARY KEY, name text NOT NULL);
    CREATE TABLE cup (
        id serial PRIMARY KEY,
        teapot_id integer NOT NULL REFERENCES teapot (id),
        brew brew_kind NOT NULL,
        label text NOT NULL
    );
"#;

mod brew {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]
    pub enum BrewKind {
        EverydayTea,
        BreakfastTea,
        #[pgorm(string_value = "Earl Grey")]
        EarlGrey,
    }
}

use brew::BrewKind;

mod teapot {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "teapot")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(has_many = super::cup::Entity)]
        Cup,
    }
}

/// A cup is labelled on its first save unless it was given a label.
mod cup {
    use super::BrewKind;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "cup", hooks)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub teapot_id: i32,
        pub brew: BrewKind,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::teapot::Entity,
            from = Column::TeapotId,
            to = super::teapot::Column::Id
        )]
        Teapot,
    }

    #[async_trait::async_trait]
    impl ActiveModelBehavior for ActiveModel {
        async fn before_save<C>(mut self, _db: &C, insert: bool) -> Result<Self, Error>
        where
            C: ConnectionTrait,
        {
            if insert && self.label.is_not_set() {
                self.label = set("unlabelled");
            }
            Ok(self)
        }
    }
}

// [spec:pgorm:sem:macros.derive.boilerplate/test]    entities written with nothing but the derives: `find_related` walks the `Related` impls the relation derive writes, a `hooks` model's own behaviour runs on insert, and an enum of names reads and writes the snake_case `CREATE TYPE`
#[pgorm_macros::test]
async fn entities_of_derives_alone_work_live() -> Result<(), Error> {
    let ctx = TestContext::new("derive_boilerplate").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SCHEMA).await?;

    let pot = teapot::ActiveModel {
        name: set("brown betty"),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    let other = teapot::ActiveModel {
        name: set("tetsubin"),
        ..Default::default()
    }
    .insert(&db)
    .await?;

    let unlabelled = cup::ActiveModel {
        teapot_id: set(pot.id),
        brew: set(BrewKind::EverydayTea),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    assert_eq!(unlabelled.label, "unlabelled");
    let labelled = cup::ActiveModel {
        teapot_id: set(pot.id),
        brew: set(BrewKind::EarlGrey),
        label: set("best"),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    assert_eq!(labelled.label, "best");
    cup::ActiveModel {
        teapot_id: set(other.id),
        brew: set(BrewKind::BreakfastTea),
        ..Default::default()
    }
    .insert(&db)
    .await?;

    let cups = pot
        .find_related(cup::Entity)
        .order_by_asc(cup::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(cups, [unlabelled.clone(), labelled.clone()]);
    assert_eq!(
        cups.iter().map(|cup| cup.brew.clone()).collect::<Vec<_>>(),
        [BrewKind::EverydayTea, BrewKind::EarlGrey]
    );

    let owner = labelled.find_related(teapot::Entity).one(&db).await?;
    assert_eq!(owner, pot);

    let stored: Vec<String> = db
        .query_all("SELECT brew::text AS brew FROM cup ORDER BY id", &[])
        .await?
        .iter()
        .map(|row| row.get("brew"))
        .collect();
    assert_eq!(stored, ["everyday_tea", "Earl Grey", "breakfast_tea"]);

    let breakfast = cup::Entity::find()
        .filter(cup::Column::Brew.eq(BrewKind::BreakfastTea))
        .all(&db)
        .await?;
    assert_eq!(breakfast.len(), 1);
    assert_eq!(breakfast[0].teapot_id, other.id);

    ctx.delete().await;
    Ok(())
}
