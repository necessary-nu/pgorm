//! Verification for `Model`, the `ActiveModel` the composite derive
//! writes (with `IntoActiveModel`), and `Column`.

#![allow(dead_code)]

use pgorm::entity::prelude::*;
use pgorm::{
    ActiveModelTrait, ActiveValue, Error, FromQueryResult, IntoActiveModel, IntoActiveValue,
    TryIntoModel, Value,
};
use std::str::FromStr;

/// `table_iden` is on so that `Column::Table` exists as a column the generated
/// `Model` / `ActiveModel` match arms deliberately do not cover — that is the
/// only way to reach their catch-all arms.
mod cake {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "cake", table_iden)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
        #[pgorm(ignore)]
        pub scratch: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The `entity` / `active_model` overrides. Neither `Entity` nor `ActiveModel`
/// is in scope at this level — only the renamed re-exports — so if the derives
/// ignored the attributes this module would not compile.
mod overrides {
    use pgorm::entity::prelude::*;

    mod inner {
        use pgorm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, Entity)]
        #[pgorm(table_name = "inner")]
        pub struct Model {
            #[pgorm(primary_key)]
            pub id: i32,
            pub name: String,
        }

        #[derive(Copy, Clone, Debug, Relation)]
        pub enum Relation {}
    }

    pub use inner::ActiveModel as InnerActive;
    pub use inner::Column as InnerColumn;
    pub use inner::Entity as InnerEntity;

    #[derive(Clone, Debug, PartialEq, Model)]
    #[pgorm(entity = InnerEntity)]
    pub struct AltModel {
        pub id: i32,
        pub name: String,
    }

    #[derive(Clone, Debug, PartialEq, IntoActiveModel)]
    #[pgorm(active_model = InnerActive)]
    pub struct NameOnly {
        pub name: String,
    }
}

/// A hand-rolled `Column` enum, away from any entity, so the derive's own
/// output is what is under test.
#[derive(Copy, Clone, Debug, Column)]
pub enum PlainColumn {
    Id,
    FirstName,
    #[pgorm(column_name = "lAsTnAmE")]
    LastName,
}

// [spec:pgorm:sem:macros.derive.model+6/test]    ModelTrait::get / set
#[test]
fn derive_model_get_and_set_walk_the_columns() {
    let mut model = cake::Model {
        id: 1,
        name: "chiffon".to_owned(),
        scratch: 42,
    };

    // `get` clones the field and converts it into a `Value`.
    assert_eq!(model.get(cake::Column::Id), Value::Int(Some(1)));
    assert_eq!(
        model.get(cake::Column::Name),
        Value::String(Some(Box::new("chiffon".to_owned())))
    );

    // `set` converts the value into the field type before assigning.
    model
        .set(
            cake::Column::Name,
            Value::String(Some(Box::new("sponge".into()))),
        )
        .expect("Name is a column of Model");
    assert_eq!(model.name, "sponge");
    assert!(
        model
            .set(cake::Column::Name, Value::Int(Some(1)))
            .is_err_and(|e| matches!(e, Error::Type(_)))
    );
    // The ignored field is untouched by either.
    assert_eq!(model.scratch, 42);
}

// [spec:pgorm:sem:macros.derive.model+6/test]    ignored fields have no match arm
#[test]
#[should_panic(expected = "field does not exist on Model")]
fn derive_model_get_panics_on_unmatched_column() {
    let model = cake::Model {
        id: 1,
        name: String::new(),
        scratch: 0,
    };
    let _ = model.get(cake::Column::Table);
}

// [spec:pgorm:sem:macros.derive.model+6/test]    ignored fields have no match arm
#[test]
fn derive_model_set_errs_on_unmatched_column() {
    let mut model = cake::Model {
        id: 1,
        name: String::new(),
        scratch: 0,
    };
    assert_eq!(
        model.set(cake::Column::Table, Value::Int(Some(1))),
        Err(Error::Type("field does not exist on Model".to_owned()))
    );
}

// [spec:pgorm:sem:macros.derive.model+6/test]    the FromQueryResult half, and the entity override
#[test]
fn derive_model_from_query_result_and_entity_override() {
    fn assert_from_query_result<T: FromQueryResult>() {}
    assert_from_query_result::<cake::Model>();
    assert_from_query_result::<overrides::AltModel>();

    // The target entity defaults to `Entity`...
    fn assert_entity<M: ModelTrait<Entity = E>, E: EntityTrait>() {}
    assert_entity::<cake::Model, cake::Entity>();
    // ...and `#[pgorm(entity = Ident)]` redirects it. `AltModel` lives in a
    // scope with no `Entity` at all, so this could only compile if the
    // attribute were honoured.
    assert_entity::<overrides::AltModel, overrides::InnerEntity>();

    let alt = overrides::AltModel {
        id: 3,
        name: "x".to_owned(),
    };
    assert_eq!(alt.get(overrides::InnerColumn::Id), Value::Int(Some(3)));
}

