#![allow(unused_imports, dead_code)]

//! An entity with PostgreSQL 18's temporal key, and a relation matching on a
//! `PERIOD`, against a live server.
//!
//! A room's rate changes over time, each rate a version of the room keyed by
//! the room and the period it holds over: `PRIMARY KEY (id, valid_at WITHOUT
//! OVERLAPS)`. A booking references the room over its own stay, `FOREIGN KEY
//! (room_id, PERIOD during) REFERENCES room (id, PERIOD valid_at)`, which the
//! server holds covered by the room's versions together. What the ORM makes of
//! the two is what this file holds: the key is looked up, updated and deleted
//! by equality, period included, and the relation joins a booking to every
//! version of its room whose period overlaps its stay.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{Name, extension::Extension};
use pgorm::{ConnectionTrait, LoaderTrait, Schema, entity::prelude::*};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

mod room {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "room")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(primary_key, without_overlaps)]
        pub valid_at: Range<Date>,
        pub rate: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {
        #[pgorm(has_many = "super::booking::Entity")]
        Booking,
    }

    impl Related<super::booking::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Booking.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod booking {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "booking")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub room_id: i32,
        pub during: Range<Date>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = "super::room::Entity",
            from = "Column::RoomId",
            to = "super::room::Column::Id",
            from_period = "Column::During",
            to_period = "super::room::Column::ValidAt"
        )]
        Room,
    }

    impl Related<super::room::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Room.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

fn date(month: i8, day: i8) -> Date {
    jiff::civil::date(2026, month, day)
}

/// `[from, to)` in 2026.
fn days(from: (i8, i8), to: (i8, i8)) -> Range<Date> {
    Range::from(date(from.0, from.1)..date(to.0, to.1))
}

fn version(id: i32, valid_at: Range<Date>, rate: i32) -> room::ActiveModel {
    room::ActiveModel {
        id: set(id),
        valid_at: set(valid_at),
        rate: set(rate),
    }
}

/// `room` and `booking` as their entities generate them, under `btree_gist`,
/// which the room key's scalar column needs, with room 1 at rate 100 for
/// January and 120 for February, and room 2 at 80 for January.
async fn rooms(db: &DatabaseConnection) -> Result<Vec<room::Model>, Error> {
    db.batch_execute(&Extension::create(Name::runtime("btree_gist")).to_string())
        .await?;
    for create in [
        Schema::new().create_table_from_entity(room::Entity)?,
        Schema::new().create_table_from_entity(booking::Entity)?,
    ] {
        db.batch_execute(&create.to_string()).await?;
    }
    Insert::many([
        version(1, days((1, 1), (2, 1)), 100),
        version(1, days((2, 1), (3, 1)), 120),
        version(2, days((1, 1), (2, 1)), 80),
    ])
    .exec_returning_models(db)
    .await
}

