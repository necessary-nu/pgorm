//! The bakery the pgorm user guide's examples share, and the scratch
//! databases they run in.
//!
//! Every guide example that talks to PostgreSQL opens a [`Scratch`] in its
//! hidden lines: a database of its own on the server `DATABASE_URL` names,
//! minted by `pgorm-test-support` so that any number of runs share the server,
//! and dropped again when the example finishes. [`Scratch::bakery`] creates
//! the bakery's tables from the entities below and fills them with the rows
//! the guide's text describes; [`Scratch::empty`] leaves the database empty
//! for an example that builds its own schema.
//!
//! The entities are the ones the guide's entities chapter declares, and the
//! rows are these, their ids assigned by the database in this order:
//!
//! | table | rows |
//! | --- | --- |
//! | `bakery` | 1 Sunrise (Oslo), 2 Moonlight (Bergen) |
//! | `baker` | 1 Ada and 2 Bo at Sunrise, 3 Cy at Moonlight |
//! | `cake` | 1 Chocolate Fudge, 2 Lemon Drizzle, 5 Chocolate Torte at Sunrise; 3 Carrot Cake, 4 Vanilla Sponge at Moonlight |
//! | `cake_baker` | Ada bakes 1 and 2, Bo bakes 2 and 5, Cy bakes 3 and 4 |
//! | `customer` | 1 Kim, 2 Lee |
//! | `order` | 1 Kim at Sunrise, 2 Lee at Moonlight |
//! | `order_line` | order 1: two Chocolate Fudge, one Lemon Drizzle; order 2: one Carrot Cake |
// [spec:pgorm:req:guide.examples]    the database every runnable example runs against

use pgorm::entity::prelude::*;
use pgorm::{Config, Schema};

pub mod baker;
pub mod bakery;
pub mod cake;
pub mod cake_baker;
pub mod customer;
pub mod order;
pub mod order_line;

/// A database of one example's own, dropped by [`Scratch::finish`].
///
/// An example that panics before it finishes leaves its database behind; the
/// first database a later run creates sweeps it, since its creator is gone.
#[derive(Debug)]
pub struct Scratch {
    /// A pool over the scratch database, closed by [`Scratch::finish`].
    pub pool: DatabasePool,
    base_url: String,
    name: String,
}

impl Scratch {
    /// A database holding the bakery's tables and rows.
    ///
    /// # Panics
    ///
    /// When `DATABASE_URL` is unset or the server refuses the schema: an
    /// example cannot run without either.
    pub async fn bakery(label: &str) -> Self {
        let scratch = Self::empty(label).await;
        let db = scratch
            .pool
            .get()
            .await
            .expect("the scratch database accepts a connection");
        create_schema(&db)
            .await
            .expect("the bakery's tables are created");
        seed(&db).await.expect("the bakery's rows are inserted");
        drop(db);
        scratch
    }

    /// An empty database.
    ///
    /// # Panics
    ///
    /// When `DATABASE_URL` is unset or the database cannot be created.
    pub async fn empty(label: &str) -> Self {
        dotenvy::from_filename(".env.local").ok();
        dotenvy::from_filename(".env").ok();
        let base_url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL names the PostgreSQL server the guide's examples run against");
        let name = pgorm_test_support::scoped_name(&format!("guide_{label}"));
        pgorm_test_support::create_database(&base_url, &name)
            .await
            .expect("the scratch database is created");
        let pool = pgorm::connect(pgorm_test_support::config(&base_url, &name));
        Self {
            pool,
            base_url,
            name,
        }
    }

    /// The scratch database's connection URL.
    pub fn url(&self) -> String {
        format!("{}/{}", self.base_url, self.name)
    }

    /// The scratch database's connection configuration.
    pub fn config(&self) -> Config {
        pgorm_test_support::config(&self.base_url, &self.name)
    }

    /// The scratch database's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Close the pool and drop the database.
    pub async fn finish(self) {
        let Self {
            pool,
            base_url,
            name,
        } = self;
        pool.close();
        drop(pool);
        // A database that outlives the drop is the next run's sweep's to
        // reclaim, once this process has exited.
        let _ = pgorm_test_support::drop_database(&base_url, &name).await;
    }
}

