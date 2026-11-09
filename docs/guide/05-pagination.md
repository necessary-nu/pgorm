# Pages, cursors and streams

A query whose result is too long to read at once can be read a piece at a
time, three ways:

- *Pages* by number, with [`paginate`](crate::PaginatorTrait::paginate): page
  3 of 20, the shape of a list with numbered pages. Each page is a
  `LIMIT`/`OFFSET`, so a deep page costs the database every row before it.
- *Cursors*, with [`cursor_by`](crate::Select::cursor_by): the rows after (or
  before) a given row, the shape of an endless feed or a "next" link. Each
  window is a `WHERE` on the ordering columns, so a deep window costs no more
  than the first, and rows written between two reads do not shift it.
- *Streams*, with [`stream`](crate::Select::stream): every row of one query,
  decoded as it arrives from the server instead of collected into a `Vec`.

The examples page through the bakery's five cakes.

## Pages

[`paginate`](crate::PaginatorTrait::paginate) turns a query into a
[`Paginator`](crate::Paginator) with a page size, a
[`NonZeroU64`](std::num::NonZeroU64), so a page size of zero does not
compile. Pages are numbered from zero.
[`fetch_page`](crate::Paginator::fetch_page) reads one;
[`num_items`](crate::Paginator::num_items),
[`num_pages`](crate::Paginator::num_pages) and
[`num_items_and_pages`](crate::Paginator::num_items_and_pages) count, each
running a `COUNT(*)` over the query.

Order the query before paginating it. Without an `ORDER BY`, PostgreSQL may
return the rows in a different order for each page, and a row can then
appear on two pages or on none.

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pagination_pages").await;
# let db = scratch.pool.get().await?;
use std::num::NonZeroU64;

use pgorm::entity::prelude::*;

const PAGE_SIZE: NonZeroU64 = NonZeroU64::new(2).unwrap();

let pages = cake::Entity::find()
    .order_by_asc(cake::Column::Name)
    .paginate(&db, PAGE_SIZE);

let counts = pages.num_items_and_pages().await?;
assert_eq!((counts.number_of_items, counts.number_of_pages), (5, 3));

let second: Vec<String> = pages
    .fetch_page(1)
    .await?
    .into_iter()
    .map(|cake| cake.name)
    .collect();
assert_eq!(second, ["Chocolate Torte", "Lemon Drizzle"]);
assert!(pages.fetch_page(3).await?.is_empty());
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A paginator also keeps a current page, which
[`fetch`](crate::Paginator::fetch) reads and
[`advance`](crate::Paginator::advance) moves on without reading.
[`fetch_and_next`](crate::Paginator::fetch_and_next) does both, and answers
`None` at the first empty page, which makes it a loop over every page;
[`into_stream`](crate::Paginator::into_stream) is the same loop as a stream
of pages:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pagination_fetch_and_next").await;
# let db = scratch.pool.get().await?;
use std::num::NonZeroU64;

use futures::TryStreamExt;
use pgorm::entity::prelude::*;

let page_size = NonZeroU64::new(2).expect("two is not zero");

let mut pages = cake::Entity::find()
    .order_by_asc(cake::Column::Id)
    .paginate(&db, page_size);
let mut sizes = Vec::new();
while let Some(cakes) = pages.fetch_and_next().await? {
    sizes.push(cakes.len());
}
assert_eq!(sizes, [2, 2, 1]);

let all_pages: Vec<Vec<cake::Model>> = cake::Entity::find()
    .order_by_asc(cake::Column::Id)
    .paginate(&db, page_size)
    .into_stream()
    .try_collect()
    .await?;
assert_eq!(all_pages.len(), 3);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A projection decoded with `into_tuple` or `into_model` paginates the same
way, and so does a [graph](crate::guide::relations) of related rows; a read of a
[disjoint enum](crate::guide::disjoint) has its own `paginate`.
[`count`](crate::PaginatorTrait::count) is the paginator's count without the
pages.

## Cursors

[`cursor_by`](crate::Select::cursor_by) orders a query by one column or a
tuple of columns, the cursor's *key*, and returns a
[`Cursor`](crate::Cursor). The key should identify a row, so end it with the
primary key: rows that tie on the key cannot be told apart, and a window
that ends inside a run of them cannot say where it ended.

