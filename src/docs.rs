//! 1. Async
//!
//!    Built on pgorm-driver (maintained from [tokio-postgres](https://github.com/rust-postgres/rust-postgres)) and
//!    [deadpool](https://github.com/bikeshedder/deadpool), pgorm is async from
//!    day 1: a connection is a pooled handle, and independent queries can be
//!    driven concurrently on separate handles.
//!
//! ```no_run
//! # use pgorm::{entity::*, error::*, query::*, tests_cfg::*, DatabasePool};
//! #
//! # async fn function(pool: &DatabasePool) -> Result<(), Error> {
//! // one pooled connection per concurrent query
//! let (cake_conn, fruit_conn) = futures::try_join!(pool.get(), pool.get())?;
//!
//! // execute multiple queries in parallel
//! let cakes_and_fruits: (Vec<cake::Model>, Vec<fruit::Model>) =
//!     futures::try_join!(Cake::find().all(&cake_conn), Fruit::find().all(&fruit_conn))?;
//! # Ok(())
//! # }
//! ```
//!
//! 2. Dynamic
//!
//!    Built upon [SeaQuery](https://github.com/SeaQL/sea-query), pgorm allows you to build complex queries without 'fighting the ORM'.
//!
//! ```no_run
//! # use pgorm_query::Query;
//! # use pgorm::{entity::*, error::*, query::*, tests_cfg::*, DatabaseConnection};
//! # async fn function(db: &DatabaseConnection) -> Result<(), Error> {
//! // build subquery with ease
//! let cakes_with_filling: Vec<cake::Model> = cake::Entity::find()
//!     .filter(
//!         Condition::any().add(
//!             cake::Column::Id.in_subquery(
//!                 Query::select()
//!                     .column(cake_filling::Column::CakeId)
//!                     .from(cake_filling::Entity)
//!                     .to_owned(),
//!             ),
//!         ),
//!     )
//!     .all(db)
//!     .await?;
//!
//! # Ok(())
//! # }
//! ```
//!
//! 3. Inspectable
//!
//!    Every builder renders to PostgreSQL text plus its bound parameters
//!    before it is sent, so a query can be asserted on without a database.
//!
//! ```
//! use pgorm::pgorm_query::{Value, Values};
//! use pgorm::{entity::*, query::*, tests_cfg::*};
//!
//! let (sql, values) = cake::Entity::find()
//!     .filter(cake::Column::Name.contains("chocolate"))
//!     .build();
//!
//! assert_eq!(
//!     sql,
//!     r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."name" LIKE $1"#
//! );
//! assert_eq!(
//!     values,
//!     Values(vec![Value::String(Some(Box::new("%chocolate%".to_owned())))])
//! );
//! ```
//!
//! 4. Service Oriented
//!
//!    Quickly build services that join, filter, sort and paginate data in APIs.
//!
//!    The sketch below is `ignore`d because it is written against a web
//!    framework (Rocket) that pgorm does not depend on.
//!
//! ```ignore
//! use std::num::NonZeroU64;
//!
//! const DEFAULT_PER_PAGE: NonZeroU64 = NonZeroU64::new(10).unwrap();
//!
//! #[get("/?<page>&<posts_per_page>")]
//! async fn list(
//!     conn: Connection<Db>,
//!     page: Option<u64>,
//!     per_page: Option<NonZeroU64>,
//! ) -> Template {
//!     // Set page number and items per page
//!     let page = page.unwrap_or(1);
//!     let per_page = per_page.unwrap_or(DEFAULT_PER_PAGE);
//!
//!     // Setup paginator
//!     let paginator = Post::find()
//!         .order_by_asc(post::Column::Id)
//!         .paginate(&conn, per_page);
//!     let num_pages = paginator.num_pages().await.unwrap();
//!
//!     // Fetch paginated posts
//!     let posts = paginator
//!         .fetch_page(page - 1)
//!         .await
//!         .expect("could not retrieve posts");
//!
//!     Template::render(
//!         "index",
//!         context! {
//!             page: page,
//!             per_page: per_page,
//!             posts: posts,
//!             num_pages: num_pages,
//!         },
//!     )
//! }
//! ```