/// Create the bakery's enum type and tables, each from its entity.
pub async fn create_schema<C: ConnectionTrait>(db: &C) -> Result<(), Error> {
    let schema = Schema::new();
    for create in schema.create_enum_from_entity(cake::Entity) {
        db.batch_execute(&create.to_string()).await?;
    }
    let tables = [
        schema.create_table_from_entity(bakery::Entity)?,
        schema.create_table_from_entity(baker::Entity)?,
        schema.create_table_from_entity(cake::Entity)?,
        schema.create_table_from_entity(cake_baker::Entity)?,
        schema.create_table_from_entity(customer::Entity)?,
        schema.create_table_from_entity(order::Entity)?,
        schema.create_table_from_entity(order_line::Entity)?,
    ];
    for create in tables {
        db.batch_execute(&create.to_string()).await?;
    }
    Ok(())
}

/// Insert the bakery's rows.
pub async fn seed<C: ConnectionTrait>(db: &C) -> Result<(), Error> {
    use cake::Flavor;

    Insert::many([
        bakery::ActiveModel {
            name: set("Sunrise"),
            city: set("Oslo"),
            ..Default::default()
        },
        bakery::ActiveModel {
            name: set("Moonlight"),
            city: set("Bergen"),
            ..Default::default()
        },
    ])
    .exec(db)
    .await?;

    let baker = |name: &str, hired_on: Date, bakery_id: i32| baker::ActiveModel {
        name: set(name),
        hired_on: set(hired_on),
        bakery_id: set(bakery_id),
        ..Default::default()
    };
    Insert::many([
        baker("Ada", Date::constant(2019, 3, 1), 1),
        baker("Bo", Date::constant(2022, 9, 15), 1),
        baker("Cy", Date::constant(2021, 5, 10), 2),
    ])
    .exec(db)
    .await?;

    let cake = |name: &str, flavor: Flavor, cents: i64, gluten_free: bool, bakery_id: i32| {
        cake::ActiveModel {
            name: set(name),
            flavor: set(flavor),
            price: set(Decimal::new(cents, 2)),
            gluten_free: set(gluten_free),
            bakery_id: set(bakery_id),
            ..Default::default()
        }
    };
    Insert::many([
        cake("Chocolate Fudge", Flavor::Chocolate, 450, false, 1),
        cake("Lemon Drizzle", Flavor::Lemon, 320, true, 1),
        cake("Carrot Cake", Flavor::Carrot, 380, false, 2),
        cake("Vanilla Sponge", Flavor::Vanilla, 290, true, 2),
        cake("Chocolate Torte", Flavor::Chocolate, 600, true, 1),
    ])
    .exec(db)
    .await?;

    let bakes = |cake_id: i32, baker_id: i32| cake_baker::ActiveModel {
        cake_id: set(cake_id),
        baker_id: set(baker_id),
    };
    Insert::many([
        bakes(1, 1),
        bakes(2, 1),
        bakes(2, 2),
        bakes(5, 2),
        bakes(3, 3),
        bakes(4, 3),
    ])
    .exec(db)
    .await?;

    Insert::many([
        customer::ActiveModel {
            name: set("Kim"),
            email: set("kim@example.com"),
            notes: set(None),
            ..Default::default()
        },
        customer::ActiveModel {
            name: set("Lee"),
            email: set("lee@example.com"),
            notes: set(Some("Allergic to nuts".to_owned())),
            ..Default::default()
        },
    ])
    .exec(db)
    .await?;

    let placed = |at: &str| -> Result<DateTimeWithTimeZone, Error> {
        at.parse()
            .map_err(|err| Error::Custom(format!("{at} is not a timestamp: {err}")))
    };
    Insert::many([
        order::ActiveModel {
            customer_id: set(1),
            bakery_id: set(1),
            placed_at: set(placed("2026-11-02T09:30:00Z")?),
            ..Default::default()
        },
        order::ActiveModel {
            customer_id: set(2),
            bakery_id: set(2),
            placed_at: set(placed("2026-11-03T14:00:00Z")?),
            ..Default::default()
        },
    ])
    .exec(db)
    .await?;

    let line = |order_id: i32, cake_id: i32, quantity: i32, cents: i64| order_line::ActiveModel {
        order_id: set(order_id),
        cake_id: set(cake_id),
        quantity: set(quantity),
        price: set(Decimal::new(cents, 2)),
        ..Default::default()
    };
    Insert::many([line(1, 1, 2, 450), line(1, 2, 1, 320), line(2, 3, 1, 380)])
        .exec(db)
        .await?;
    Ok(())
}