[`after`](crate::Cursor::after) and [`before`](crate::Cursor::before) take a
row's key values and keep the rows past it in the cursor's order.
[`first`](crate::Cursor::first) takes that many rows from the start of what
is left and [`last`](crate::Cursor::last) from its end;
[`asc`](crate::Cursor::asc) and [`desc`](crate::Cursor::desc) set the
direction. [`all`](crate::Cursor::all) runs it, and always returns the rows
in the cursor's order, `last` included.

A feed of the cakes by price, cheapest first, reads a window, then the window
after its last row:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pagination_cursor").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

let names = |cakes: &[cake::Model]| -> Vec<String> {
    cakes.iter().map(|cake| cake.name.clone()).collect()
};

let mut feed = cake::Entity::find().cursor_by((cake::Column::Price, cake::Column::Id));
let first = feed.first(2).all(&db).await?;
assert_eq!(names(&first), ["Vanilla Sponge", "Lemon Drizzle"]);

// The next window starts after the last row read.
let last_seen = first.last().expect("the first window is not empty");
let next = feed
    .after((last_seen.price, last_seen.id))
    .first(2)
    .all(&db)
    .await?;
assert_eq!(names(&next), ["Carrot Cake", "Chocolate Fudge"]);

// The two dearest, from the far end, still in ascending order.
let dearest = cake::Entity::find()
    .cursor_by((cake::Column::Price, cake::Column::Id))
    .last(2)
    .all(&db)
    .await?;
