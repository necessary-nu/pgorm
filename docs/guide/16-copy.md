# COPY

`COPY` moves rows between a table and the client in bulk. It is the fastest
way to load many rows into PostgreSQL, or to read many out: the rows stream
over the connection as one data flow, with no statement per row. pgorm runs
it through [`CopyTrait`](crate::CopyTrait), implemented for a
[`DatabaseConnection`](crate::DatabaseConnection), a
[`DatabaseTransaction`](crate::DatabaseTransaction) (and so a savepoint), and
the [metrics](crate::guide::metrics) wrappers.

A COPY is described by a typed statement from the query builder:
[`CopyFromStatement`](crate::pgorm_query::CopyFromStatement) for
`COPY .. FROM STDIN`, rows sent into a table, and
[`CopyToStatement`](crate::pgorm_query::CopyToStatement) for
`COPY .. TO STDOUT`, rows read out of a table or a query. Each carries its
format as a type parameter, one of PostgreSQL's three:

| Format | Type | The data |
| --- | --- | --- |
| text | [`CopyText`](crate::pgorm_query::CopyText) | tab-separated lines, `\N` for NULL |
| CSV | [`CopyCsv`](crate::pgorm_query::CopyCsv) | comma-separated values with quoting |
| binary | [`CopyBinary`](crate::pgorm_query::CopyBinary) | each field in its type's binary form |

The options a format takes are methods of its statement, and only of its
statement: a CSV-only option such as `quote` is not a method of a text COPY,
and a binary COPY has no delimiter, header or NULL string to set. A COPY takes
no bound parameters, so a statement has one rendering, its `Display`, with any
value in it written as an escaped literal.

## Loading rows

[`copy_in`](crate::CopyTrait::copy_in) starts a `COPY .. FROM STDIN` and
returns a [`CopyInSink`](crate::CopyInSink), a `futures` `Sink` of
[`Bytes`](crate::Bytes). What you send is the format's own encoding of the
rows, split anywhere: a chunk need not end at a row. The sink sends the data
as it arrives and holds the sender back when the connection is busy, so a
large source streams rather than piling up in memory.
[`finish`](crate::CopyInSink::finish) completes the COPY and returns the
number of rows it inserted.

The bakery's new cakes arrive as tab-separated text. A column list names the
columns each line holds; the others take their defaults, so the database
assigns each cake its id:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("copy_text_in").await;
# let pool = scratch.pool.clone();
use futures::SinkExt;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{CopyFromStatement, CopyText};
use pgorm::{Bytes, CopyTrait};

let copy = CopyFromStatement::new(cake::Entity, CopyText)
    .columns([
        cake::Column::Name,
        cake::Column::Flavor,
        cake::Column::Price,
        cake::Column::GlutenFree,
        cake::Column::BakeryId,
    ])
    .to_owned();
assert_eq!(
    copy.to_string(),
    r#"COPY "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") FROM STDIN WITH (FORMAT text)"#
);

let mut db = pool.get().await?;
let mut sink = db.copy_in(&copy).await?;
sink.send(Bytes::from_static(b"Lemon Tart\tlemon\t4.10\tf\t1\n")).await?;
sink.send(Bytes::from_static(b"Carrot Muffin\tcarrot\t2.20\tt\t2\n")).await?;
assert_eq!(sink.finish().await?, 2);

let cakes = cake::Entity::find().count(&db).await?;
assert_eq!(cakes, 7);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

`copy_in` borrows the connection exclusively for as long as the sink lives.
A COPY in progress holds the connection: a statement sent while it is open
would wait behind data not yet sent, and so would the `COMMIT` of a
transaction the COPY runs in. The borrow makes both a compile error rather
than a hang:

```rust,compile_fail,E0499
use futures::SinkExt;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{CopyFromStatement, CopyText};
use pgorm::{Bytes, CopyTrait};
use pgorm_guide_support::cake;

async fn interleaved(db: &mut DatabaseConnection) -> Result<(), Error> {
    let copy = CopyFromStatement::new(cake::Entity, CopyText);
    let mut sink = db.copy_in(&copy).await?;
    // The sink still holds the connection.
    let cakes = cake::Entity::find().count(&*db).await?;
    sink.send(Bytes::from(format!("{cakes}\n"))).await?;
    sink.finish().await?;
    Ok(())
}
```

A row the server refuses, such as one that breaks a constraint or a field
that does not parse, is not reported while the data is still being sent, since
the protocol carries no error back to a client that is sending. It is the
error `finish` returns, and the COPY then inserts nothing. Dropping a sink
without finishing it aborts the COPY: no row is written. Inside a transaction
either one aborts the transaction, as any failed statement does.

## Reading rows