// [spec:pgorm:sem:macros.derive.active-model+7/test]    the generated struct and its conversions
#[test]
fn derive_active_model_struct_default_and_from_model() {
    // One `pub field: ActiveValue<T>` per *non-ignored* field: the struct
    // literal below is exhaustive, so `scratch` is absent.
    let active = cake::ActiveModel {
        id: ActiveValue::Set(1),
        name: ActiveValue::Set("chiffon".to_owned()),
    };
    assert_eq!(active.id, ActiveValue::Set(1));

    // `Default` delegates to `ActiveModelBehavior::new()`, which bottoms out in
    // `ActiveModelTrait::default()` — every field `NotSet`.
    let blank = <cake::ActiveModel as Default>::default();
    assert_eq!(blank.id, ActiveValue::NotSet);
    assert_eq!(blank.name, ActiveValue::NotSet);
    assert_eq!(blank, <cake::ActiveModel as ActiveModelTrait>::default());

    // `From<Model>` maps every field through `ActiveValue::unchanged`, and
    // `IntoActiveModel` is implemented for `Model` in terms of it.
    let model = cake::Model {
        id: 7,
        name: "opera".to_owned(),
        scratch: 5,
    };
    let converted: cake::ActiveModel = model.clone().into();
    assert_eq!(converted.id, ActiveValue::Unchanged(7));
    assert_eq!(converted.name, ActiveValue::Unchanged("opera".to_owned()));
    assert_eq!(model.into_active_model(), converted);
}

// [spec:pgorm:sem:macros.derive.active-model+7/test]    ActiveModelTrait
// accessors, including the one answer `take`, `get` and `set` share: a column
// this ActiveModel does not carry is `Error::Type`, in those same words
// [spec:pgorm:req:entity.active-model+4/test]    that refusal is the trait's
// rule, not the derive's convenience: an unknown column is never answered with
// `NotSet`, which is what a column holding no value reads as
#[test]
fn derive_active_model_trait_accessors() {
    let mut active = cake::ActiveModel {
        id: ActiveValue::Set(1),
        name: ActiveValue::Unchanged("chiffon".to_owned()),
    };

    // `get` clones into a wrapped value.
    assert_eq!(
        ActiveModelTrait::get(&active, cake::Column::Id),
        Ok(ActiveValue::Set(Value::Int(Some(1))))
    );
    // ...and an unmatched column is reported, not answered with `NotSet`: the
    // column does not exist here, which is not the same as holding no value.
    assert_eq!(
        ActiveModelTrait::get(&active, cake::Column::Table),
        Err(Error::Type(
            "This ActiveModel does not have this field".to_owned()
        ))
    );

    // `take` swaps the field out, leaving it `NotSet`.
    assert_eq!(
        active.take(cake::Column::Id),
        Ok(ActiveValue::Set(Value::Int(Some(1))))
    );
    assert_eq!(active.id, ActiveValue::NotSet);
    assert_eq!(
        active.take(cake::Column::Table),
        Err(Error::Type(
            "This ActiveModel does not have this field".to_owned()
        ))
    );

    // `set` converts the value into the field type before assigning `Set`.
    active
        .set(cake::Column::Id, Value::Int(Some(9)))
        .expect("Id is a column of ActiveModel");
    assert_eq!(active.id, ActiveValue::Set(9));
    assert!(
        active
            .set(cake::Column::Id, Value::String(None))
            .is_err_and(|e| matches!(e, Error::Type(_)))
    );

    // `not_set` clears a field, and silently ignores unmatched columns.
    active.not_set(cake::Column::Id);
    assert_eq!(active.id, ActiveValue::NotSet);
    active.not_set(cake::Column::Table);

    assert!(active.is_not_set(cake::Column::Id));
    assert!(!active.is_not_set(cake::Column::Name));

    // `reset` turns `Unchanged` into `Set`.
    active.reset(cake::Column::Name);
    assert_eq!(active.name, ActiveValue::Set("chiffon".to_owned()));
}

// [spec:pgorm:sem:macros.derive.active-model+7/test]    set errors on an unmatched column
#[test]
fn active_model_set_errs_on_unmatched_column() {
    let mut active = <cake::ActiveModel as Default>::default();
    assert_eq!(
        active.set(cake::Column::Table, Value::Int(Some(1))),
        Err(Error::Type(
            "This ActiveModel does not have this field".to_owned()
        ))
    );
}

