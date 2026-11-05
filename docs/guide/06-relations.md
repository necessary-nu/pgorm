# Relations

A relation says how two tables' rows belong together: a cake belongs to a
bakery, a bakery has many cakes, a cake has many bakers through the
`cake_baker` table. [Entities](crate::guide::entities) declares them on each
entity's `Relation` enum. This chapter reads along them.

pgorm offers three ways to do it, one for each shape of question:

- one model's related rows: [`find_related`](crate::ModelTrait::find_related),
  a query of its own;
- the related rows of a list of models, in one query rather than one per
  model: the loaders of [`LoaderTrait`](crate::LoaderTrait);
- rows read together with their related rows, in one joined query: the
  graph, [`EntityTrait::graph`](crate::EntityTrait::graph).

## The related rows of one model

[`Related<E>`](crate::Related) says how to reach entity `E` from this one,
and [`find_related`](crate::ModelTrait::find_related) turns it into a query
for one model's related rows. The query is an ordinary
[`Select`](crate::Select), so it can be filtered and ordered before it runs:

```rust
# use pgorm_guide_support::{baker, bakery, cake};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("relations_find_related").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

let sunrise = bakery::Entity::find_by_id(1).one(&db).await?;
let cakes = sunrise
    .find_related(cake::Entity)
    .order_by_asc(cake::Column::Price)
    .all(&db)
    .await?;
let names: Vec<&str> = cakes.iter().map(|cake| cake.name.as_str()).collect();
assert_eq!(names, ["Lemon Drizzle", "Chocolate Fudge", "Chocolate Torte"]);

// A cake's bakers, through the cake_baker junction.
let lemon_drizzle = &cakes[0];
let bakers = lemon_drizzle
    .find_related(baker::Entity)
    .order_by_asc(baker::Column::Name)
    .all(&db)
    .await?;
let names: Vec<&str> = bakers.iter().map(|baker| baker.name.as_str()).collect();
assert_eq!(names, ["Ada", "Bo"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The cake's `Related<baker::Entity>` names the junction in its `via`, so
`find_related` joins through it:

```rust
# use pgorm_guide_support::{baker, cake};
use pgorm::entity::prelude::*;

let lemon_drizzle = cake::Model {
    id: 2,
    name: "Lemon Drizzle".to_owned(),
    flavor: cake::Flavor::Lemon,
    price: Decimal::new(320, 2),
    gluten_free: true,
    bakery_id: 1,
};
assert_eq!(
    lemon_drizzle.find_related(baker::Entity).build().0,
    r#"SELECT "baker"."id", "baker"."name", "baker"."hired_on", "baker"."bakery_id" FROM "baker" INNER JOIN "cake_baker" ON "cake_baker"."baker_id" = "baker"."id" INNER JOIN "cake" ON "cake"."id" = "cake_baker"."cake_id" WHERE "cake"."id" = $1"#
);
```

## Longer paths

A path of more than one relation is a [`Linked`](crate::Linked) chain: a type
listing the relations from one entity to another, in order.
[`find_linked`](crate::ModelTrait::find_linked) reads along it. Here a
baker's customers are the people who ordered a cake the baker bakes:

```rust
# use pgorm_guide_support::{baker, cake, cake_baker, customer, order, order_line};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("relations_linked").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

/// From a baker to the customers who ordered what they bake.
struct BakedFor;

impl Linked for BakedFor {
    type FromEntity = baker::Entity;
    type ToEntity = customer::Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![
            cake_baker::Relation::Baker.def().rev(),
            cake_baker::Relation::Cake.def(),
            order_line::Relation::Cake.def().rev(),
            order_line::Relation::Order.def(),
            order::Relation::Customer.def(),
        ]
    }
}

