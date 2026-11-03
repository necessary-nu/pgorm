//! Verification for the macro catalogue itself: every derive `pgorm-macros`
//! exposes is reachable through the entity prelude alone, they all read their
//! configuration from the shared `#[pgorm(...)]` helper attribute (except the
//! name derives, which use pgorm-query's `#[iden]`), and the auxiliary derives
//! behave as documented.
//!
//! The crate depends on pgorm and not on pgorm-query, so `pgorm_query` names
//! nothing here: a derive whose expansion named it would not compile.

#![allow(dead_code)]

// The prelude twice over, by both its paths, beside the crate root: each name
// reaches one item however it is imported, so no glob makes a derive
// ambiguous.
#[allow(unused_imports)]
use pgorm::entity::prelude::*;
#[allow(unused_imports)]
use pgorm::prelude::*;
#[allow(unused_imports)]
use pgorm::*;
use serde::{Deserialize, Serialize};

/// The `Entity` derive (which re-runs the `Model` derive) and `Relation`.
mod cake {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "cake")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// An entity whose name is computed at run time, which is what exercises
/// `EntityTrait`: its `Entity` is written by hand beside a composite derive
/// given no `table_name`. `Model` makes it a second model.
mod fruit {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Default, Debug, EntityTrait)]
    pub struct Entity {
        pub orchard: Option<&'static str>,
    }

    impl EntityName for Entity {
        fn schema_name(&self) -> Option<&str> {
            self.orchard
        }

        fn table_name(&self) -> &str {
            "fruit"
        }
    }

    #[derive(Clone, Debug, PartialEq, Entity)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}

    #[derive(Clone, Debug, PartialEq, Model)]
    pub struct Named {
        pub name: String,
    }
}

/// The column enum of a custom select, one of whose result columns is not
/// the snake_case of its variant.
#[derive(Copy, Clone, Debug, Column)]
pub enum Selected {
    Id,
    #[pgorm(column_name = "fruit_count")]
    Count,
}

/// `IntoActiveModel`.
use cake::ActiveModel as CakeActive;

#[derive(Clone, Debug, PartialEq, IntoActiveModel)]
#[pgorm(active_model = CakeActive)]
pub struct CakeName {
    pub name: String,
}

/// `ActiveEnum`, with the `display` key.
#[derive(Debug, Clone, PartialEq, Eq, ActiveEnum)]
#[pgorm(rs_type = String, db_type = ColumnType::String(StringLen::None), display)]
pub enum Tea {
    #[pgorm(string_value = "E", display_value = "Everyday")]
    Everyday,
}

/// `FromQueryResult`.
#[derive(Debug, FromQueryResult)]
pub struct Projection {
    pub name: String,
}

/// `JsonValueType`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonValueType)]
pub struct Meta {
    pub tags: Vec<String>,
}

/// `PartialModel`.
#[derive(Debug, PartialEq, PartialModel)]
#[pgorm(entity = cake::Entity)]
pub struct CakeSlice {
    pub name: String,
}

/// `ValueType`.
#[derive(Debug, PartialEq, ValueType)]
pub struct Grams(i32);

/// `SqlName`, pgorm-query's derive reached through pgorm.
#[derive(SqlName)]
#[iden = "cake_table"]
pub struct CakeTable;

/// `StaticName`, likewise.
#[derive(Clone, Copy, Debug, StaticName)]
pub enum Shelf {
    Table,
    #[iden = "top_shelf"]
    Top,
}

/// `CreatedRange`.
#[derive(Clone, Debug, PartialEq, CreatedRange)]
#[pgorm(range_name = "floatrange")]
pub struct FloatRange(pub Range<f64>);

/// `Disjoint`, over a base whose kind column holds the kind type it writes.
mod shelf {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "shelf")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: super::StockKind,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = shelf::Entity)]
pub enum Stock {
    Cake(cake::Model),
    Empty,
}

// [spec:pgorm:def:macros.derive+7/test]    every derive in the catalogue is reachable and wired up
// [spec:pgorm:req:macros.derive.names+1/test]    each derive by its name through the prelude, glob-imported by both its paths and beside the crate root's glob
#[test]
fn the_whole_derive_catalogue_is_exposed() {
    // Entity-side derives: the composite, and the entity-trait derive beside
    // it for a name computed at run time.
    assert_eq!(cake::Entity.table_name(), "cake");
    let orchard = fruit::Entity {
        orchard: Some("orchard"),
    };
    assert_eq!(orchard.table_name(), "fruit");
    assert_eq!(orchard.schema_name(), Some("orchard"));
    assert_eq!(cake::Column::iter().count(), 2);
    assert_eq!(fruit::Column::iter().count(), 2);
    assert!(fruit::PrimaryKey::auto_increment());
    assert_eq!(Selected::Count.to_string(), "fruit_count");
    let named = fruit::Named {
        name: "pear".to_owned(),
    };
    assert_eq!(named.get(fruit::Column::Name), "pear".into());

    // Model / ActiveModel derives.
    let model = fruit::Model {
        id: 1,
        name: "pear".to_owned(),
    };
    let active: fruit::ActiveModel = model.into_active_model();
    assert_eq!(
        active.name,
        pgorm::ActiveValue::unchanged("pear".to_owned())
    );
    let from_form: cake::ActiveModel = CakeName {
        name: "opera".to_owned(),
    }
    .into_active_model();
    assert_eq!(from_form.id, pgorm::ActiveValue::NotSet);

    // Value / enum / projection derives.
    assert_eq!(Tea::Everyday.to_value(), "E");
    assert_eq!(Tea::Everyday.to_string(), "Everyday");
    assert_eq!(pgorm::Value::from(Grams(5)), pgorm::Value::Int(Some(5)));
    assert_eq!(CakeTable.to_string(), "cake_table");
    assert_eq!(Shelf::Table.as_str(), "shelf");
    assert_eq!(Shelf::Top.as_str(), "top_shelf");
    assert_eq!(
        <FloatRange as CreatedRange>::name(),
        pgorm::pgorm_query::TypeName::new(Name::runtime("floatrange"))
    );
    assert_eq!(StockKind::values(), ["cake", "empty"]);
    assert_eq!(Stock::Empty.kind(), StockKind::Empty);

    // The projection derives are compile-time contracts; assert the traits.
    fn assert_from_query_result<T: FromQueryResult>() {}
    assert_from_query_result::<Projection>();
    assert_from_query_result::<CakeSlice>();
    fn assert_partial_model<T: pgorm::PartialModelTrait>() {}
    assert_partial_model::<CakeSlice>();
    fn assert_json_marker<T: pgorm::TryGetableFromJson>() {}
    assert_json_marker::<Meta>();
    match pgorm::Value::from(Meta {
        tags: vec!["a".to_owned()],
    }) {
        pgorm::Value::Json(Some(json)) => assert_eq!(json["tags"][0], "a"),
        other => panic!("expected Value::Json, got {other:?}"),
    }
}
