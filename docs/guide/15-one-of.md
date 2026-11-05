# One-of references

A review at the bakery is of a cake or of a bakery, never both and never
neither. Rust says that with an enum. PostgreSQL says it with one nullable
foreign-key column per target, `cake_id` and `bakery_id`, and a `CHECK` that
exactly one of them is set. Each column is an ordinary foreign key, so the
database still knows what every review points at, and the `CHECK` keeps a row
from pointing at two things.

The [`OneOf`](derive@crate::OneOf) derive declares such a reference over the
entity holding it, and gives you three things: a row's reference read as an
enum, a reference set on an `ActiveModel` with the other columns cleared, and
the targets of many rows loaded with one query per target table.
[`Schema::create_one_of`](crate::Schema::create_one_of) writes the `CHECK`.

## Declaring one

The entity holding the reference has a nullable column for each target and a
`belongs_to` relation along it. The enum names the entity, and each variant
names a target entity and the column holding its key. A unit variant, if
there is one, is a row that references none of them:

```rust
pub mod review {
    use pgorm::entity::prelude::*;
    use pgorm_guide_support::{bakery, cake};

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "review")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub stars: i16,
        pub body: String,
        pub cake_id: Option<i32>,
        pub bakery_id: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(belongs_to = cake::Entity, from = Column::CakeId, to = cake::Column::Id)]
        Cake,
        #[pgorm(belongs_to = bakery::Entity, from = Column::BakeryId, to = bakery::Column::Id)]
        Bakery,
    }

    #[derive(Clone, Copy, Debug, OneOf)]
    #[pgorm(entity = Entity, model_attrs(derive(PartialEq, Eq)))]
    pub enum Subject {
        #[pgorm(column = Column::CakeId)]
        Cake(cake::Entity),
        #[pgorm(column = Column::BakeryId)]
        Bakery(bakery::Entity),
    }
}

# fn main() {
use pgorm::entity::prelude::*;
use review::{Subject, SubjectKey};

assert_eq!(Subject::name(), "subject");
let columns: Vec<String> = Subject::columns().iter().map(|column| column.to_string()).collect();
assert_eq!(columns, ["cake_id", "bakery_id"]);

let on_fudge = review::Model {
    id: 1,
    stars: 5,
    body: "Rich and dark".to_owned(),
    cake_id: Some(1),
    bakery_id: None,
};
assert_eq!(Subject::key(&on_fudge).ok(), Some(SubjectKey::Cake(1)));
# }
```

The derive writes two enums beside it:

- `SubjectKey`, each variant holding its target's key, `SubjectKey::Cake(1)`;
- `SubjectModel`, each variant holding its target's `Model`.

`key_attrs(..)` and `model_attrs(..)` give them further attributes. What does
not fit the shape does not compile: a column that is not nullable or is not
of its target's key type, a target the entity has no `Related` relation to,
two variants naming one target, or more than one unit variant.

## The constraint

`create_table_from_entity` writes the foreign keys, since they are the
relations' own. [`Schema::create_one_of`](crate::Schema::create_one_of)
writes the `CHECK` beside them, `num_nonnulls(..) = 1`, or `<= 1` where the
enum has a unit variant. It is named `ck-{table}-{enum}`:

