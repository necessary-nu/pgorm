#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, bakery_chain::*, setup::*};
use pgorm::{
    Error, PartialModelTrait, PrimaryKeyTrait, QueryResult, Related, RelationDef, RelationTrait,
    Schema, Select, Value, entity::prelude::*,
};
use pgorm_query::{
    Expr, IntoName, IntoValueTuple, QueryBuilder, TableName, TryFromValueTuple, ValueTupleError,
};
use pretty_assertions::assert_eq;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Ordinary derived entity: one auto-increment key, a text column, a nullable one.
mod item {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "item")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
        pub note: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Same shape but declaring a schema, for the `table_ref` qualification claim.
mod scoped_item {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "item", schema_name = "warehouse")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Composite (two-column) primary key.
mod pair {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "pair")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub left_id: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub right_id: i32,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Composite four-column primary key: the arity past every hand-written tuple
/// impl, so it exercises the one representation a wide key reaches.
mod quad {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "quad")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub a: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub b: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub c: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub d: i32,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// An entity whose key is written by hand, which the traits allow: its
/// `Entity` names the key through the `EntityTrait` derive's `primary_key` override. It
/// deliberately declares a `ValueType` with more components than the key has
/// variants, which is what the `find_by_id` / `delete_by_id` arity guard exists
/// to catch; no derive writes such a key.
mod too_many_values {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Default, Debug, EntityTrait)]
    #[pgorm(primary_key = WideKey)]
    pub struct Entity;

    impl EntityName for Entity {
        fn table_name(&self) -> &str {
            "too_many_values"
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id_1: i32,
        pub id_2: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    #[derive(Copy, Clone, Debug, EnumIter)]
    pub enum WideKey {
        Id1,
    }

    impl SqlName for WideKey {
        fn unquoted(&self, s: &mut dyn std::fmt::Write) {
            s.write_str(self.as_str()).expect("write to sql sink");
        }
    }

    impl StaticName for WideKey {
        fn as_str(&self) -> &str {
            match self {
                Self::Id1 => "id_1",
            }
        }
    }

    impl PrimaryKeyToColumn for WideKey {
        type Column = Column;

        fn into_column(self) -> Column {
            match self {
                Self::Id1 => Column::Id1,
            }
        }

        fn from_column(col: Column) -> Option<Self> {
            match col {
                Column::Id1 => Some(Self::Id1),
                Column::Id2 => None,
            }
        }
    }

    impl PrimaryKeyTrait for WideKey {
        type ValueType = (i32, i32);

        fn auto_increment() -> bool {
            false
        }
    }
}

/// The mirror image: two key columns but a single-component `ValueType`.
mod too_few_values {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Default, Debug, EntityTrait)]
    #[pgorm(primary_key = NarrowKey)]
    pub struct Entity;

    impl EntityName for Entity {
        fn table_name(&self) -> &str {
            "too_few_values"
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id_1: i32,
        #[pgorm(primary_key)]
        pub id_2: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    #[derive(Copy, Clone, Debug, EnumIter)]
    pub enum NarrowKey {
        Id1,
        Id2,
    }

    impl SqlName for NarrowKey {
        fn unquoted(&self, s: &mut dyn std::fmt::Write) {
            s.write_str(self.as_str()).expect("write to sql sink");
        }
    }

    impl StaticName for NarrowKey {
        fn as_str(&self) -> &str {
            match self {
                Self::Id1 => "id_1",
                Self::Id2 => "id_2",
            }
        }
    }

    impl PrimaryKeyToColumn for NarrowKey {
        type Column = Column;

        fn into_column(self) -> Column {
            match self {
                Self::Id1 => Column::Id1,
                Self::Id2 => Column::Id2,
            }
        }

        fn from_column(col: Column) -> Option<Self> {
            match col {
                Column::Id1 => Some(Self::Id1),
                Column::Id2 => Some(Self::Id2),
            }
        }
    }

    impl PrimaryKeyTrait for NarrowKey {
        type ValueType = i32;

        fn auto_increment() -> bool {
            false
        }
    }
}

/// One key column of type `i32`, but a `ValueType` declaring `String`. The
/// arities agree, so `get_primary_key_value` builds a well-shaped tuple and the
/// element type is what disagrees wherever the declared key type is consulted.
mod mistyped_key {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Default, Debug, EntityTrait)]
    #[pgorm(primary_key = MistypedKey)]
    pub struct Entity;

    impl EntityName for Entity {
        fn table_name(&self) -> &str {
            "mistyped_key"
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    #[derive(Copy, Clone, Debug, EnumIter)]
    pub enum MistypedKey {
        Id,
    }

    impl SqlName for MistypedKey {
        fn unquoted(&self, s: &mut dyn std::fmt::Write) {
            s.write_str(self.as_str()).expect("write to sql sink");
        }
    }

    impl StaticName for MistypedKey {
        fn as_str(&self) -> &str {
            match self {
                Self::Id => "id",
            }
        }
    }

    impl PrimaryKeyToColumn for MistypedKey {
        type Column = Column;

        fn into_column(self) -> Column {
            match self {
                Self::Id => Column::Id,
            }
        }

        fn from_column(col: Column) -> Option<Self> {
            match col {
                Column::Id => Some(Self::Id),
            }
        }
    }

    impl PrimaryKeyTrait for MistypedKey {
        type ValueType = String;

        fn auto_increment() -> bool {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// entity.traits — the trait family
// ---------------------------------------------------------------------------

/// Every associated type of a concrete entity really does satisfy the bounds
/// `EntityTrait` declares. This only compiles if the five associated types are
/// wired the way the spec says.
fn assert_entity_family<E>()
where
    E: EntityTrait + EntityName + StaticName + Default,
    E::Model: ModelTrait<Entity = E> + FromQueryResult,
    E::ActiveModel: ActiveModelBehavior<Entity = E>,
    E::Column: ColumnTrait,
    E::Relation: RelationTrait,
    E::PrimaryKey: PrimaryKeyTrait + PrimaryKeyToColumn<Column = E::Column>,
{
}

/// Build an entity the way the CRUD entry points do — through the `Default`
/// bound `EntityName` carries, with nothing else in scope.
fn via_default<E: Default>() -> E {
    E::default()
}

// [spec:pgorm:def:entity.traits+4/test]    `StaticName` as the base identifier
// contract (`as_str` alongside `SqlName`'s quoting), `EntityName: StaticName +
// Default`, and `EntityTrait`'s five associated types resolving for both a
// derived entity and one whose name and key are written by hand
#[test]
fn entity_trait_family() {
    // The five associated types satisfy their declared bounds — for a derived
    // entity and for one whose key is written by hand.
    assert_entity_family::<item::Entity>();
    assert_entity_family::<pair::Entity>();
    assert_entity_family::<too_many_values::Entity>();

    // `StaticName::as_str` is the static-string identity, on entities...
    assert_eq!(item::Entity.as_str(), "item");
    assert_eq!(too_many_values::Entity.as_str(), "too_many_values");
    // ...and on columns and primary keys, which are `StaticName` too.
    assert_eq!(item::Column::Id.as_str(), "id");
    assert_eq!(item::Column::Note.as_str(), "note");
    assert_eq!(item::PrimaryKey::Id.as_str(), "id");

    // `StaticName: SqlName`, so the same identifier renders through `to_string`.
    assert_eq!(pgorm::SqlName::to_string(&item::Entity), "item");
    assert_eq!(pgorm::SqlName::to_string(&item::Column::Name), "name");

    // `EntityName: Default` — the CRUD entry points construct `Self::default()`.
    assert_eq!(via_default::<item::Entity>().table_name(), "item");
    assert_eq!(
        via_default::<too_many_values::Entity>().table_name(),
        "too_many_values"
    );

    // `StaticName: Copy`, so passing an entity by value does not move it away.
    let entity = item::Entity;
    let copied = entity;
    assert_eq!(entity.as_str(), copied.as_str());
}

// [spec:pgorm:req:entity.traits.entity-name+4/test]    `table_name` is the only
// required method; `schema_name` and `comment` default to `None` and
// `module_name` to `table_name()`; `table_ref` yields a bare `TableName::Table`
// without a schema and a `SchemaTable` with one, and every statement that names
// the table is qualified as a result
#[test]
fn entity_name_defaults_and_table_ref() {
    // Only `table_name` was supplied, so the other three take their defaults.
    assert_eq!(item::Entity.table_name(), "item");
    assert_eq!(item::Entity.schema_name(), None);
    assert_eq!(item::Entity.comment(), None);
    assert_eq!(item::Entity.module_name(), item::Entity.table_name());

    // Without a schema, `table_ref` is a bare table reference.
    assert!(
        matches!(item::Entity.table_ref(), TableName::Table(iden) if iden.to_string() == "item")
    );

    // With one, it is schema-qualified — note both entities share `table_name`.
    assert_eq!(scoped_item::Entity.table_name(), "item");
    assert_eq!(scoped_item::Entity.schema_name(), Some("warehouse"));
    assert!(matches!(
        scoped_item::Entity.table_ref(),
        TableName::SchemaTable(schema, table)
            if schema.to_string() == "warehouse" && table.to_string() == "item"
    ));

    // And because all generated SQL goes through `table_ref`, the qualification
    // shows up in every statement kind.
    assert_eq!(
        scoped_item::Entity::find().build().0,
        r#"SELECT "item"."id", "item"."name" FROM "warehouse"."item""#
    );
    assert_eq!(
        Insert::one(scoped_item::ActiveModel {
            id: NotSet,
            name: set("x"),
        })
        .build()
        .0,
        r#"INSERT INTO "warehouse"."item" ("name") VALUES ($1)"#
    );
    assert_eq!(
        Update::one(scoped_item::ActiveModel {
            id: ActiveValue::Unchanged(1),
            name: set("x"),
        })
        .expect("the primary key is unchanged, not unset")
        .build()
        .0,
        r#"UPDATE "warehouse"."item" SET "name" = $1 WHERE "item"."id" = $2"#
    );
    assert_eq!(
        scoped_item::Entity::delete_by_id(1).build().0,
        r#"DELETE FROM "warehouse"."item" WHERE "item"."id" = $1"#
    );

    // The unqualified entity stays unqualified.
    assert_eq!(
        item::Entity::find().build().0,
        r#"SELECT "item"."id", "item"."name", "item"."note" FROM "item""#
    );
}

// ---------------------------------------------------------------------------
// entity.traits.crud
// ---------------------------------------------------------------------------

// [spec:pgorm:req:entity.traits.crud+4/test]    the static CRUD surface: `find`
// returns a fresh `Select`, `find_by_id` adds one equality filter per key column
// in primary-key iteration order, and `insert` / `insert_many` / `update` /
// `update_many` / `delete` / `delete_many` / `delete_by_id` build their
// respective statements
#[test]
fn entity_crud_surface() {
    // `find()` selects every column and is fresh each call.
    assert_eq!(
        item::Entity::find().build().0,
        r#"SELECT "item"."id", "item"."name", "item"."note" FROM "item""#
    );
    let filtered = item::Entity::find().filter(item::Column::Id.eq(1));
    assert_eq!(
        item::Entity::find().build().0,
        r#"SELECT "item"."id", "item"."name", "item"."note" FROM "item""#,
        "find() must not accumulate state across calls"
    );
    assert_eq!(
        filtered.build().0,
        r#"SELECT "item"."id", "item"."name", "item"."note" FROM "item" WHERE "item"."id" = $1"#
    );

    // `find_by_id` builds on `find()` with one equality filter per key column.
    assert_eq!(
        item::Entity::find_by_id(11).build().0,
        r#"SELECT "item"."id", "item"."name", "item"."note" FROM "item" WHERE "item"."id" = $1"#
    );
    // A composite key consumes the tuple in primary-key iteration order:
    // `LeftId` first, then `RightId`.
    assert_eq!(
        pair::Entity::find_by_id((2, 3)).build().0,
        [
            r#"SELECT "pair"."left_id", "pair"."right_id", "pair"."label" FROM "pair""#,
            r#"WHERE "pair"."left_id" = $1 AND "pair"."right_id" = $2"#,
        ]
        .join(" ")
    );
    // A four-column key walks the same one filter per column, with no arity
    // past which the key changes representation.
    assert_eq!(
        quad::Entity::find_by_id((2, 3, 4, 5)).build().0,
        [
            r#"SELECT "quad"."a", "quad"."b", "quad"."c", "quad"."d", "quad"."label""#,
            r#"FROM "quad" WHERE "quad"."a" = $1 AND "quad"."b" = $2"#,
            r#"AND "quad"."c" = $3 AND "quad"."d" = $4"#,
        ]
        .join(" ")
    );
    // And the key round-trips back out of the tuple at that arity.
    assert_eq!(
        <(i32, i32, i32, i32)>::try_from_value_tuple((2, 3, 4, 5).into_value_tuple()),
        Ok((2, 3, 4, 5))
    );

    // `insert` is `Insert::one`.
    assert_eq!(
        Insert::one(item::ActiveModel {
            id: NotSet,
            name: set("Apple"),
            note: NotSet,
        })
        .build()
        .0,
        r#"INSERT INTO "item" ("name") VALUES ($1)"#
    );

    // `insert_many` is `Insert::many`.
    assert_eq!(
        Insert::many([
            item::ActiveModel {
                id: NotSet,
                name: set("Apple"),
                note: NotSet,
            },
            item::ActiveModel {
                id: NotSet,
                name: set("Pear"),
                note: NotSet,
            },
        ])
        .build()
        .0,
        r#"INSERT INTO "item" ("name") VALUES ($1), ($2)"#
    );

    // `update` is an `UpdateOne` keyed on the primary key.
    assert_eq!(
        Update::one(item::ActiveModel {
            id: ActiveValue::Unchanged(1),
            name: set("Apple"),
            note: NotSet,
        })
        .expect("the primary key is unchanged, not unset")
        .build()
        .0,
        r#"UPDATE "item" SET "name" = $1 WHERE "item"."id" = $2"#
    );

    // `update_many` is an `UpdateMany`: no key filter, only what you add.
    assert_eq!(
        Update::many(item::Entity)
            .col_expr(item::Column::Name, Expr::value("Apple"))
            .filter(item::Column::Note.is_null())
            .build()
            .0,
        r#"UPDATE "item" SET "name" = $1 WHERE "item"."note" IS NULL"#
    );

    // `delete` is a `DeleteOne` keyed on the primary key.
    assert_eq!(
        Delete::one(item::ActiveModel {
            id: set(3),
            name: NotSet,
            note: NotSet,
        })
        .expect("the primary key is set")
        .build()
        .0,
        r#"DELETE FROM "item" WHERE "item"."id" = $1"#
    );

    // `delete_many` is unfiltered until you filter it.
    assert_eq!(
        Delete::many(item::Entity)
            .filter(item::Column::Name.contains("Apple"))
            .build()
            .0,
        r#"DELETE FROM "item" WHERE "item"."name" LIKE $1"#
    );

    // `delete_by_id` filters per key column just like `find_by_id`.
    assert_eq!(
        item::Entity::delete_by_id(1).build().0,
        r#"DELETE FROM "item" WHERE "item"."id" = $1"#
    );
    assert_eq!(
        pair::Entity::delete_by_id((2, 3)).build().0,
        r#"DELETE FROM "pair" WHERE "pair"."left_id" = $1 AND "pair"."right_id" = $2"#
    );
}

// [spec:pgorm:req:entity.traits.crud+4/test]    `find_by_id` panics with
// `primary key arity mismatch` when more values arrive than the key has columns
#[test]
#[should_panic(expected = "primary key arity mismatch")]
fn find_by_id_panics_when_values_outnumber_key() {
    let _ = too_many_values::Entity::find_by_id((1, 2));
}

// [spec:pgorm:req:entity.traits.crud+4/test]    ...and in the other direction too,
// when the key has more columns than values were supplied
#[test]
#[should_panic(expected = "primary key arity mismatch")]
fn find_by_id_panics_when_key_outnumbers_values() {
    let _ = too_few_values::Entity::find_by_id(1);
}

// [spec:pgorm:req:entity.traits.crud+4/test]    `delete_by_id` carries the same
// guard as `find_by_id`, in both directions
#[test]
#[should_panic(expected = "primary key arity mismatch")]
fn delete_by_id_panics_when_values_outnumber_key() {
    let _ = too_many_values::Entity::delete_by_id((1, 2));
}

// [spec:pgorm:req:entity.traits.crud+4/test]
#[test]
#[should_panic(expected = "primary key arity mismatch")]
fn delete_by_id_panics_when_key_outnumbers_values() {
    let _ = too_few_values::Entity::delete_by_id(1);
}

// [spec:pgorm:def:sql.value.tuple+3/test]    the conversion the update-side
// `find_updated_model_by_id` runs a primary-key tuple through errs on an arity
// the entity's `ValueType` does not have, naming both lengths
#[test]
fn primary_key_value_type_errs_on_arity() {
    type ItemKey = <item::PrimaryKey as PrimaryKeyTrait>::ValueType;
    type PairKey = <pair::PrimaryKey as PrimaryKeyTrait>::ValueType;

    assert_eq!(
        ItemKey::try_from_value_tuple((1i32, 2i32).into_value_tuple()),
        Err(ValueTupleError::Arity {
            expected: 1,
            actual: 2,
        })
    );
    assert_eq!(
        PairKey::try_from_value_tuple(1i32.into_value_tuple()),
        Err(ValueTupleError::Arity {
            expected: 2,
            actual: 1,
        })
    );
    assert_eq!(
        PairKey::try_from_value_tuple((1i32, "b").into_value_tuple()),
        Err(ValueTupleError::Element {
            position: 1,
            expected: "i32".to_owned(),
        })
    );
}

// [spec:pgorm:sem:exec.crud.insert+6/test]    an insert whose `ValueType`
// disagrees with the key column fails decoding the `RETURNING` row, with
// `Error::UnpackInsertId` rather than a panic. There is no cached tuple to
// reconstruct from, so the `Error::Type` the reconstruction used to raise is not
// reachable here. Only `exec_returning_pk` decodes the key, so only it can fail
// this way; plain `exec` asks for no key and reports the row it wrote.
// [spec:pgorm:sem:exec.crud.update+7/test]    the no-op read reuses the
// statement's own `WHERE` and never consults `ValueType`, so even a mistyped
// key declaration re-reads the model successfully
// [spec:pgorm:req:exec.crud.exec-vocabulary+2/test]    the two insert terminals
// differ exactly as their names say: a count needs no key and cannot fail on one
#[pgorm_macros::test]
async fn mistyped_primary_key_errs_on_crud() -> Result<(), Error> {
    let ctx = TestContext::new("mistyped_primary_key").await;
    let db = ctx.db.get().await?;
    let stmt = Schema::new().create_table_from_entity(mistyped_key::Entity)?;
    db.execute(&stmt.to_string(), &[]).await?;

    let inserted = Insert::one(mistyped_key::ActiveModel { id: set(1) })
        .exec_returning_pk(&db)
        .await;
    assert_eq!(inserted.unwrap_err(), Error::UnpackInsertId);

    // `exec` decodes no key, so the mistyped `ValueType` is never consulted and
    // the insert simply reports the row it wrote.
    let counted = Insert::one(mistyped_key::ActiveModel {
        id: ActiveValue::Set(2),
    })
    .exec(&db)
    .await?;
    assert_eq!(counted, 1);

    let updated = Update::one(mistyped_key::ActiveModel {
        id: ActiveValue::Unchanged(1),
    })?
    .exec_returning_model(&db)
    .await?;
    assert_eq!(updated, mistyped_key::Model { id: 1 });

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:exec.crud.update+7/test]    a composite key's no-op read
// carries every key predicate and the caller's guard together
#[pgorm_macros::test]
async fn composite_key_noop_update_keeps_filters() -> Result<(), Error> {
    let ctx = TestContext::new("composite_noop_filters").await;
    let db = ctx.db.get().await?;
    let stmt = Schema::new().create_table_from_entity(pair::Entity)?;
    db.execute(&stmt.to_string(), &[]).await?;

    let row = Insert::one(pair::ActiveModel {
        left_id: set(1),
        right_id: set(2),
        label: set("kept"),
    })
    .exec_returning_model(&db)
    .await?;

    let unchanged = || pair::ActiveModel {
        left_id: ActiveValue::Unchanged(1),
        right_id: ActiveValue::Unchanged(2),
        label: ActiveValue::Unchanged("kept".to_owned()),
    };

    let matched = Update::one(unchanged())?
        .filter(pair::Column::Label.eq("kept"))
        .exec_returning_model(&db)
        .await?;
    assert_eq!(matched, row);

    let missed = Update::one(unchanged())?
        .filter(pair::Column::Label.eq("other"))
        .exec_returning_model(&db)
        .await;
    assert_eq!(missed.unwrap_err(), Error::RecordNotFound);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// entity.traits.column
// ---------------------------------------------------------------------------

fn sql(expr: pgorm_query::SimpleExpr) -> String {
    item::Entity::find().filter(expr).as_query().to_string()
}

const SELECT_ITEM: &str = r#"SELECT "item"."id", "item"."name", "item"."note" FROM "item" WHERE "#;

// [spec:pgorm:def:entity.traits.column+7/test]    the expression-building surface
// `ColumnTrait` wraps around `Expr`: comparisons, ranges, pattern matching and
// its sugar, aggregates, null checks, set membership and subqueries — plus
// `def`, `entity_name`, `as_column_ref`, `into_expr` and `into_returning_expr`
#[test]
fn column_trait_expression_surface() {
    // `def()` hands back the column's definition.
    assert_eq!(
        item::Column::Id.def().get_column_type(),
        &ColumnType::Integer
    );
    assert!(!item::Column::Name.def().is_null());
    assert!(item::Column::Note.def().is_null());

    // `entity_name` / `as_column_ref` qualify the column with its entity.
    assert_eq!(item::Column::Name.entity_name().to_string(), "item");
    let (entity, column) = item::Column::Name.as_column_ref();
    assert_eq!(entity.to_string(), "item");
    assert_eq!(column.to_string(), "name");

    // Comparison operators.
    assert_eq!(
        sql(item::Column::Id.eq(1)),
        format!(r#"{SELECT_ITEM}"item"."id" = 1"#)
    );
    assert_eq!(
        sql(item::Column::Id.ne(1)),
        format!(r#"{SELECT_ITEM}"item"."id" <> 1"#)
    );
    assert_eq!(
        sql(item::Column::Id.gt(1)),
        format!(r#"{SELECT_ITEM}"item"."id" > 1"#)
    );
    assert_eq!(
        sql(item::Column::Id.gte(1)),
        format!(r#"{SELECT_ITEM}"item"."id" >= 1"#)
    );
    assert_eq!(
        sql(item::Column::Id.lt(1)),
        format!(r#"{SELECT_ITEM}"item"."id" < 1"#)
    );
    assert_eq!(
        sql(item::Column::Id.lte(1)),
        format!(r#"{SELECT_ITEM}"item"."id" <= 1"#)
    );

    // Ranges.
    assert_eq!(
        sql(item::Column::Id.between(2, 3)),
        format!(r#"{SELECT_ITEM}"item"."id" BETWEEN 2 AND 3"#)
    );
    assert_eq!(
        sql(item::Column::Id.not_between(2, 3)),
        format!(r#"{SELECT_ITEM}"item"."id" NOT BETWEEN 2 AND 3"#)
    );

    // Pattern matching, and the three sugar forms that place the wildcards.
    assert_eq!(
        sql(item::Column::Name.like("cheese")),
        format!(r#"{SELECT_ITEM}"item"."name" LIKE 'cheese'"#)
    );
    assert_eq!(
        sql(item::Column::Name.not_like("cheese")),
        format!(r#"{SELECT_ITEM}"item"."name" NOT LIKE 'cheese'"#)
    );
    assert_eq!(
        sql(item::Column::Name.starts_with("cheese")),
        format!(r#"{SELECT_ITEM}"item"."name" LIKE 'cheese%'"#)
    );
    assert_eq!(
        sql(item::Column::Name.ends_with("cheese")),
        format!(r#"{SELECT_ITEM}"item"."name" LIKE '%cheese'"#)
    );
    assert_eq!(
        sql(item::Column::Name.contains("cheese")),
        format!(r#"{SELECT_ITEM}"item"."name" LIKE '%cheese%'"#)
    );

    // Null checks.
    assert_eq!(
        sql(item::Column::Note.is_null()),
        format!(r#"{SELECT_ITEM}"item"."note" IS NULL"#)
    );
    assert_eq!(
        sql(item::Column::Note.is_not_null()),
        format!(r#"{SELECT_ITEM}"item"."note" IS NOT NULL"#)
    );

    // Set membership.
    assert_eq!(
        sql(item::Column::Id.is_in([1, 2, 3])),
        format!(r#"{SELECT_ITEM}"item"."id" IN (1, 2, 3)"#)
    );
    assert_eq!(
        sql(item::Column::Id.is_not_in([1, 2, 3])),
        format!(r#"{SELECT_ITEM}"item"."id" NOT IN (1, 2, 3)"#)
    );

    // Subqueries.
    let sub = item::Entity::find().select(item::Column::Id).into_query();
    assert_eq!(
        sql(item::Column::Id.in_subquery(sub.clone())),
        format!(r#"{SELECT_ITEM}"item"."id" IN (SELECT "item"."id" FROM "item")"#)
    );
    assert_eq!(
        sql(item::Column::Id.not_in_subquery(sub)),
        format!(r#"{SELECT_ITEM}"item"."id" NOT IN (SELECT "item"."id" FROM "item")"#)
    );

    // Aggregates and `if_null` render as projections.
    let agg = |e: pgorm_query::SimpleExpr| {
        item::Entity::find()
            .select_only()
            .column_as(e, Name::runtime("agg"))
            .as_query()
            .to_string()
    };
    assert_eq!(
        agg(item::Column::Id.max()),
        r#"SELECT MAX("item"."id") AS "agg" FROM "item""#
    );
    assert_eq!(
        agg(item::Column::Id.min()),
        r#"SELECT MIN("item"."id") AS "agg" FROM "item""#
    );
    assert_eq!(
        agg(item::Column::Id.sum()),
        r#"SELECT SUM("item"."id") AS "agg" FROM "item""#
    );
    assert_eq!(
        agg(item::Column::Id.count()),
        r#"SELECT COUNT("item"."id") AS "agg" FROM "item""#
    );
    assert_eq!(
        agg(item::Column::Note.if_null("none")),
        r#"SELECT COALESCE("item"."note", 'none') AS "agg" FROM "item""#
    );

    // `into_expr` / `into_returning_expr` turn a column into a bare expression.
    assert_eq!(
        agg(item::Column::Name.into_expr().into()),
        r#"SELECT "item"."name" AS "agg" FROM "item""#
    );
    assert_eq!(
        agg(item::Column::Name.into_returning_expr().into()),
        r#"SELECT "name" AS "agg" FROM "item""#
    );

    // `ColumnType` is a re-export of `pgorm_query::ColumnType`.
    let _: ColumnType = pgorm_query::ColumnType::Integer;
}

// [spec:pgorm:def:entity.traits.column+7/test]    the column-to-column family
// `eq_col` / `ne_col` / `gt_col` / `gte_col` / `lt_col` / `lte_col` and the
// expression form `eq_expr` — each side qualified by its own entity, same SQL as
// the `Expr::col` escape they replace, and the value-taking `eq` left alone
#[test]
fn column_trait_column_to_column_surface() {
    // Two columns of the same table.
    assert_eq!(
        sql(pair::Column::LeftId.eq_col(pair::Column::RightId)),
        format!(r#"{SELECT_ITEM}"pair"."left_id" = "pair"."right_id""#)
    );

    // Two columns of different tables: neither table is respelled at the call
    // site, each operand carries its own.
    assert_eq!(
        sql(item::Column::Id.eq_col(pair::Column::LeftId)),
        format!(r#"{SELECT_ITEM}"item"."id" = "pair"."left_id""#)
    );
    assert_eq!(
        sql(item::Column::Id.ne_col(pair::Column::LeftId)),
        format!(r#"{SELECT_ITEM}"item"."id" <> "pair"."left_id""#)
    );
    assert_eq!(
        sql(item::Column::Id.gt_col(pair::Column::LeftId)),
        format!(r#"{SELECT_ITEM}"item"."id" > "pair"."left_id""#)
    );
    assert_eq!(
        sql(item::Column::Id.gte_col(pair::Column::LeftId)),
        format!(r#"{SELECT_ITEM}"item"."id" >= "pair"."left_id""#)
    );
    assert_eq!(
        sql(item::Column::Id.lt_col(pair::Column::LeftId)),
        format!(r#"{SELECT_ITEM}"item"."id" < "pair"."left_id""#)
    );
    assert_eq!(
        sql(item::Column::Id.lte_col(pair::Column::LeftId)),
        format!(r#"{SELECT_ITEM}"item"."id" <= "pair"."left_id""#)
    );

    // The same predicate written through the `Expr::col` escape the sugar
    // replaces: identical SQL, so nothing is lost by taking the short road.
    assert_eq!(
        sql(item::Column::Id.eq_col(pair::Column::LeftId)),
        sql(Expr::col((item::Entity, item::Column::Id))
            .equals((pair::Entity, pair::Column::LeftId)))
    );

    // `eq_expr` takes anything computed — the case with no spelling at all in
    // the column vocabulary.
    assert_eq!(
        sql(item::Column::Id.eq_expr(Expr::col((item::Entity, item::Column::Id)).add(1))),
        format!(r#"{SELECT_ITEM}"item"."id" = "item"."id" + 1"#)
    );

    // The value-taking `eq` is untouched: it still takes `Into<Value>`, and a
    // bare column value still renders as a literal rather than a column ref.
    assert_eq!(
        sql(item::Column::Id.eq(1)),
        format!(r#"{SELECT_ITEM}"item"."id" = 1"#)
    );
}

// ---------------------------------------------------------------------------
// entity.traits.column-def
// ---------------------------------------------------------------------------

// [spec:pgorm:req:entity.traits.column-def+2/test]    `ColumnTypeTrait::def()`
// initialises a definition as non-null, non-unique, non-indexed with no default
// and no comment; each builder method flips exactly one attribute; and
// `get_column_type` / `is_null` expose the type and nullability
#[test]
fn column_def_defaults_and_builders() {
    let base = ColumnType::Integer.def();

    // The freshly initialised definition.
    assert_eq!(base.get_column_type(), &ColumnType::Integer);
    assert!(!base.is_null(), "a fresh ColumnDef must be non-null");

    // Each builder produces something different from the base, which is how we
    // know the base had that attribute switched off to begin with.
    assert_ne!(base, ColumnType::Integer.def().unique());
    assert_ne!(base, ColumnType::Integer.def().indexed());
    assert_ne!(base, ColumnType::Integer.def().null());
    assert_ne!(base, ColumnType::Integer.def().comment("hi"));
    assert_ne!(base, ColumnType::Integer.def().default_value(1));
    assert_ne!(base, ColumnType::Integer.def().default(Expr::value(1)));

    // ...and applying the same builder twice is idempotent, so these really are
    // flags rather than accumulators.
    assert_eq!(
        ColumnType::Integer.def().unique(),
        ColumnType::Integer.def().unique().unique()
    );

    // `null()` and `nullable()` are aliases.
    assert_eq!(
        ColumnType::Integer.def().null(),
        ColumnType::Integer.def().nullable()
    );
    assert!(ColumnType::Integer.def().null().is_null());
    assert!(ColumnType::Integer.def().nullable().is_null());

    // `default_value` takes a value, `default` takes an arbitrary expression;
    // a plain value routed through `default` lands in the same place.
    assert_eq!(
        ColumnType::Integer.def().default_value(1),
        ColumnType::Integer.def().default(Expr::value(1))
    );
    assert_ne!(
        ColumnType::Integer.def().default_value(1),
        ColumnType::Integer.def().default(Expr::raw("now()"))
    );

    // `get_column_type` reports whatever type the definition was built from.
    assert_eq!(ColumnType::Text.def().get_column_type(), &ColumnType::Text);
    assert_eq!(
        ColumnType::Boolean.def().nullable().get_column_type(),
        &ColumnType::Boolean
    );

    // `ColumnTypeTrait` is also implemented for `ColumnDef` itself, where `def()`
    // is the identity — that is the "or an existing ColumnDef" bridge.
    let built = ColumnType::Integer.def().unique().nullable();
    assert_eq!(built.clone().def(), built);

    // A derived entity's columns carry these attributes through.
    assert!(!item::Column::Id.def().is_null());
    assert!(!item::Column::Name.def().is_null());
    assert!(item::Column::Note.def().is_null());
    assert_eq!(
        item::Column::Name.def().get_column_type(),
        &ColumnType::String(pgorm_query::StringLen::None)
    );
}

// ---------------------------------------------------------------------------
// entity.traits.primary-key
// ---------------------------------------------------------------------------

// [spec:pgorm:def:entity.traits.primary-key+7/test]    `PrimaryKeyArity::ARITY` is
// 1 for any single scalar and matches the component count for tuples up to 12;
// `auto_increment` reports whether the key is database-generated; and
// `PrimaryKeyToColumn` maps variants to columns and back, with `from_column`
// returning `None` for a column that is not part of the key
#[test]
fn primary_key_trait_surface() {
    // Any single `TryGetable` scalar has arity 1.
    assert_eq!(<i32 as PrimaryKeyArity>::ARITY, 1);
    assert_eq!(<i64 as PrimaryKeyArity>::ARITY, 1);
    assert_eq!(<String as PrimaryKeyArity>::ARITY, 1);
    assert_eq!(<uuid::Uuid as PrimaryKeyArity>::ARITY, 1);

    // Tuple impls cover composite keys of 1 through 12 components.
    assert_eq!(<(i32,) as PrimaryKeyArity>::ARITY, 1);
    assert_eq!(<(i32, i32) as PrimaryKeyArity>::ARITY, 2);
    assert_eq!(<(i32, i32, i32) as PrimaryKeyArity>::ARITY, 3);
    assert_eq!(<(i32, i32, i32, i32) as PrimaryKeyArity>::ARITY, 4);
    assert_eq!(
        <(i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32) as PrimaryKeyArity>::ARITY,
        12
    );

    // An entity's `ValueType` picks up the matching arity.
    assert_eq!(
        <<item::PrimaryKey as PrimaryKeyTrait>::ValueType as PrimaryKeyArity>::ARITY,
        1
    );
    assert_eq!(
        <<pair::PrimaryKey as PrimaryKeyTrait>::ValueType as PrimaryKeyArity>::ARITY,
        2
    );

    // `auto_increment` distinguishes a database-generated key from a manual one.
    assert!(item::PrimaryKey::auto_increment());
    assert!(!pair::PrimaryKey::auto_increment());

    // The key is an iterable enum of key columns, in declaration order.
    assert_eq!(pair::PrimaryKey::iter().collect::<Vec<_>>().len(), 2);
    assert_eq!(
        pair::PrimaryKey::iter()
            .map(|k| k.into_column().as_str().to_owned())
            .collect::<Vec<_>>(),
        ["left_id", "right_id"]
    );

    // `into_column` and `from_column` are inverses over the key columns.
    assert_eq!(
        item::PrimaryKey::Id.into_column().as_str(),
        item::Column::Id.as_str()
    );
    assert!(matches!(
        item::PrimaryKey::from_column(item::Column::Id),
        Some(item::PrimaryKey::Id)
    ));
    // ...and `from_column` is `None` for a column outside the key.
    assert!(item::PrimaryKey::from_column(item::Column::Name).is_none());
    assert!(item::PrimaryKey::from_column(item::Column::Note).is_none());
    assert!(pair::PrimaryKey::from_column(pair::Column::Label).is_none());
    assert!(pair::PrimaryKey::from_column(pair::Column::RightId).is_some());
}

// ---------------------------------------------------------------------------
// entity.traits.model
// ---------------------------------------------------------------------------

// [spec:pgorm:def:entity.traits.model+3/test]    `ModelTrait::get` reads a column
// as a `Value` and `set` writes one; `find_related` scopes a `Select` to this
// instance; and `TryIntoModel` has a blanket identity impl for any model
#[test]
fn model_trait_get_set_and_identity() {
    let mut model = item::Model {
        id: 1,
        name: "Apple".to_owned(),
        note: None,
    };

    // `get` projects each column to its `Value`.
    assert_eq!(model.get(item::Column::Id), Value::Int(Some(1)));
    assert_eq!(
        model.get(item::Column::Name),
        Value::String(Some(Box::new("Apple".to_owned())))
    );
    assert_eq!(model.get(item::Column::Note), Value::String(None));

    // `set` writes it back, reporting a value of the wrong type as an error.
    model
        .set(
            item::Column::Name,
            Value::String(Some(Box::new("Pear".to_owned()))),
        )
        .expect("Name takes a String");
    assert_eq!(model.name, "Pear");
    model
        .set(
            item::Column::Note,
            Value::String(Some(Box::new("ripe".to_owned()))),
        )
        .expect("Note takes a String");
    assert_eq!(model.note, Some("ripe".to_owned()));
    model
        .set(item::Column::Note, Value::String(None))
        .expect("Note is nullable");
    assert_eq!(model.note, None);
    assert!(model.set(item::Column::Name, Value::Int(Some(1))).is_err());

    // `TryIntoModel` has a blanket identity impl for any `ModelTrait`.
    let same = model.clone().try_into_model().unwrap();
    assert_eq!(same, model);

    // `find_related` scopes a fresh `Select<R>` to this instance by filtering on
    // the owning side's key — the bakery/baker pair is a plain has-many.
    let bakery = bakery::Model {
        id: 7,
        name: "SeaSide".to_owned(),
        profit_margin: 10.4,
    };
    assert_eq!(
        bakery.find_related(Baker).as_query().to_string(),
        [
            r#"SELECT "baker"."id", "baker"."name", "baker"."contact_details", "baker"."bakery_id""#,
            r#"FROM "baker""#,
            r#"INNER JOIN "bakery" ON "bakery"."id" = "baker"."bakery_id""#,
            r#"WHERE "bakery"."id" = 7"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:def:entity.traits.model+3/test]    `into_active` converts a model
// into its own entity's ActiveModel with the destination fixed by the entity, so
// unlike `into_active_model` the call site carries no type annotation
#[test]
fn model_trait_into_active_needs_no_annotation() {
    let model = item::Model {
        id: 1,
        name: "Apple".to_owned(),
        note: None,
    };

    // No `let am: item::ActiveModel = ...` — the entity names the destination.
    let mut am = model.clone().into_active();
    am.name = set("Pear");
    assert!(am.is_changed());

    // Same conversion as the annotated `into_active_model`, before the edit.
    let annotated: item::ActiveModel = model.into_active_model();
    assert_eq!(annotated.id, am.id);
    assert_eq!(annotated.name, ActiveValue::Unchanged("Apple".to_owned()));
}

// [spec:pgorm:def:entity.traits.model+3/test]    `ModelTrait::delete` converts the
// model through `IntoActiveModel` and delegates to `ActiveModelTrait::delete`,
// so the behavior hooks run on the way through
#[pgorm_macros::test]
async fn model_trait_delete_runs_through_active_model() -> Result<(), Error> {
    let ctx = TestContext::new("model_trait_delete").await;
    let db = ctx.db.get().await?;
    let stmt = Schema::new().create_table_from_entity(item::Entity)?;
    db.execute(&stmt.to_string(), &[]).await?;

    let apple = item::ActiveModel {
        id: NotSet,
        name: set("Apple"),
        note: NotSet,
    }
    .insert(&db)
    .await?;
    let pear = item::ActiveModel {
        id: NotSet,
        name: set("Pear"),
        note: NotSet,
    }
    .insert(&db)
    .await?;

    // Deleting straight off the Model removes exactly that row.
    let res = apple.clone().delete(&db).await?;
    assert_eq!(res, 1);
    assert_eq!(item::Entity::find().all(&db).await?, [pear]);

    // A model whose key no longer matches anything deletes nothing, without error.
    let res = apple.delete(&db).await?;
    assert_eq!(res, 0);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// entity.traits.from-query-result
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, FromQueryResult)]
struct ItemRow {
    id: i32,
    name: String,
}

#[derive(Debug, PartialEq, PartialModel)]
#[pgorm(entity = item::Entity)]
struct ItemNameOnly {
    #[pgorm(from_col = item::Column::Name)]
    label: String,
}

/// The same two columns, read through a hand-written implementation that does
/// not report the columns it reads — the fallback branch of the optional
/// decode's absence rule.
#[derive(Debug, PartialEq)]
struct UnreflectedItemRow {
    id: i32,
    name: String,
}

impl FromQueryResult for UnreflectedItemRow {
    fn from_query_result(res: &QueryResult, pre: &str) -> Result<Self, Error> {
        Ok(Self {
            id: res.try_get(pre, "id")?,
            name: res.try_get(pre, "name")?,
        })
    }
}

/// `QueryResult` cannot be constructed outside the crate, so the prefix-handling
/// claims are probed from inside a `FromQueryResult` impl driven by a real row.
#[derive(Debug, PartialEq)]
struct PrefixProbe {
    /// Decoded under the prefix the columns were actually aliased with.
    prefixed: ItemRow,
    /// The same decode under the empty prefix cannot find those columns.
    unprefixed_is_err: bool,
    /// `from_query_result_optional` reports that miss rather than answering
    /// `None`: a column the statement never projected is a projection mistake,
    /// not an absent row.
    unprefixed_optional_is_err: bool,
    /// ...and still returns `Some` when the row does decode.
    prefixed_optional: Option<ItemRow>,
    /// Every column under the prefix present and `NULL` is the one shape read
    /// as an absent row.
    absent_optional: Option<ItemRow>,
    /// A type that reports no columns is judged on the result set instead, so
    /// the same all-`NULL` prefix is absent for it too...
    unreflected_absent_optional: Option<UnreflectedItemRow>,
    /// ...while a prefix the result set carries no column for is a miss it
    /// reports rather than swallows.
    unreflected_unknown_prefix_is_err: bool,
}

impl FromQueryResult for PrefixProbe {
    fn from_query_result(res: &QueryResult, _pre: &str) -> Result<Self, Error> {
        Ok(Self {
            prefixed: ItemRow::from_query_result(res, "s0_")?,
            unprefixed_is_err: ItemRow::from_query_result(res, "").is_err(),
            unprefixed_optional_is_err: ItemRow::from_query_result_optional(res, "").is_err(),
            prefixed_optional: ItemRow::from_query_result_optional(res, "s0_")?,
            absent_optional: ItemRow::from_query_result_optional(res, "s1_")?,
            unreflected_absent_optional: UnreflectedItemRow::from_query_result_optional(
                res, "s1_",
            )?,
            unreflected_unknown_prefix_is_err: UnreflectedItemRow::from_query_result_optional(
                res, "s2_",
            )
            .is_err(),
        })
    }
}

// [spec:pgorm:def:entity.traits.from-query-result+5/test]    `from_query_result`
// instantiates a type from a row under a column-name prefix,
// `from_query_result_optional` answers `Ok(None)` for an absent row and reports
// every other decode failure, `find_by_statement` runs raw SQL into typed rows,
// and `PartialModelTrait::select_cols` narrows a select to the columns it needs
// [spec:pgorm:req:exec.decode.absent/test]    the witness both branches of the
// absence rule are decided on
#[pgorm_macros::test]
async fn from_query_result_surface() -> Result<(), Error> {
    let ctx = TestContext::new("from_query_result_surface").await;
    let db = ctx.db.get().await?;
    let stmt = Schema::new().create_table_from_entity(item::Entity)?;
    db.execute(&stmt.to_string(), &[]).await?;

    Insert::many([
        item::ActiveModel {
            id: NotSet,
            name: set("Apple"),
            note: NotSet,
        },
        item::ActiveModel {
            id: NotSet,
            name: set("Pear"),
            note: NotSet,
        },
    ])
    .exec(&db)
    .await?;

    // `find_by_statement` builds a SelectorRaw that decodes raw SQL into `Self`.
    let rows: Vec<ItemRow> =
        ItemRow::find_by_statement(r#"SELECT "id", "name" FROM "item" ORDER BY "id""#, vec![])
            .all(&db)
            .await?;
    assert_eq!(
        rows,
        [
            ItemRow {
                id: 1,
                name: "Apple".to_owned()
            },
            ItemRow {
                id: 2,
                name: "Pear".to_owned()
            },
        ]
    );

    // `from_query_result` reads a row under a column-name prefix. Aliasing the
    // columns with a prefix is exactly how a source graph addresses each of its
    // decoded sources, and `from_query_result_optional` answers `Ok(None)` for
    // exactly one shape — every column it reads under the prefix present and
    // `NULL`, the row an outer join that matched nothing produces. The `s1_`
    // columns here stand in for that unmatched side; every other decode failure
    // is surfaced.
    let probe = PrefixProbe::find_by_statement(
        r#"SELECT "id" AS "s0_id", "name" AS "s0_name",
                  NULL::int AS "s1_id", NULL::text AS "s1_name"
           FROM "item" WHERE "id" = 1"#,
        vec![],
    )
    .one(&db)
    .await?;
    let apple = ItemRow {
        id: 1,
        name: "Apple".to_owned(),
    };
    assert_eq!(
        probe,
        PrefixProbe {
            prefixed: ItemRow {
                id: 1,
                name: "Apple".to_owned()
            },
            unprefixed_is_err: true,
            unprefixed_optional_is_err: true,
            prefixed_optional: Some(apple),
            absent_optional: None,
            unreflected_absent_optional: None,
            unreflected_unknown_prefix_is_err: true,
        }
    );

    // `PartialModelTrait::select_cols` declares exactly the columns it needs,
    // so the built statement projects only those.
    let narrowed = ItemNameOnly::select_cols(item::Entity::find().select_only());
    assert_eq!(
        narrowed.build().0,
        r#"SELECT "item"."name" AS "label" FROM "item""#
    );
    assert_eq!(
        item::Entity::find()
            .order_by_asc(item::Column::Id)
            .into_partial_model::<ItemNameOnly>()
            .all(&db)
            .await?,
        [
            ItemNameOnly {
                label: "Apple".to_owned()
            },
            ItemNameOnly {
                label: "Pear".to_owned()
            },
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
