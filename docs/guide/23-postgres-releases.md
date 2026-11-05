# PostgreSQL 18 and 19

pgorm targets PostgreSQL 18 or later, and nothing older. A build targets one
major release, 18 unless it asks for 19, and the release decides two things:
which statements the builders can make, and which answers the documentation
and the tests expect where the two releases' servers answer one statement
differently.

## The default: PostgreSQL 18

A build with no feature named targets PostgreSQL 18. There is no feature to
say so, and no build reaches back to 17. pgorm uses what 18 added wherever it
fits. Among what this guide covers:

- *virtual generated columns*, computed when read rather than stored
  (`generated_virtual` in [Entities](crate::guide::entities));
- *temporal keys*, a primary key `WITHOUT OVERLAPS` and a foreign key over a
  `PERIOD` (`without_overlaps` and `from_period`);
- `uuidv7()`, a time-ordered UUID a key can default to;
- `RETURNING` the old and the new version of a written row
  ([Writing rows](crate::guide::writing));
- named `NOT NULL` constraints, and constraints declared `NOT ENFORCED`
  ([Schema generation](crate::guide::schema)).

```rust
mod oven_booking {
    use pgorm::entity::prelude::*;

    /// When each of the bakery's ovens is booked, and for which batch.
    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "oven_booking")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub oven_id: i32,
        #[pgorm(primary_key, without_overlaps)]
        pub during: Range<DateTimeWithTimeZone>,
        #[pgorm(default_expr = Func::uuidv7())]
        pub batch: Uuid,
        pub trays: i32,
        #[pgorm(generated_virtual = Expr::col(Column::Trays).mul(12))]
        pub cakes: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

# fn main() -> Result<(), pgorm::Error> {
let create = pgorm::Schema::new().create_table_from_entity(oven_booking::Entity)?;
assert_eq!(
    create.to_string(),
    r#"CREATE TABLE "oven_booking" ( "oven_id" integer NOT NULL, "during" tstzrange NOT NULL, "batch" uuid NOT NULL DEFAULT UUIDV7(), "trays" integer NOT NULL, "cakes" integer NOT NULL GENERATED ALWAYS AS ("trays" * 12) VIRTUAL, CONSTRAINT "pk-oven_booking" PRIMARY KEY ("oven_id", "during" WITHOUT OVERLAPS) )"#
);
# Ok(())
# }
```

A temporal key over a plain column beside the range, such as `oven_id`
here, needs the `btree_gist` extension in the database.

## Asking for PostgreSQL 19

The `pg-19` feature targets PostgreSQL 19:

```toml
[dependencies]
pgorm = { git = "https://github.com/necessary-nu/pgorm", features = ["pg-19"] }
```

The feature only adds. What 19 alone accepts arrives behind it as new items,
and nothing an 18 build has changes shape or meaning: a statement built the
same way renders the same SQL under either target. That matters because cargo
turns a feature on for every crate in a build once any one crate asks for it.
A library elsewhere in the dependency graph that turns on `pg-19` cannot break
this crate's statements.

The open vocabularies, such as the kinds of column type, are
`#[non_exhaustive]` enums, so a `match` over one already has an arm for what
it does not know, and the variants 19 adds compile against it.

A crate cannot read another crate's features through `cfg`, so pgorm says
which release a build targets as a constant,
[`pgorm_query::POSTGRES_TARGET`](crate::pgorm_query::POSTGRES_TARGET). A
program that needs 19 can refuse to compile without it:

```rust
use pgorm::pgorm_query::POSTGRES_TARGET;

#[cfg(not(feature = "pg-19"))]
assert_eq!(POSTGRES_TARGET, 18);
#[cfg(feature = "pg-19")]
assert_eq!(POSTGRES_TARGET, 19);

// In a program that relies on 19:
// const _: () = assert!(POSTGRES_TARGET >= 19, "build pgorm with its pg-19 feature");
```

The feature passes through: pgorm forwards it to pgorm-query, and the Python
binding forwards it to pgorm.

## Where the two releases answer differently

Some statements both releases accept, but answer differently. A builder value
records only what the statement says, not what the server holds, so pgorm
renders the statement the same way for both and documents both answers. The
API documentation of each such item gives 18's answer and 19's. Three
differences exist today:

- `ALTER CONSTRAINT .. ENFORCED` and `NOT ENFORCED` reach a `CHECK` constraint
  from 19, and only a foreign key on 18;
- `SET EXPRESSION` on a virtual generated column that a `CHECK` reads is
  refused on 18 and accepted on 19;
- `JSON_ARRAY` over a query of no rows is `NULL` on 18 and the empty array on
  19.

Code that runs against both decodes the last into an `Option`:

```rust
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{Func, Query};
use pgorm_guide_support::cake;

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("postgres_releases_json_array").await;
# let db = scratch.pool.get().await?;
// The names of the cakes over 10.00: there are none.
let dear = Query::select()
    .column(cake::Column::Name)
    .from(cake::Entity)
    .and_where(cake::Column::Price.gt(Decimal::new(1000, 2)))
    .to_owned();
let (sql, values) = Query::select().expr(Func::json_array_query(dear)).build();

let names: Option<Json> = (sql, values).into_tuple().one(&db).await?;
#[cfg(not(feature = "pg-19"))]
assert_eq!(names, None);
#[cfg(feature = "pg-19")]
assert_eq!(names, Some(serde_json::json!([])));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

Each build is tested against the server it targets: the default build
against PostgreSQL 18, and a `pg-19` build against 19.

## The grammar stays at 18 for now

Every SQL parse pgorm makes uses PostgreSQL 18's grammar, whichever release a
build targets: [`sql!`](crate::sql) and [`prql!`](crate::prql), code
generation's DDL reader ([Generating entities](crate::guide::codegen)), the
paginator's reading of raw SQL, and query fingerprints in
[Metrics](crate::guide::metrics). The parser is libpg_query, which has no 19 grammar
yet. Until it does, `sql!` refuses syntax only 19 parses under `pg-19` too,
and no builder makes such syntax, since everything a builder renders has to
parse.