```rust
# pub mod review {
#     use pgorm::entity::prelude::*;
#     use pgorm_guide_support::{bakery, cake};
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "review")]
#     pub struct Model {
#         #[pgorm(primary_key)]
#         pub id: i32,
#         pub stars: i16,
#         pub body: String,
#         pub cake_id: Option<i32>,
#         pub bakery_id: Option<i32>,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {
#         #[pgorm(belongs_to = cake::Entity, from = Column::CakeId, to = cake::Column::Id)]
#         Cake,
#         #[pgorm(belongs_to = bakery::Entity, from = Column::BakeryId, to = bakery::Column::Id)]
#         Bakery,
#     }
#     #[derive(Clone, Copy, Debug, OneOf)]
#     #[pgorm(entity = Entity, model_attrs(derive(PartialEq, Eq)))]
#     pub enum Subject {
#         #[pgorm(column = Column::CakeId)]
#         Cake(cake::Entity),
#         #[pgorm(column = Column::BakeryId)]
#         Bakery(bakery::Entity),
#     }
# }
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("one_of_check").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;
use pgorm::Schema;
use review::Subject;

let schema = Schema::new();
db.batch_execute(&schema.create_table_from_entity(review::Entity)?.to_string()).await?;

let check = schema.create_one_of::<Subject>();
assert_eq!(
    check.to_string(),
    r#"ALTER TABLE "review" ADD CONSTRAINT "ck-review-subject" CHECK (num_nonnulls("cake_id", "bakery_id") = 1)"#
);
db.batch_execute(&check.to_string()).await?;

// A review of a cake and a bakery at once is refused.
let both = review::ActiveModel {
    stars: set(3_i16),
    body: set("Both?"),
    cake_id: set(Some(1)),
    bakery_id: set(Some(1)),
    ..Default::default()
}
.insert(&db)
.await;
assert!(matches!(both.unwrap_err().sql_error(), Some(SqlError::CheckConstraintViolation(_))));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Reading and writing

[`OneOf::key`](crate::OneOf::key) reads a row's reference as its
`SubjectKey`. [`OneOf::set`](crate::OneOf::set) sets one on an
`ActiveModel`: the variant's column to its key and every other column to
`NULL`, so moving a review from a cake to a bakery is one call.
[`OneOf::load`](crate::OneOf::load) takes a list of rows and returns each
row's `SubjectModel`, in order, reading each target table once, with one
query for the keys that table's variant holds:

```rust
# pub mod review {
#     use pgorm::entity::prelude::*;
#     use pgorm_guide_support::{bakery, cake};
#     #[derive(Clone, Debug, PartialEq, Eq, Entity)]
#     #[pgorm(table_name = "review")]
#     pub struct Model {
#         #[pgorm(primary_key)]
#         pub id: i32,
#         pub stars: i16,
#         pub body: String,
#         pub cake_id: Option<i32>,
#         pub bakery_id: Option<i32>,
#     }
#     #[derive(Copy, Clone, Debug, Relation)]
#     pub enum Relation {
#         #[pgorm(belongs_to = cake::Entity, from = Column::CakeId, to = cake::Column::Id)]
#         Cake,
#         #[pgorm(belongs_to = bakery::Entity, from = Column::BakeryId, to = bakery::Column::Id)]
#         Bakery,
#     }
#     #[derive(Clone, Copy, Debug, OneOf)]
#     #[pgorm(entity = Entity, model_attrs(derive(PartialEq, Eq)))]
#     pub enum Subject {
#         #[pgorm(column = Column::CakeId)]
#         Cake(cake::Entity),
#         #[pgorm(column = Column::BakeryId)]
#         Bakery(bakery::Entity),
#     }
# }
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("one_of_read_write").await;
# let db = scratch.pool.get().await?;
# let schema = pgorm::Schema::new();
# db.batch_execute(&schema.create_table_from_entity(review::Entity)?.to_string()).await?;
# db.batch_execute(&schema.create_one_of::<review::Subject>().to_string()).await?;
use pgorm::entity::prelude::*;
use pgorm_guide_support::{bakery, cake};
use review::{Subject, SubjectKey, SubjectModel};

let write = |stars: i16, body: &str, subject: SubjectKey| -> Result<review::ActiveModel, Error> {
    let mut review = review::ActiveModel {
        stars: set(stars),
        body: set(body),
        ..Default::default()
    };
    Subject::set(&mut review, subject)?;
    Ok(review)
};
Insert::many([
    write(5, "Rich and dark", SubjectKey::Cake(1))?,
    write(4, "Friendly staff", SubjectKey::Bakery(2))?,
    write(3, "A little dry", SubjectKey::Cake(3))?,
])
.exec(&db)
.await?;

let reviews = review::Entity::find().order_by_asc(review::Column::Id).all(&db).await?;
let subjects = Subject::load(&reviews, &db).await?;
let named: Vec<String> = subjects
    .iter()
    .map(|subject| match subject {
        SubjectModel::Cake(cake) => format!("cake {}", cake.name),
        SubjectModel::Bakery(bakery) => format!("bakery {}", bakery.name),
    })
    .collect();
assert_eq!(named, ["cake Chocolate Fudge", "bakery Moonlight", "cake Carrot Cake"]);

// The third review was really about the bakery that sells the cake.
let mut moved = reviews[2].clone().into_active();
Subject::set(&mut moved, SubjectKey::Bakery(2))?;
let moved = moved.update(&db).await?;
assert_eq!((moved.cake_id, moved.bakery_id), (None, Some(2)));
assert_eq!(Subject::key(&moved)?, SubjectKey::Bakery(2));
# let _ = (bakery::Entity, cake::Entity);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A row the `CHECK` rules out can still be met in a database without it: a
review with both columns set, or with neither where the enum has no unit
variant. `key` and `load` report it as [`Error::Query`](crate::Error::Query),
naming the table, the row's key and the columns, rather than read it as one
of its targets. So does `load` for a key its target's table does not hold.

The targets of one row are also reachable the ordinary way: each column is a
relation, so `review.find_related(cake::Entity)` and the graph of
[Relations](crate::guide::relations) read them too. `OneOf` is for when the program
wants the one target a row has, whichever it is.

## Generated entities

[pgorm-codegen](crate::guide::codegen) recognises the shape in a schema: nullable
foreign-key columns covered by a `CHECK (num_nonnulls(..) = 1)` or `<= 1`
generate a `OneOf` enum, named for the constraint, beside the entity.
