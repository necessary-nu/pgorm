//! Verification for the macro catalogue itself: every derive `pgorm-macros`
//! exposes is reachable, they all read their configuration from the shared
//! `#[pgorm(...)]` helper attribute (except `EnumIter`, which uses
//! `#[strum(...)]`), and the auxiliary derives behave as documented.

#![allow(dead_code)]

use pgorm::entity::prelude::*;
use pgorm::{
    DeriveDisplay, DeriveIntoActiveModel, DeriveValueType, FromJsonQueryResult, FromQueryResult,
    IntoActiveModel, Iterable,
};
use serde::{Deserialize, Serialize};

/// `DeriveEntityModel` (which re-runs `DeriveModel`) and `DeriveRelation`.
mod cake {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[pgorm(table_name = "cake")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// An entity whose name is computed at run time, which is what exercises
/// `DeriveEntity`: its `Entity` is written by hand beside a composite derive
/// given no `table_name`. `DeriveModel` makes it a second model and
/// `DeriveActiveModelBehavior` its behaviour.
mod fruit {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Default, Debug, DeriveEntity)]
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

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, DeriveActiveModelBehavior)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    #[derive(Clone, Debug, PartialEq, DeriveModel)]
    pub struct Named {
        pub name: String,
    }
}

/// The column enum of a custom select, one of whose result columns is not
/// the snake_case of its variant.
#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
pub enum Selected {
    Id,
    #[pgorm(column_name = "fruit_count")]
    Count,
}

/// `DeriveIntoActiveModel`.
use cake::ActiveModel as CakeActive;

#[derive(Clone, Debug, PartialEq, DeriveIntoActiveModel)]
#[pgorm(active_model = CakeActive)]
pub struct CakeName {
    pub name: String,
}

/// `DeriveActiveEnum` + `DeriveDisplay`.
#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, DeriveDisplay)]
#[pgorm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum Tea {
    #[pgorm(string_value = "E", display_value = "Everyday")]
    Everyday,
}

/// `FromQueryResult`.
#[derive(Debug, FromQueryResult)]
pub struct Projection {
    pub name: String,
}

/// `FromJsonQueryResult`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromJsonQueryResult)]
pub struct Meta {
    pub tags: Vec<String>,
}

/// `DerivePartialModel`.
#[derive(Debug, PartialEq, DerivePartialModel, FromQueryResult)]
#[pgorm(entity = "cake::Entity")]
pub struct CakeSlice {
    pub name: String,
}

/// `DeriveValueType`.
#[derive(Debug, PartialEq, DeriveValueType)]
pub struct Grams(i32);

/// `DeriveSqlName`.
#[derive(DeriveSqlName)]
#[pgorm(iden = "cake_table")]
pub struct CakeTable;

/// `EnumIter`, the one derive configured through `#[strum(...)]` rather than
/// `#[pgorm(...)]`.
#[derive(Debug, PartialEq, EnumIter)]
pub enum Flavour {
    Sweet,
    #[strum(disabled)]
    Secret,
}

// [spec:pgorm:def:macros.derive+4/test]    every derive in the catalogue is reachable and wired up
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
    assert_eq!(Flavour::iter().collect::<Vec<_>>(), vec![Flavour::Sweet]);

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
