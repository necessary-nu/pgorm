//! Verification for `Relation`: which variant attributes are required,
//! which are optional, and what each one contributes to the generated
//! `RelationDef`.

#![allow(dead_code)]

use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{ConditionType, Name, Values};
use pgorm::{Key, RelationType, SqlName};

mod cake {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "cake")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {
        // `has_many` needs neither `from` nor `to`: it reverses the target's
        // `Related<Self>` definition.
        #[pgorm(has_many = super::fruit::Entity)]
        Fruits,
        // ...but it still accepts them, as does `has_one`.
        #[pgorm(
            has_one = super::fruit::Entity,
            from = Column::Id,
            to = super::fruit::Column::CakeId
        )]
        TopFruit,
        // A closure `on_condition` names the two tables as the join binds
        // them; its parameters are typed for it unless it types them itself.
        #[pgorm(
            has_many = super::fruit::Entity,
            on_condition = |_left, right| Expr::col((right, super::fruit::Column::CakeId)).gt(10)
        )]
        BigFruits,
        #[pgorm(
            has_many = super::fruit::Entity,
            on_condition = |left: Name, _: Name| Expr::col((left, Column::Id)).gt(1)
        )]
        FromLaterCakes,
    }

    impl Related<super::fruit::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Fruits.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod fruit {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "fruit")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub cake_id: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {
        // The minimal `belongs_to`: the target path plus the mandatory
        // `from` / `to` pair.
        #[pgorm(
            belongs_to = super::cake::Entity,
            from = Column::CakeId,
            to = super::cake::Column::Id
        )]
        Cake,
        // The same, plus every optional builder key.
        #[pgorm(
            belongs_to = super::cake::Entity,
            from = Column::CakeId,
            to = super::cake::Column::Id,
            on_update = ForeignKeyAction::Cascade,
            on_delete = ForeignKeyAction::SetNull,
            on_condition = pgorm::pgorm_query::Expr::val(1).eq(1),
            fk_name = "fk_fruit_cake",
            condition_type = ConditionType::Any
        )]
        DecoratedCake,
    }

    impl Related<super::cake::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Cake.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// A standalone relation enum whose entity is named by the container attribute.
/// `Entity` is deliberately not in scope here, so this only compiles if
/// `#[pgorm(entity = ...)]` is honoured.
mod entity_override {
    use pgorm::entity::prelude::*;

    pub use super::cake::Entity as Renamed;

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    #[pgorm(entity = Renamed)]
    pub enum AltRelation {
        #[pgorm(
            belongs_to = super::fruit::Entity,
            from = super::cake::Column::Id,
            to = super::fruit::Column::CakeId
        )]
        Fruit,
    }
}

/// A shelf keyed `(tenant_id, id)`, and the books on it: a book names its
/// shelf by both columns, so `from` and `to` are tuples.
mod shelf {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "shelf")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub tenant_id: i32,
        #[pgorm(primary_key)]
        pub id: i32,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {
        #[pgorm(has_many = super::book::Entity)]
        Books,
    }

    impl Related<super::book::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Books.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod book {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "book")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub tenant_id: i32,
        #[pgorm(primary_key)]
        pub id: i32,
        pub shelf_id: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::shelf::Entity,
            from = (Column::TenantId, Column::ShelfId),
            to = (super::shelf::Column::TenantId, super::shelf::Column::Id),
            on_delete = ForeignKeyAction::Cascade
        )]
        Shelf,
        #[pgorm(
            belongs_to = super::shelf::Entity,
            from = (Column::TenantId, Column::ShelfId),
            to = (super::shelf::Column::TenantId, super::shelf::Column::Id),
            enforcement = Enforcement::NotEnforced,
            deferrability = Deferrability::DeferrableInitiallyDeferred
        )]
        LooseShelf,
        #[pgorm(
            belongs_to = super::shelf::Entity,
            from = Column::TenantId,
            to = super::shelf::Column::TenantId,
            from_period = Column::ShelfId,
            to_period = super::shelf::Column::Label,
            on_delete = ForeignKeyAction::NoAction
        )]
        ShelfOverTime,
    }

    impl Related<super::shelf::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Shelf.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

fn cols(id: &Key) -> Vec<String> {
    id.iter().map(|i| SqlName::to_string(&**i)).collect()
}