// [spec:pgorm:sem:macros.derive.active-model+7/test]    is_not_set panics on an unmatched column
#[test]
#[should_panic(expected = "This ActiveModel does not have this field")]
fn active_model_is_not_set_panics_on_unmatched() {
    let active = <cake::ActiveModel as Default>::default();
    let _ = active.is_not_set(cake::Column::Table);
}

// [spec:pgorm:sem:macros.derive.active-model+7/test]    reset panics on an unmatched column
#[test]
#[should_panic(expected = "This ActiveModel does not have this field")]
fn active_model_reset_panics_on_unmatched_column() {
    let mut active = <cake::ActiveModel as Default>::default();
    active.reset(cake::Column::Table);
}

// [spec:pgorm:sem:macros.derive.active-model+7/test]    TryFrom<ActiveModel> / TryIntoModel
#[test]
fn derive_active_model_try_into_model() {
    // A non-ignored field left `NotSet` fails with `AttrNotSet(field)`.
    let partial = cake::ActiveModel {
        id: ActiveValue::Set(1),
        name: ActiveValue::NotSet,
    };
    match partial.clone().try_into_model() {
        Err(Error::AttrNotSet(field)) => assert_eq!(field, "name"),
        other => panic!("expected AttrNotSet, got {other:?}"),
    }
    assert!(matches!(
        cake::Model::try_from(partial),
        Err(Error::AttrNotSet(_))
    ));

    // With everything set, the ignored field is rebuilt with `Default::default()`.
    let full = cake::ActiveModel {
        id: ActiveValue::Set(1),
        name: ActiveValue::Unchanged("chiffon".to_owned()),
    };
    let model = full.try_into_model().unwrap();
    assert_eq!(
        model,
        cake::Model {
            id: 1,
            name: "chiffon".to_owned(),
            scratch: 0,
        }
    );
}

// [spec:pgorm:sem:macros.derive.active-model+7/test]    IntoActiveModel
#[test]
fn into_active_model_derive_fills_a_form() {
    // `IntoActiveModel` builds a partial "form" struct into the named
    // active model, filling the rest with `..Default::default()`.
    let form = overrides::NameOnly {
        name: "tart".to_owned(),
    };
    let active: overrides::InnerActive = form.into_active_model();
    assert_eq!(
        active.name,
        IntoActiveValue::<String>::into_active_value("tart".to_owned())
    );
    assert_eq!(active.id, ActiveValue::NotSet);
}

// [spec:pgorm:sem:macros.derive.column+7/test]    default_as_str / StaticName / SqlName
#[test]
fn derive_column_names() {
    // `default_as_str` is the snake_case of the variant, or the `column_name`
    // override.
    assert_eq!(PlainColumn::Id.default_as_str(), "id");
    assert_eq!(PlainColumn::FirstName.default_as_str(), "first_name");
    assert_eq!(PlainColumn::LastName.default_as_str(), "lAsTnAmE");

    // `StaticName::as_str` is `default_as_str`, and `SqlName` writes it.
    assert_eq!(
        pgorm::StaticName::as_str(&PlainColumn::FirstName),
        "first_name"
    );
    assert_eq!(PlainColumn::FirstName.to_string(), "first_name");
    assert_eq!(PlainColumn::LastName.to_string(), "lAsTnAmE");
}

// [spec:pgorm:sem:macros.derive.column+7/test]    FromStr accepts both spellings
#[test]
fn derive_column_from_str() {
    assert!(matches!(
        PlainColumn::from_str("first_name"),
        Ok(PlainColumn::FirstName)
    ));
    assert!(matches!(
        PlainColumn::from_str("firstName"),
        Ok(PlainColumn::FirstName)
    ));
    // The `column_name` override does *not* widen `FromStr`: it still keys off
    // the variant identifier.
    assert!(matches!(
        PlainColumn::from_str("last_name"),
        Ok(PlainColumn::LastName)
    ));
    assert!(matches!(
        PlainColumn::from_str("lastName"),
        Ok(PlainColumn::LastName)
    ));

    match PlainColumn::from_str("lAsTnAmE") {
        Err(pgorm::ColumnFromStrError(s)) => assert_eq!(s, "lAsTnAmE"),
        other => panic!("expected ColumnFromStrError, got {other:?}"),
    }
}

// [spec:pgorm:sem:macros.derive.column+7/test]    the derive makes the enum iterable itself
// [spec:pgorm:sem:macros.derive.boilerplate/test]    `Column` iterates its variants with no `EnumIter` beside it
#[test]
fn column_derive_iterates_its_variants() {
    use pgorm::Iterable;
    assert_eq!(PlainColumn::iter().count(), 3);
}