[`copy_out`](crate::CopyTrait::copy_out) starts a `COPY .. TO STDOUT` and
returns a [`CopyOutStream`](crate::CopyOutStream), a `Stream` of `Bytes`
chunks in the statement's format. The server sends rows as the stream is
read, so a slow reader holds the server back rather than filling memory.

A `CopyToStatement` reads a whole table, or the rows of a `SELECT`. A
`SELECT` is also how to read some of a table's columns or rows. Here the
Sunrise bakery's price list is written as CSV with a header line:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("copy_csv_out").await;
# let pool = scratch.pool.clone();
use futures::TryStreamExt;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{CopyCsv, CopyToStatement, Query};
use pgorm::CopyTrait;

let price_list = Query::select()
    .columns([cake::Column::Name, cake::Column::Price])
    .from(cake::Entity)
    .and_where(cake::Column::BakeryId.eq(1))
    .order_by(cake::Column::Name, pgorm::pgorm_query::Order::Asc)
    .to_owned();
let copy = CopyToStatement::new(price_list, CopyCsv).header().to_owned();
assert_eq!(
    copy.to_string(),
    r#"COPY (SELECT "name", "price" FROM "cake" WHERE "cake"."bakery_id" = 1 ORDER BY "name" ASC) TO STDOUT WITH (FORMAT csv, HEADER true)"#
);

let mut db = pool.get().await?;
let chunks: Vec<pgorm::Bytes> = db.copy_out(&copy).await?.try_collect().await?;
let csv: String = chunks
    .iter()
    .map(|chunk| String::from_utf8_lossy(chunk))
    .collect();