// [spec:pgorm:syn:macros.derive.relation+6/test]    belongs_to + the mandatory from/to
#[test]
fn belongs_to_builds_a_non_owning_relation_def() {
    let def = fruit::Relation::Cake.def();

    // `Entity::belongs_to(target)` => HasOne, not the owner.
    assert_eq!(def.rel_type, RelationType::HasOne);
    assert!(!def.is_owner);
    assert_eq!(def.from_tbl, fruit::Entity.table_ref().into());
    assert_eq!(def.to_tbl, cake::Entity.table_ref().into());
    assert_eq!(cols(&def.columns.from_key()), vec!["cake_id".to_owned()]);
    assert_eq!(cols(&def.columns.to_key()), vec!["id".to_owned()]);

    // Everything optional is left alone.
    assert!(def.on_update.is_none());
    assert!(def.on_delete.is_none());
    assert!(def.on_condition.is_none());
    assert_eq!(def.fk_name, None);
    assert_eq!(def.condition_type, ConditionType::All);
}

// [spec:pgorm:syn:macros.derive.relation+6/test]    the optional builder keys
#[test]
fn optional_keys_chain_onto_the_relation_builder() {
    let def = fruit::Relation::DecoratedCake.def();

    assert!(matches!(def.on_update, Some(ForeignKeyAction::Cascade)));
    assert!(matches!(def.on_delete, Some(ForeignKeyAction::SetNull)));
    assert_eq!(def.fk_name.as_deref(), Some("fk_fruit_cake"));
    // `condition_type` is matched case-insensitively.
    assert_eq!(def.condition_type, ConditionType::Any);

    // `on_condition` is wrapped in an `IntoCondition` closure taking the two
    // join-side names.
    let on_condition = def.on_condition.expect("on_condition should be set");
    let condition = on_condition(Name::runtime("l"), Name::runtime("r"));
    assert_eq!(
        condition,
        pgorm::pgorm_query::IntoCondition::into_condition(pgorm::pgorm_query::Expr::val(1).eq(1))
    );
}

// [spec:pgorm:req:macros.derive.one-form/test]    a closure `on_condition` takes the two
// tables as the join binds them
// [spec:pgorm:syn:macros.derive.relation+6/test]
#[test]
fn on_condition_closure_reads_the_bound_tables() {
    use pgorm::pgorm_query::IntoCondition;

    let (left, right) = (Name::runtime("l"), Name::runtime("r"));
    let big = cake::Relation::BigFruits
        .def()
        .on_condition
        .expect("on_condition should be set");
    assert_eq!(
        big(left.clone(), right.clone()),
        pgorm::pgorm_query::Expr::col((right.clone(), fruit::Column::CakeId))
            .gt(10)
            .into_condition()
    );
    let later = cake::Relation::FromLaterCakes
        .def()
        .on_condition
        .expect("on_condition should be set");
    assert_eq!(
        later(left.clone(), right),
        pgorm::pgorm_query::Expr::col((left, cake::Column::Id))
            .gt(1)
            .into_condition()
    );
}

// [spec:pgorm:syn:macros.derive.relation+6/test]    `enforcement` and `deferrability` name a
// variant of their enum and chain onto the builder, reaching the foreign key schema generation
// builds from the relation
#[test]
fn key_check_attributes_reach_the_foreign_key() {
    use pgorm::pgorm_query::{Deferrability, Enforcement, ForeignKeyCreateStatement};

    let def = book::Relation::LooseShelf.def();
    assert_eq!(def.enforcement, Some(Enforcement::NotEnforced));
    assert_eq!(
        def.deferrability,
        Some(Deferrability::DeferrableInitiallyDeferred)
    );
    assert_eq!(
        ForeignKeyCreateStatement::from(def).to_string(),
        [
            r#"ALTER TABLE "book" ADD CONSTRAINT "fk-book-tenant_id-shelf_id""#,
            r#"FOREIGN KEY ("tenant_id", "shelf_id") REFERENCES "shelf" ("tenant_id", "id")"#,
            r#"DEFERRABLE INITIALLY DEFERRED NOT ENFORCED"#,
        ]
        .join(" ")
    );
    let plain = book::Relation::Shelf.def();
    assert_eq!(plain.enforcement, None);
    assert_eq!(plain.deferrability, None);
}

