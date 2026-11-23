# PostgreSQL 19

pgorm targets PostgreSQL 19, and nothing older. No feature names a release:
every build makes the statements 19 accepts, the documentation gives 19's
answers, and pgorm's own tests run against a 19 server. An application whose
server runs an older release moves it to 19 before it moves to this pgorm.

## What pgorm uses

pgorm uses what recent releases added wherever it fits. Among what this guide
covers, from PostgreSQL 18:

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

## Coming from PostgreSQL 18

Some statements 18 and 19 both accept, but answer differently. A builder value
records only what the statement says, not what the server holds, so the
statement renders as it did, and the server's answer is 19's:

- `ALTER CONSTRAINT .. ENFORCED` and `NOT ENFORCED` reach a `CHECK` constraint
  as they reach a foreign key, `ENFORCED` checking the rows already there;
- `SET EXPRESSION` on a virtual generated column that a `CHECK` reads checks
  the rows against the `CHECK`, where 18 refused it;
- `JSON_ARRAY` over a query of no rows is the empty array, where 18 answered
  `NULL`.

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

let names: Json = (sql, values).into_tuple().one(&db).await?;
assert_eq!(names, serde_json::json!([]));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## The grammar is PostgreSQL 19's

Every SQL parse pgorm makes uses one parser, pgsql-syntax, a Rust port of
PostgreSQL 19's own scanner and parser: [`sql!`](crate::sql) and
[`prql!`](crate::prql), the files `sql_migration!` reads, their run-time forms
[`sql_check::check_sql`](crate::sql_check::check_sql) and
[`sql_check::compile_prql`](crate::sql_check::compile_prql)
([Raw SQL](crate::guide::raw_sql)), code generation's DDL reader
([Generating entities](crate::guide::codegen)), the paginator's reading of
raw SQL, and query fingerprints in [Metrics](crate::guide::metrics). A check
of text is a check against 19's grammar, so what 19 added passes it:

```rust
use pgorm::sql_check::check_sql;

// REPACK and ON CONFLICT .. DO SELECT are PostgreSQL 19's.
assert!(check_sql("REPACK cake").is_ok());
assert!(check_sql("INSERT INTO cake (id) VALUES (1) ON CONFLICT (id) DO SELECT RETURNING *").is_ok());
```

19's grammar also refuses a little that 18's parsed. It makes `ignore` and
`respect` keywords that a column label needs `AS` before, so a bare
`SELECT 1 ignore`, which an 18 server ran, is refused; `SELECT 1 AS ignore`
parses. pgorm writes `AS` before every column label it renders, so no
statement a builder makes is affected. The five keywords 19 adds, `ignore`,
`lsn`, `repack`, `respect` and `wait`, are all unreserved, so pgorm quotes the
identifiers it quoted before.