assert_eq!(
    csv,
    "name,price\nChocolate Fudge,4.50\nChocolate Torte,6.00\nLemon Drizzle,3.20\n"
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A write is a source too, when it has a `RETURNING` list: COPY TO copies the
rows it returns. Since a write without one returns nothing to copy, it is
converted with [`CopySource::try_from`](crate::pgorm_query::CopySource), which
refuses it, rather than passed straight to `CopyToStatement::new`.

A `SELECT` source PostgreSQL's grammar would refuse inside a COPY, one
with a tie limit (`FETCH .. WITH TIES`) or a `WINDOW` clause, is refused by
`copy_out` as [`Error::QueryBuilder`](crate::Error::QueryBuilder) before
anything is sent.

Dropping a `CopyOutStream` early does not stop the server. The connection
reads and discards the rest of the data before its next statement runs.

## The text and CSV options

The two text formats share most of their options, and CSV adds a few of its
own. On COPY FROM:

| Option | Formats | Does |
| --- | --- | --- |
| `delimiter(c)` | text, CSV | separates the fields |
| `null_string(s)` | text, CSV | the field that means NULL |
| `default_string(s)` | text, CSV | the field that means the column's default |
| `header()`, `header_match()` | text, CSV | skips a header line; `header_match` also checks it names the columns |
| `encoding(e)` | text, CSV | the data's character encoding |
| `on_error_ignore(..)` | text, CSV | skips a row whose field does not convert, rather than failing the COPY |
| `quote(c)`, `escape(c)` | CSV | the quoting and escape characters |
| `force_null(..)`, `force_not_null(..)` | CSV | for the columns given, reads a field matching the NULL string as NULL even when quoted, or never as NULL; `force_null_all` and `force_not_null_all` for every column |
| `freeze()` | all | writes the rows frozen, into a table created or emptied in the same transaction |

On COPY TO the text formats take `delimiter`, `null_string`, `header` and
`encoding`, and CSV adds `quote`, `escape` and `force_quote`, which quotes
every non-NULL field of the columns given (`force_quote_all` for every
column).

`on_error_ignore` takes a [`CopyIgnore`](crate::pgorm_query::CopyIgnore),
which can also bound how many rows may be skipped before the COPY fails after
all. Here a price list from a supplier has one line whose price is not a
number, and the COPY loads the others:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("copy_csv_options").await;
# let pool = scratch.pool.clone();
use futures::SinkExt;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{CopyCsv, CopyFromStatement, CopyIgnore};
use pgorm::{Bytes, CopyTrait};

let copy = CopyFromStatement::new(cake::Entity, CopyCsv)
    .columns([
        cake::Column::Name,
        cake::Column::Flavor,
        cake::Column::Price,
        cake::Column::GlutenFree,
        cake::Column::BakeryId,
    ])
    .header()
    .quote('\'')
    .on_error_ignore(CopyIgnore::default())
    .to_owned();
assert_eq!(
    copy.to_string(),
    r#"COPY "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") FROM STDIN WITH (FORMAT csv, HEADER true, QUOTE E'\'', ON_ERROR ignore)"#
);

let mut db = pool.get().await?;
let mut sink = db.copy_in(&copy).await?;
sink.send(Bytes::from_static(
    b"name,flavor,price,gluten_free,bakery_id\n\
      'Vanilla Slice, iced',vanilla,3.40,false,2\n\
      Lemon Bun,lemon,ask,false,1\n",
))
.await?;
// The second line's price did not convert, so it was skipped.
assert_eq!(sink.finish().await?, 1);

let slice = cake::Entity::find()
    .filter(cake::Column::Name.eq("Vanilla Slice, iced"))
    .one(&db)
    .await?;
assert_eq!(slice.price, Decimal::new(340, 2));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Rows of values

The binary format sends each field in its type's binary form, the form a
bound parameter travels in, so a binary COPY can carry rows of pgorm
[`Value`](crate::Value)s with no text to write or parse. The format itself
names no column types, so pgorm asks the server:
[`copy_in_columns`](crate::CopyTrait::copy_in_columns) and
[`copy_out_columns`](crate::CopyTrait::copy_out_columns) describe a COPY's
columns, in order, each a [`CopyColumn`](crate::CopyColumn) with its name and
type, by preparing a `SELECT` of them and running nothing.

[`copy_in_rows`](crate::CopyTrait::copy_in_rows) and
[`copy_out_rows`](crate::CopyTrait::copy_out_rows) do that, then start the
COPY:

- A [`CopyRowSink`](crate::CopyRowSink) takes each row with
  [`send`](crate::CopyRowSink::send), one value for each column, or a model
  with [`send_model`](crate::CopyRowSink::send_model), which takes each of
  the COPY's columns from the model's field of that name. A row of the wrong
  width, or a value its column's type does not take, is refused with
  [`Error::Type`](crate::Error::Type) before anything of it is written, and
  the sink goes on taking rows.
- A [`CopyRowStream`](crate::CopyRowStream) yields each row as a
  `Vec<Value>`, each field decoded by its column's type, and
  [`models`](crate::CopyRowStream::models) reads each row into an entity's
  model instead.

Here every cake at Sunrise is copied out as models and copied back in as new
cakes at Moonlight. The COPY FROM lists every column but the id, so
`send_model` sends each model's other columns and the database assigns new
ids:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("copy_binary_rows").await;
# let pool = scratch.pool.clone();
use futures::TryStreamExt;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{CopyBinary, CopyFromStatement, CopyToStatement, Query};
use pgorm::CopyTrait;

let mut db = pool.get().await?;
let sunrise = CopyToStatement::new(
    Query::select()
        .columns(cake::Column::iter())
        .from(cake::Entity)
        .and_where(cake::Column::BakeryId.eq(1))
        .to_owned(),
    CopyBinary,
);
let mut cakes: Vec<cake::Model> = db
    .copy_out_rows(&sunrise)
    .await?
    .models::<cake::Entity>()
    .try_collect()
    .await?;
assert_eq!(cakes.len(), 3);

let into = CopyFromStatement::new(cake::Entity, CopyBinary)
    .columns([
        cake::Column::Name,
        cake::Column::Flavor,
        cake::Column::Price,
        cake::Column::GlutenFree,
        cake::Column::BakeryId,
    ])
    .to_owned();
let mut sink = db.copy_in_rows(&into).await?;
let names: Vec<&str> = sink.columns().iter().map(|column| column.name()).collect();
assert_eq!(names, ["name", "flavor", "price", "gluten_free", "bakery_id"]);
for cake in &mut cakes {
    cake.bakery_id = 2;
    sink.send_model(cake).await?;
}
assert_eq!(sink.finish().await?, 3);

let at_moonlight = cake::Entity::find()
    .filter(cake::Column::BakeryId.eq(2))
    .count(&db)
    .await?;
assert_eq!(at_moonlight, 5);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A row of values is written the same way, one value per column, and the
row's values are checked against the column types before any of it is sent:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("copy_binary_values").await;
# let pool = scratch.pool.clone();
use futures::TryStreamExt;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{CopyBinary, CopyFromStatement, CopyToStatement};
use pgorm::CopyTrait;

let mut db = pool.get().await?;
let bakeries = CopyFromStatement::new(pgorm_guide_support::bakery::Entity, CopyBinary)
    .columns([
        pgorm_guide_support::bakery::Column::Name,
        pgorm_guide_support::bakery::Column::City,
    ])
    .to_owned();
let mut sink = db.copy_in_rows(&bakeries).await?;
sink.send(["Daybreak".into(), "Trondheim".into()]).await?;

// A row of the wrong width is refused, and the sink goes on taking rows.
let refused = sink.send(["Dusk".into()]).await;
assert!(matches!(refused, Err(Error::Type(_))));
assert_eq!(sink.finish().await?, 1);

let rows: Vec<Vec<Value>> = db
    .copy_out_rows(&CopyToStatement::new(pgorm_guide_support::bakery::Entity, CopyBinary))
    .await?
    .try_collect()
    .await?;
assert_eq!(
    rows[2],
    [Value::Int(Some(3)), "Daybreak".into(), "Trondheim".into()]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

Each field reads as the `Value` variant its column's type reads as: the text
types and an enum's label as `String`, a `numeric` as `Decimal`, an array as
an `Array` of its element's variant, and so on. A `numeric` field reads
exactly, its scale included, and one a `Decimal` cannot hold exactly is
refused rather than rounded. A `money` column reads and writes as a `Decimal`
in its currency's units, the places of which the description asks the server
for.

## Encoding rows yourself

[`CopyRowEncoder`](crate::CopyRowEncoder) and
[`CopyRowDecoder`](crate::CopyRowDecoder) are the binary format's two sides,
over a list of columns, for code that moves the bytes itself: writing a COPY
file to disk, or reading one produced elsewhere. The encoder writes the
header, each row of any `ToSql` values (checked as `send` checks one), and the
trailer, into a `BytesMut` from the [`bytes`](https://docs.rs/bytes) crate.
The decoder takes the data in chunks split anywhere and yields each row once
all of it has arrived:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("copy_encoder").await;
# let pool = scratch.pool.clone();
use bytes::BytesMut;
use futures::{SinkExt, TryStreamExt};
use pgorm::pgorm_query::{CopyBinary, CopyFromStatement, CopyToStatement, Query};
use pgorm::types::Type;
use pgorm::{Bytes, CopyColumn, CopyRowDecoder, CopyRowEncoder, CopyTrait, Value};
use pgorm_guide_support::bakery;

let columns = vec![
    CopyColumn::new("name", Type::VARCHAR),
    CopyColumn::new("city", Type::VARCHAR),
];
let encoder = CopyRowEncoder::new(columns.clone());
let mut data = BytesMut::new();
CopyRowEncoder::header(&mut data);
encoder.encode(&["Noon", "Stavanger"], &mut data)?;
encoder.encode(&["Night Owl", "Tromsø"], &mut data)?;
CopyRowEncoder::trailer(&mut data);

let mut db = pool.get().await?;
let into = CopyFromStatement::new(bakery::Entity, CopyBinary)
    .columns([bakery::Column::Name, bakery::Column::City])
    .to_owned();
let mut sink = db.copy_in(&into).await?;
sink.send(data.freeze()).await?;
assert_eq!(sink.finish().await?, 2);

let out = CopyToStatement::new(
    Query::select()
        .columns([bakery::Column::Name, bakery::Column::City])
        .from(bakery::Entity)
        .to_owned(),
    CopyBinary,
);
let chunks: Vec<Bytes> = db.copy_out(&out).await?.try_collect().await?;
let mut decoder = CopyRowDecoder::new(columns);
let mut rows = Vec::new();
for chunk in &chunks {
    decoder.push(chunk);
    while let Some(row) = decoder.next_row()? {
        rows.push(row);
    }
}
decoder.finish()?;
assert_eq!(rows.len(), 4);
assert_eq!(rows[3], [Value::from("Night Owl"), Value::from("Tromsø")]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## When the server refuses a COPY

A COPY the server refuses at its start, such as one naming a table or column
that does not exist, is the error `copy_in` or `copy_out` returns. The two
directions leave the connection differently:

- A refused COPY TO leaves the connection as it was.
- A refused COPY FROM closes its connection, and with it any transaction open
  on it, which ends without committing. The driver has by then sent a message
  whose answer nothing is waiting for, and that answer would reach the next
  request on the connection, leaving every later answer one request out of
  step. Closing is the only safe way out. Every later call on the connection
  fails as closed, and the pool discards the connection rather than lend it
  again. The same holds when the `copy_in` future is dropped before it
  completes.

A generated column takes no data, and PostgreSQL refuses a column list that
names one, at the COPY's start. List only the columns the data supplies, or
give no list, which leaves the generated columns out and lets the server
compute them.

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("copy_refused").await;
# let pool = scratch.pool.clone();
use pgorm::pgorm_query::{CopyFromStatement, CopyText, Name};
use pgorm::{ConnectionTrait, CopyTrait, Error};

let mut db = pool.get().await?;
let missing = CopyFromStatement::new(Name::runtime("pastry"), CopyText);
let refused = db.copy_in(&missing).await;
assert!(refused.is_err());

// The connection is closed now.
let after = db.query_one("SELECT 1", &[]).await;
assert!(matches!(after, Err(Error::Postgres(error)) if error.is_closed()));
drop(db);

// The pool does not lend it again.
let db = pool.get().await?;
let one: i32 = db.query_one("SELECT 1", &[]).await?.get(0);
assert_eq!(one, 1);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```