// [spec:pgorm:syn:macros.derive.relation+6/test]    `from_period` / `to_period` set the
// relation's period pair after its equality pairs, which its foreign key writes PERIOD
#[test]
fn period_attributes_reach_the_foreign_key() {
    use pgorm::pgorm_query::ForeignKeyCreateStatement;

    let def = book::Relation::ShelfOverTime.def();
    let period = def.columns.period().expect("a period pair");
    assert_eq!(
        (period.0.to_string(), period.1.to_string()),
        ("shelf_id".to_owned(), "label".to_owned())
    );
    assert_eq!(cols(&def.columns.from_key()), ["tenant_id", "shelf_id"]);
    assert_eq!(
        ForeignKeyCreateStatement::from(def).to_string(),
        [
            r#"ALTER TABLE "book" ADD CONSTRAINT "fk-book-tenant_id-shelf_id""#,
            r#"FOREIGN KEY ("tenant_id", PERIOD "shelf_id") REFERENCES "shelf" ("tenant_id", PERIOD "label")"#,
            r#"ON DELETE NO ACTION"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:syn:macros.derive.relation+6/test]    has_many / has_one, where from and to are optional
#[test]
fn has_many_and_has_one_reverse_the_target() {
    let many = cake::Relation::Fruits.def();
    assert_eq!(many.rel_type, RelationType::HasMany);
    assert!(many.is_owner);
    // Reversed: the def reads from cake to fruit.
    assert_eq!(many.from_tbl, cake::Entity.table_ref().into());
    assert_eq!(many.to_tbl, fruit::Entity.table_ref().into());
    assert_eq!(cols(&many.columns.from_key()), vec!["id".to_owned()]);
    assert_eq!(cols(&many.columns.to_key()), vec!["cake_id".to_owned()]);

    let one = cake::Relation::TopFruit.def();
    assert_eq!(one.rel_type, RelationType::HasOne);
    assert!(one.is_owner);
    // Explicit `from` / `to` are accepted here too and override the reversal.
    assert_eq!(cols(&one.columns.from_key()), vec!["id".to_owned()]);
    assert_eq!(cols(&one.columns.to_key()), vec!["cake_id".to_owned()]);
}

// [spec:pgorm:syn:macros.derive.relation+6/test]    container-level entity override
#[test]
fn the_entity_identifier_is_overridable() {
    let def = entity_override::AltRelation::Fruit.def();
    // The relation was built from `Renamed` (= cake::Entity), not from an
    // `Entity` in scope — there is none.
    assert_eq!(def.from_tbl, cake::Entity.table_ref().into());
    assert_eq!(def.to_tbl, fruit::Entity.table_ref().into());
    assert_eq!(def.rel_type, RelationType::HasOne);
}

// [spec:pgorm:syn:macros.derive.relation+6/test]    tuple `from` / `to` pair
// column by column: the def carries every pair in order, and the join it
// renders, both ways round, constrains every pair rather than the first
#[test]
fn tuple_from_to_joins_every_column_pair() {
    let def = book::Relation::Shelf.def();
    assert_eq!(def.rel_type, RelationType::HasOne);
    assert_eq!(
        cols(&def.columns.from_key()),
        vec!["tenant_id".to_owned(), "shelf_id".to_owned()]
    );
    assert_eq!(
        cols(&def.columns.to_key()),
        vec!["tenant_id".to_owned(), "id".to_owned()]
    );
    assert!(matches!(def.on_delete, Some(ForeignKeyAction::Cascade)));

    let (books, _) = book::Entity::find()
        .join(JoinType::InnerJoin, book::Relation::Shelf.def())
        .build();
    assert_eq!(
        books,
        [
            r#"SELECT "book"."tenant_id", "book"."id", "book"."shelf_id" FROM "book""#,
            r#"INNER JOIN "shelf" ON "book"."tenant_id" = "shelf"."tenant_id""#,
            r#"AND "book"."shelf_id" = "shelf"."id""#,
        ]
        .join(" ")
    );

    let reversed = shelf::Relation::Books.def();
    assert_eq!(reversed.rel_type, RelationType::HasMany);
    assert_eq!(
        cols(&reversed.columns.from_key()),
        vec!["tenant_id".to_owned(), "id".to_owned()]
    );
    assert_eq!(
        cols(&reversed.columns.to_key()),
        vec!["tenant_id".to_owned(), "shelf_id".to_owned()]
    );
    let shelf = shelf::Model {
        tenant_id: 7,
        id: 3,
        label: "fiction".to_owned(),
    };
    let (books_on_shelf, values) = shelf.find_related(book::Entity).build();
    assert_eq!(
        books_on_shelf,
        [
            r#"SELECT "book"."tenant_id", "book"."id", "book"."shelf_id" FROM "book""#,
            r#"INNER JOIN "shelf" ON "shelf"."tenant_id" = "book"."tenant_id""#,
            r#"AND "shelf"."id" = "book"."shelf_id""#,
            r#"WHERE "shelf"."tenant_id" = $1 AND "shelf"."id" = $2"#,
        ]
        .join(" ")
    );
    assert_eq!(values, Values(vec![7.into(), 3.into()]));
}