let cy = baker::Entity::find_by_id(3).one(&db).await?;
let customers = cy.find_linked(BakedFor).all(&db).await?;
let names: Vec<&str> = customers.iter().map(|c| c.name.as_str()).collect();
assert_eq!(names, ["Lee"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

Each hop joins its table under an alias of its own, `r0`, `r1` and so on, so
a chain may pass through one table twice. A filter or an ordering on a
hop's columns names that alias; [`LinkedAlias::hop`](crate::LinkedAlias::hop)
mints it, and [`last_hop_alias`](crate::Linked::last_hop_alias) gives the
source table's. A path that a `Related` impl already describes needs no
`Linked` type of its own: [`RelatedLink`](crate::RelatedLink) is that chain.

## Loading related rows for many models

Reading each model's related rows with `find_related` runs one query per
model. The loaders read them for a whole list in one query, and return one
entry per input model, in the input's order:

- [`load_one`](crate::LoaderTrait::load_one) follows a relation to at most one
  row, such as a cake's bakery, giving an `Option` per model;
- [`load_many`](crate::LoaderTrait::load_many) follows a relation to many
  rows, such as a bakery's cakes, giving a `Vec` per model;
- [`load_many_via`](crate::LoaderTrait::load_many_via) follows a relation
  through a junction table, such as a cake's bakers.

Each takes the related entity, or a `Select` of it to filter and order what
is loaded:

```rust
# use pgorm_guide_support::{baker, bakery, cake};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("relations_loaders").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

let bakeries = bakery::Entity::find()
    .order_by_asc(bakery::Column::Id)
    .all(&db)
    .await?;

// Each bakery's gluten-free cakes, cheapest first.
let cakes = bakeries
    .load_many(
        cake::Entity::find()
            .filter(cake::Column::GlutenFree.eq(true))
            .order_by_asc(cake::Column::Price),
        &db,
    )
    .await?;
let names: Vec<Vec<&str>> = cakes
    .iter()
    .map(|cakes| cakes.iter().map(|cake| cake.name.as_str()).collect())
    .collect();
assert_eq!(names, [vec!["Lemon Drizzle", "Chocolate Torte"], vec!["Vanilla Sponge"]]);

// Each of Sunrise's cakes, with its bakery and its bakers.
let sunrise_cakes = &cakes[0];
let their_bakery = sunrise_cakes.load_one(bakery::Entity, &db).await?;
assert!(their_bakery.iter().all(|b| b.as_ref().map(|b| b.name.as_str()) == Some("Sunrise")));

let bakers = sunrise_cakes
    .load_many_via(baker::Entity::find().order_by_asc(baker::Column::Name), &db)
    .await?;
let names: Vec<Vec<&str>> = bakers
    .iter()
    .map(|bakers| bakers.iter().map(|baker| baker.name.as_str()).collect())
    .collect();
assert_eq!(names, [vec!["Ada", "Bo"], vec!["Bo"]]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A loader reads the relation whole, its `on_condition` included, and an
ordering on its `Select` orders every model's list. Two things are errors
rather than silent answers: two different rows for one model under
`load_one`, which contradicts a relation to at most one row, and a related
row whose key matches in SQL but not as a Rust value, such as an `i64`
column read against an `i32` field.

## Reading rows with their related rows

The graph reads rows of one entity, the *root*, with rows of related
entities joined beside them, in one query. Start it with
[`graph()`](crate::EntityTrait::graph) where you would write `find()`, and add
each related entity as a *slot*. A slot is required or optional:

| Method | Joins | Decodes as |
| --- | --- | --- |
| [`related_one::<F>()`](crate::SelectGraph::related_one) | the path `Related<F>` describes, `INNER JOIN` | `F::Model` |
| [`related_maybe::<F>()`](crate::SelectGraph::related_maybe) | the same path, `LEFT JOIN` | `Option<F::Model>` |
| [`join_one::<F>(rel)`](crate::SelectGraph::join_one) | one relation, `INNER JOIN` | `F::Model` |
| [`join_maybe::<F>(rel)`](crate::SelectGraph::join_maybe) | one relation, `LEFT JOIN` | `Option<F::Model>` |

Each row is a tuple of the root's model and one item per slot, in the order
the slots were added. The compiler knows the tuple's type from the slots, so
a mismatch is a compile error rather than a decoding failure at run time.

```rust
# use pgorm_guide_support::{bakery, customer, order};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("relations_graph").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

let orders: Vec<(order::Model, customer::Model, bakery::Model)> = order::Entity::graph()
    .related_one::<customer::Entity>()
    .related_one::<bakery::Entity>()
    .order_by_asc(order::Column::PlacedAt)
    .all(&db)
    .await?;

let summary: Vec<(i32, &str, &str)> = orders
    .iter()
    .map(|(order, customer, bakery)| (order.id, customer.name.as_str(), bakery.name.as_str()))
    .collect();
assert_eq!(summary, [(1, "Kim", "Sunrise"), (2, "Lee", "Moonlight")]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A required slot leaves out a root row with nothing to join; an optional one
keeps it and decodes `None`. A graph with one optional slot can also be read
grouped: [`all_grouped`](crate::SelectGraph::all_grouped) returns each root
once, with its related rows collected in a `Vec`, an empty one when nothing
matched. It orders by your ordering first and the root's primary key after
it, so your ordering decides the order of both the roots and each list.

```rust
# use pgorm_guide_support::{bakery, cake};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("relations_grouped").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

// A third bakery, which sells nothing yet.
bakery::ActiveModel {
    name: set("Noon"),
    city: set("Tromsø"),
    ..Default::default()
}
.insert(&db)
.await?;

let rows: Vec<(bakery::Model, Option<cake::Model>)> = bakery::Entity::graph()
    .related_maybe::<cake::Entity>()
    .filter(bakery::Column::Name.ne("Sunrise"))
    .order_by_asc(bakery::Column::Id)
    .order_by_asc(cake::Column::Name)
    .all(&db)
    .await?;
let flat: Vec<(&str, Option<&str>)> = rows
    .iter()
    .map(|(bakery, cake)| (bakery.name.as_str(), cake.as_ref().map(|c| c.name.as_str())))
    .collect();
assert_eq!(
    flat,
    [("Moonlight", Some("Carrot Cake")), ("Moonlight", Some("Vanilla Sponge")), ("Noon", None)]
);

let grouped: Vec<(bakery::Model, Vec<cake::Model>)> = bakery::Entity::graph()
    .related_maybe::<cake::Entity>()
    .order_by_asc(cake::Column::Price)
    .all_grouped(&db)
    .await?;
let menu: Vec<(&str, usize)> = grouped
    .iter()
    .map(|(bakery, cakes)| (bakery.name.as_str(), cakes.len()))
    .collect();
assert_eq!(menu, [("Moonlight", 2), ("Sunrise", 3), ("Noon", 0)]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The roots come in the order of their first row. Ordering by price put
Moonlight's Vanilla Sponge first, so Moonlight comes first; Noon has no row
with a price, and PostgreSQL sorts its `NULL` last.

A graph has no `one`, because one root can make several rows;
[`one_opt`](crate::SelectGraph::one_opt) reads the first row.
[`stream`](crate::SelectGraph::stream) reads the rows one at a time, as
[Pages, cursors and streams](crate::guide::pagination) describes for `Select`.

### Junctions and other paths

[`via(rel)`](crate::SelectGraph::via) joins a table the path runs through
without decoding it, such as a junction. `related_maybe` and `related_one`
add a junction's hop themselves when the `Related` impl names one; written by
hand, the same many-to-many read is a `via` and a slot:

```rust
# use pgorm_guide_support::{baker, cake, cake_baker};
use pgorm::entity::prelude::*;

let by_hand = cake::Entity::graph()
    .via(cake_baker::Relation::Cake.def().rev())
    .join_maybe::<baker::Entity>(cake_baker::Relation::Baker.def());
let described = cake::Entity::graph().related_maybe::<baker::Entity>();
assert_eq!(by_hand.build(), described.build());
assert_eq!(
    described.build().0,
    r#"SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name", CAST("cake"."flavor" AS text) AS "s0_flavor", "cake"."price" AS "s0_price", "cake"."gluten_free" AS "s0_gluten_free", "cake"."bakery_id" AS "s0_bakery_id", "baker"."id" AS "s1_id", "baker"."name" AS "s1_name", "baker"."hired_on" AS "s1_hired_on", "baker"."bakery_id" AS "s1_bakery_id" FROM "cake" LEFT JOIN "cake_baker" ON "cake"."id" = "cake_baker"."cake_id" LEFT JOIN "baker" ON "cake_baker"."baker_id" = "baker"."id""#
);
```

The graph names its result columns `s0_` for the root and `s1_`, `s2_` and on
for each slot. A graph holds up to six slots; `via` hops do not count.

A path that passes through one table twice binds the second visit to an
alias: [`via_as`](crate::SelectGraph::via_as) and
[`join_maybe_as`](crate::SelectGraph::join_maybe_as) /
[`join_one_as`](crate::SelectGraph::join_one_as) are `via` and the two joins
with an alias, and [`from_alias`](crate::RelationDef::from_alias) makes the
next relation start from it. Here each cake reads the other cakes one of its
bakers bakes: from the cake through `cake_baker` to the baker, and back
through `cake_baker` a second time, under an alias, to the other cakes:

```rust
# use pgorm_guide_support::{cake, cake_baker};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("relations_aliases").await;
# let db = scratch.pool.get().await?;
use pgorm::{alias, entity::prelude::*};

let shared = alias("shared");
let sibling = alias("sibling");
let siblings: Vec<(cake::Model, Vec<cake::Model>)> = cake::Entity::graph()
    .via(cake_baker::Relation::Cake.def().rev())
    .via(cake_baker::Relation::Baker.def())
    .via_as(cake_baker::Relation::Baker.def().rev(), shared)
    .join_maybe_as::<cake::Entity>(cake_baker::Relation::Cake.def().from_alias(shared), sibling)
    .filter(Expr::col((sibling, cake::Column::Id)).ne(Expr::col((cake::Entity, cake::Column::Id))))
    .order_by_asc(Expr::col((sibling, cake::Column::Name)))
    .all_grouped(&db)
    .await?;

let lemon_drizzle = siblings.iter().find(|(cake, _)| cake.id == 2).map(|(_, s)| s);
let names: Vec<&str> = lemon_drizzle.into_iter().flatten().map(|c| c.name.as_str()).collect();
assert_eq!(names, ["Chocolate Fudge", "Chocolate Torte"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A filter or an ordering on an aliased table names the alias, as above.

### Narrowing a join

A `filter` lands in the query's `WHERE`, where it applies to the joined row
as a whole: under a `LEFT JOIN`, a root whose slot did not match has `NULL`s
there, fails the predicate, and drops out, as if the join were `INNER`. To
narrow which rows *match* instead, keeping the unmatched roots,
[`join_maybe_filtered`](crate::SelectGraph::join_maybe_filtered) adds the
predicate to the join's `ON`. Its closure receives the join's two table names,
so it qualifies its columns through them:

```rust
# use pgorm_guide_support::{bakery, cake};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("relations_filtered").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

// Every bakery, with its carrot cakes if it has any.
let carrot: Vec<(bakery::Model, Vec<cake::Model>)> = bakery::Entity::graph()
    .join_maybe_filtered::<cake::Entity, _>(bakery::Relation::Cake.def(), |_bakery, cake| {
        Condition::all().add(Expr::col((cake, cake::Column::Flavor)).eq(cake::Flavor::Carrot))
    })
    .all_grouped(&db)
    .await?;
let menu: Vec<(&str, usize)> = carrot.iter().map(|(b, cakes)| (b.name.as_str(), cakes.len())).collect();
assert_eq!(menu, [("Sunrise", 0), ("Moonlight", 1)]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

### Limits, pages and cursors

[`limit`](crate::SelectGraph::limit) and [`offset`](crate::SelectGraph::offset)
count the rows of the join, not the roots: a bakery with three cakes spends
three rows of a limit. A limited graph therefore has no `all_grouped`, whose
last entry could hold only part of its root's rows, and no `paginate`,
`count` or cursor, which set the rows themselves. The compiler refuses them:

```rust,compile_fail,E0599
# use pgorm_guide_support::{bakery, cake};
# async fn first_ten(db: &pgorm::DatabaseConnection) -> Result<(), pgorm::Error> {
use pgorm::entity::prelude::*;

let first_ten = bakery::Entity::graph()
    .related_maybe::<cake::Entity>()
    .limit(10)
    .all_grouped(db)
    .await?;
# Ok(())
# }
```

To read the first `n` roots with all of their related rows, limit the roots
and load the rest:

```rust
# use pgorm_guide_support::{bakery, cake};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("relations_first_roots").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

let first = bakery::Entity::find()
    .order_by_asc(bakery::Column::Id)
    .limit(1)
    .all(&db)
    .await?;
let cakes = first.load_many(cake::Entity, &db).await?;
assert_eq!((first.len(), cakes[0].len()), (1, 3));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

An unlimited graph pages as a `Select` does, by rows of the join, through
[`paginate`](crate::PaginatorTrait::paginate) and
[`cursor_by`](crate::SelectGraph::cursor_by).
[`cursor_by_on::<I, _>(columns)`](crate::SelectGraph::cursor_by_on) orders by
the columns of slot `I`, counting the first slot as `1`. A graph cursor
breaks ties by the primary key of every source it decodes, so rows that share
an ordering value still page correctly. [Pages, cursors and
streams](crate::guide::pagination) covers both.

```rust
# use pgorm_guide_support::{bakery, cake};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("relations_graph_cursor").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

// Every cake with its bakery, two rows at a time, cheapest first.
let mut cursor = cake::Entity::graph()
    .related_one::<bakery::Entity>()
    .cursor_by(cake::Column::Price);
let page: Vec<(cake::Model, bakery::Model)> = cursor.first(2).all(&db).await?;
let names: Vec<(&str, &str)> = page.iter().map(|(c, b)| (c.name.as_str(), b.name.as_str())).collect();
assert_eq!(names, [("Vanilla Sponge", "Moonlight"), ("Lemon Drizzle", "Sunrise")]);

let last_price = page[1].0.price;
let next: Vec<(cake::Model, bakery::Model)> = cursor.after(last_price).first(2).all(&db).await?;
let names: Vec<&str> = next.iter().map(|(c, _)| c.name.as_str()).collect();
assert_eq!(names, ["Carrot Cake", "Chocolate Fudge"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```