/// The entity's key generates `PRIMARY KEY (id, valid_at WITHOUT OVERLAPS)`,
/// which refuses a second version overlapping the first (`23P01`) and admits
/// one that only touches it. A lookup by key is by equality, period included:
/// it names one version, and a period that only overlaps a version names none.
/// An update and a delete by key reach that version alone.
// [spec:pgorm:def:entity.traits.primary-key+7/test]    against a live server: a temporal key is
// generated WITHOUT OVERLAPS, and looked up, updated and deleted by equality, period included
// [spec:pgorm:sem:schema.from-entity+10/test]
#[pgorm_macros::test]
async fn a_temporal_key_names_one_version() -> Result<(), Error> {
    let ctx = TestContext::new("temporal_entity_key").await;
    let db = ctx.db.get().await?;
    let create = Schema::new()
        .create_table_from_entity(room::Entity)?
        .to_string();
    assert!(
        create.contains(r#"PRIMARY KEY ("id", "valid_at" WITHOUT OVERLAPS)"#),
        "{create}"
    );
    let versions = rooms(&db).await?;

    let refused = version(1, days((1, 20), (2, 10)), 110)
        .insert(&db)
        .await
        .expect_err("a version overlapping two");
    refused_with(&refused, &SqlState::EXCLUSION_VIOLATION);
    version(1, days((3, 1), (4, 1)), 130).insert(&db).await?;

    assert_eq!(
        room::Entity::find_by_id((1, days((2, 1), (3, 1))))
            .one(&db)
            .await?,
        versions[1]
    );
    assert_eq!(
        room::Entity::find_by_id((1, days((2, 1), (2, 15))))
            .one_opt(&db)
            .await?,
        None
    );

    let mut february = versions[1].clone().into_active();
    february.rate = set(125);
    february.update(&db).await?;
    assert_eq!(
        room::Entity::find()
            .filter(room::Column::Id.eq(1))
            .order_by_asc(room::Column::ValidAt)
            .all(&db)
            .await?
            .into_iter()
            .map(|room| room.rate)
            .collect::<Vec<_>>(),
        [100, 125, 130]
    );
    assert_eq!(
        room::Entity::delete_by_id((1, days((3, 1), (4, 1))))
            .exec(&db)
            .await?,
        1
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The relation generates `FOREIGN KEY (room_id, PERIOD during) REFERENCES
/// room (id, PERIOD valid_at)`: a stay inside one version or across two
/// adjacent ones is admitted, one running past them refused (`23503`). It
/// joins a booking to every version of its room whose period overlaps the
/// stay: `find_related` and a graph slot reach each, `load_one` reads the one
/// version a stay falls in and reports a stay across two as more than one
/// related row, and the room's versions' `load_many` gives each version the
/// bookings it covers. Bookings of one room in different periods keep their
/// own versions, the period being part of the key a loader files them under.
// [spec:pgorm:def:entity.relation.def+12/test]    against a live server: a PERIOD relation
// creates the temporal foreign key and joins a row to each version its period overlaps
// [spec:pgorm:req:entity.relation.fk+5/test]
// [spec:pgorm:sem:query.loader.regroup+6/test]
#[pgorm_macros::test]
async fn a_period_relation_joins_overlapping_versions() -> Result<(), Error> {
    let ctx = TestContext::new("temporal_entity_relation").await;
    let db = ctx.db.get().await?;
    let create = Schema::new()
        .create_table_from_entity(booking::Entity)?
        .to_string();
    assert!(
        create.contains(
            r#"FOREIGN KEY ("room_id", PERIOD "during") REFERENCES "room" ("id", PERIOD "valid_at")"#
        ),
        "{create}"
    );
    let versions = rooms(&db).await?;

    let booking = |id: i32, room_id: i32, during: Range<Date>| booking::ActiveModel {
        id: set(id),
        room_id: set(room_id),
        during: set(during),
    };
    let refused = booking(9, 1, days((2, 20), (3, 10)))
        .insert(&db)
        .await
        .expect_err("a stay running past the room's versions");
    refused_with(&refused, &SqlState::FOREIGN_KEY_VIOLATION);
    let bookings = Insert::many([
        booking(1, 1, days((1, 5), (1, 9))),
        booking(2, 1, days((2, 5), (2, 9))),
        booking(3, 1, days((1, 28), (2, 3))),
        booking(4, 2, days((1, 10), (1, 12))),
    ])
    .exec_returning_models(&db)
    .await?;

    assert_eq!(
        bookings[2]
            .find_related(room::Entity)
            .order_by_asc(room::Column::ValidAt)
            .all(&db)
            .await?,
        versions[..2]
    );
    assert_eq!(
        booking::Entity::graph()
            .join_one::<room::Entity>(booking::Relation::Room.def())
            .order_by_asc(booking::Column::Id)
            .order_by_asc(room::Column::ValidAt)
            .all(&db)
            .await?,
        [
            (bookings[0].clone(), versions[0].clone()),
            (bookings[1].clone(), versions[1].clone()),
            (bookings[2].clone(), versions[0].clone()),
            (bookings[2].clone(), versions[1].clone()),
            (bookings[3].clone(), versions[2].clone()),
        ]
    );

    let within = vec![
        bookings[0].clone(),
        bookings[1].clone(),
        bookings[3].clone(),
    ];
    assert_eq!(
        within.load_one(room::Entity, &db).await?,
        [
            Some(versions[0].clone()),
            Some(versions[1].clone()),
            Some(versions[2].clone())
        ]
    );
    let across = vec![bookings[2].clone()].load_one(room::Entity, &db).await;
    assert!(matches!(across, Err(Error::Query(_))), "{across:?}");

    assert_eq!(
        versions.load_many(booking::Entity, &db).await?,
        [
            vec![bookings[0].clone(), bookings[2].clone()],
            vec![bookings[1].clone(), bookings[2].clone()],
            vec![bookings[3].clone()],
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