assert_eq!(names(&dearest), ["Chocolate Fudge", "Chocolate Torte"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The key's length fixes the boundary's: a cursor over `(price, id)` takes a
pair, and a single value does not compile. A cursor runs again each time
`all` is called, with the boundary and window it holds then, so one cursor
serves a whole feed.

The cursor writes the ordering and the boundary itself. A boundary over
several columns becomes the comparison that reads them in order:

```rust
# use pgorm_guide_support::cake;
use pgorm::entity::prelude::*;

let mut feed = cake::Entity::find()
    .select(cake::Column::Name)
    .cursor_by((cake::Column::Price, cake::Column::Id));
feed.after((Decimal::new(320, 2), 2)).first(2);
assert_eq!(
    feed.statement()?.build().0,
    r#"SELECT "cake"."name" FROM "cake" WHERE ("cake"."price" = $1 AND "cake"."id" > $2) OR "cake"."price" > $3 ORDER BY "cake"."price" ASC, "cake"."id" ASC LIMIT $4"#
);
# Ok::<(), pgorm::Error>(())
```

A cursor over chosen columns, as above, decodes once it is told what into,
with [`into_model`](crate::Cursor::into_model) or
[`into_partial_model`](crate::Cursor::into_partial_model).

### Ties and secondary keys

[`add_secondary_order_by`](crate::Cursor::add_secondary_order_by) appends a
column of the queried table or a joined one to the cursor's ordering, after
the key, which is how a cursor over a join is made total. `after` and
`before` still take the key alone, and so resume past every row sharing the
boundary's key. [`after_with`](crate::Cursor::after_with) and
[`before_with`](crate::Cursor::before_with) take the whole ordering, the
appended columns included, and resume from exactly one row:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pagination_secondary").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

// By bakery, then by id within each bakery.
let mut by_bakery = cake::Entity::find().cursor_by(cake::Column::BakeryId);
by_bakery.add_secondary_order_by(cake::Entity, cake::Column::Id);

let first = by_bakery.first(2).all(&db).await?;
let ids: Vec<i32> = first.iter().map(|cake| cake.id).collect();
assert_eq!(ids, [1, 2]);

// Bakery 1 has a third cake, which `after(1)` would skip.
let rest = by_bakery.after_with((1, 2)).first(2).all(&db).await?;
let ids: Vec<i32> = rest.iter().map(|cake| cake.id).collect();
assert_eq!(ids, [5, 3]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A [graph](crate::guide::relations) has cursors too, `cursor_by` on its root and
`cursor_by_on` on one of its related tables, and adds every related table's
primary key to the ordering itself.

## Paginating raw SQL

A query written as SQL paginates and counts too:
[`SelectorRaw::paginate`](crate::SelectorRaw::paginate) wraps the statement
as a subquery and pages over it, so its own `ORDER BY` still decides the
pages. pgorm reads the statement with PostgreSQL's own parser to do so, and
refuses anything but a single `SELECT` (a `WITH .. SELECT`, a `VALUES` list
and a `UNION` count as one), so here `paginate` and
[`count`](crate::SelectorRaw::count) return a `Result`:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pagination_raw").await;
# let db = scratch.pool.get().await?;
use std::num::NonZeroU64;

use pgorm::entity::prelude::*;
use pgorm::pgorm_query::Values;

let sql = r#"SELECT name, price FROM cake WHERE bakery_id = $1 ORDER BY price"#;
let page_size = NonZeroU64::new(2).expect("two is not zero");

let pages = (sql, Values(vec![1.into()]))
    .into_tuple::<(String, Decimal)>()
    .paginate(&db, page_size)?;
assert_eq!(pages.num_items().await?, 3);
let first = pages.fetch_page(0).await?;
assert_eq!(first[0], ("Lemon Drizzle".to_owned(), Decimal::new(320, 2)));

let refused = (r#"DELETE FROM cake"#, Values(vec![]))
    .into_tuple::<(i32,)>()
    .count(&db)
    .await;
assert!(refused.is_err());
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The wrapping is [`raw_select`](crate::raw_select), for a caller that sends a
page or the count itself: it gives the `SelectStatement` `paginate` pages,
which [`page_statement`](crate::page_statement) and
[`count_statement`](crate::count_statement) take.

```rust
use pgorm::{Value, count_statement, raw_select};

let wrapped = raw_select("SELECT name FROM cake WHERE bakery_id = $1 ORDER BY price", vec![Value::from(1)])?;
let (sql, values) = count_statement(&wrapped)?.build();
assert_eq!(
    sql,
    r#"SELECT COUNT(*) AS num_items FROM (SELECT * FROM (SELECT name FROM cake WHERE bakery_id = $1 ORDER BY price
) AS "sub_statement") AS "sub_query""#
);
assert_eq!(values.0, [Value::from(1)]);
# Ok::<(), pgorm::Error>(())
```

## Streams

[`stream`](crate::Select::stream) runs a query and yields its rows as they
arrive, each decoded when it is polled, so a large result never sits in
memory whole. The stream is `Send`, so a spawned task can consume it. A row
that fails to decode is one `Err` in the stream, and the rows after it still
come. `stream` exists on a [`Select`](crate::Select), on a decoded
[`Selector`](crate::Selector) or [`SelectorRaw`](crate::SelectorRaw), and on
a [graph](crate::guide::relations), and
[`stream_partial_model`](crate::Select::stream_partial_model) streams a
[`PartialModel`](derive@crate::PartialModel).

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pagination_stream").await;
# let db = scratch.pool.get().await?;
use futures::TryStreamExt;
use pgorm::entity::prelude::*;

let mut cakes = cake::Entity::find()
    .order_by_asc(cake::Column::Id)
    .stream(&db)
    .await?;
let mut total = Decimal::ZERO;
while let Some(cake) = cakes.try_next().await? {
    total += cake.price;
}
assert_eq!(total, Decimal::new(2040, 2));

let names: Vec<String> = cake::Entity::find()
    .select(cake::Column::Name)
    .filter(cake::Column::BakeryId.eq(2))
    .order_by_asc(cake::Column::Name)
    .into_tuple()
    .stream(&db)
    .await?
    .try_collect()
    .await?;
assert_eq!(names, ["Carrot Cake", "Vanilla Sponge"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

Underneath, [`ConnectionTrait::query_raw`](crate::ConnectionTrait::query_raw)
streams the rows of any SQL statement as tokio-postgres
[`Row`](crate::Row)s, undecoded:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pagination_query_raw").await;
# let db = scratch.pool.get().await?;
use futures::TryStreamExt;
use pgorm::ConnectionTrait;

let rows: Vec<String> = db
    .query_raw("SELECT name FROM baker WHERE bakery_id = $1 ORDER BY name", [1_i32])
    .await?
    .map_ok(|row| row.get::<_, String>(0))
    .try_collect()
    .await?;
assert_eq!(rows, ["Ada", "Bo"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A stream holds no borrow of its connection, but it reads from it: keep the
connection, or the [transaction](crate::guide::transactions) the stream came from,
until the stream is done. Dropping a stream early is safe, and the connection
stays usable, though the server still runs the query to its end.
