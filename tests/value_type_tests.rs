#![allow(unused_imports, dead_code)]

pub mod common;

use std::sync::Arc;
use std::vec;

pub use common::{
    TestContext,
    features::{
        value_type::{Boolbean, Integer, StringVec, value_type_general, value_type_pg},
        *,
    },
    setup::*,
};
use pgorm::{ActiveValue::Unchanged, entity::prelude::*, entity::*};
use pgorm_query::{ArrayType, ColumnType, Value, ValueType, ValueTypeError};
use pretty_assertions::assert_eq;

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("value_type_tests").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    insert_value(&db).await?;
    drop(db);
    ctx.delete().await;

    let ctx = TestContext::new("value_type_postgres_tests").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    postgres_insert_value(&db).await?;
    drop(db);
    ctx.delete().await;

    type_test();
    conversion_test();

    Ok(())
}

pub async fn insert_value(db: &DatabaseConnection) -> Result<(), Error> {
    let model = value_type_general::Model {
        id: 1,
        number: 48.into(),
    };
    let result = model.clone().into_active_model().insert(db).await?;
    assert_eq!(result, model);

    Ok(())
}

pub async fn postgres_insert_value(db: &DatabaseConnection) -> Result<(), Error> {
    let model = value_type_pg::Model {
        id: 1,
        number: 48.into(),
        str_vec: StringVec(vec!["ab".to_string(), "cd".to_string()]),
    };
    let result = model.clone().into_active_model().insert(db).await?;
    assert_eq!(result, model);

    Ok(())
}

pub fn type_test() {
    assert_eq!(StringVec::type_name(), "StringVec");

    // custom types
    assert_eq!(Integer::array_type(), ArrayType::Int);
    assert_eq!(Integer::array_type(), ArrayType::Int);
    assert_eq!(Boolbean::column_type(), ColumnType::Boolean);
    assert_eq!(Boolbean::array_type(), ArrayType::Bool);

    // self implied
    assert_eq!(
        StringVec::column_type(),
        ColumnType::Array(Arc::new(ColumnType::String(StringLen::None)))
    );
    assert_eq!(StringVec::array_type(), ArrayType::String);
}

pub fn conversion_test() {
    let stringvec = StringVec(vec!["ab".to_string(), "cd".to_string()]);
    let string: Value = stringvec.into();
    assert_eq!(
        string,
        Value::Array(
            ArrayType::String,
            Some(Box::new(vec![
                "ab".to_string().into(),
                "cd".to_string().into()
            ]))
        )
    );

    let value_random_int = Value::Int(Some(523));
    let try_from_int =
        <Integer as ValueType>::from_value(value_random_int).expect("should be ok to convert");

    let direct_int: Integer = 523.into();
    assert_eq!(direct_int, try_from_int);

    // test for error
    let try_from_string_vec = <StringVec as ValueType>::from_value(Value::Char(Some('a')))
        .expect_err("should not be ok to convert char to stringvec");
    assert_eq!(try_from_string_vec.to_string(), ValueTypeError.to_string());
}

// [spec:pgorm:def:sql.value.value-type+5/test]    the extraction is named
// `from_value`, so with the trait and the prelude in scope it is called as
// written: neither the language prelude's `TryFrom::try_from` nor an
// `ActiveEnum`'s `try_from_value` shares the name
#[test]
fn from_value_reads_as_written_beside_the_prelude() {
    use pgorm::tests_cfg::active_enums::Tea;

    let price: Value = Decimal::new(450, 2).into();
    assert_eq!(
        Decimal::from_value(price.clone()).ok(),
        Some(Decimal::new(450, 2))
    );
    assert!(i32::from_value(price).is_err());
    assert_eq!(
        Tea::from_value(Tea::EverydayTea.into()).ok(),
        Some(Tea::EverydayTea)
    );
    assert_eq!(
        Tea::try_from_value(&"EverydayTea".to_owned()).ok(),
        Some(Tea::EverydayTea)
    );
}

mod parcel {
    use pgorm::entity::prelude::*;

    /// A weight, an ordinary `integer` the compiler keeps apart.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, ValueType)]
    pub struct Grams(pub i32);

    /// A newtype over an optional value: nullable however it is held.
    #[derive(Clone, Debug, PartialEq, Eq, ValueType)]
    pub struct Note(pub Option<String>);

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "nullable_parcel")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub declared: Option<Grams>,
        pub label: Option<String>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}

    /// A form setting the nullable newtype columns.
    #[derive(IntoActiveModel)]
    pub struct Weighed {
        pub id: i32,
        pub declared: Option<Option<Grams>>,
    }
}

// [spec:pgorm:sem:macros.derive.active-model+9/test]    a form's optional newtype field
// [spec:pgorm:sem:macros.derive.value-type+7/test]    a ValueType newtype is a
// nullable column: `None` binds as NULL, reads back as `None`, is filtered on,
// and a form sets the column, to NULL through `Some(None)`
#[pgorm_macros::test]
async fn an_optional_newtype_is_a_nullable_column() -> Result<(), Error> {
    use parcel::{ActiveModel, Column, Entity, Grams, Model, Note, Weighed};

    let ctx = TestContext::new("value_type_nullable_column").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        &pgorm::Schema::new()
            .create_table_from_entity(Entity)?
            .to_string(),
    )
    .await?;

    let weighed = Model {
        id: 1,
        declared: Some(Grams(250)),
        label: Some("fragile".to_owned()),
    };
    let unweighed = Model {
        id: 2,
        declared: None,
        label: None,
    };
    for model in [&weighed, &unweighed] {
        model.clone().into_active().insert(&db).await?;
    }
    assert_eq!(
        Entity::find().order_by_asc(Column::Id).all(&db).await?,
        [weighed.clone(), unweighed.clone()]
    );
    assert_eq!(
        Entity::find()
            .filter(Column::Declared.is_null())
            .all(&db)
            .await?,
        std::slice::from_ref(&unweighed)
    );

    let cleared = Weighed {
        id: 1,
        declared: Some(None),
    }
    .into_active_model();
    assert_eq!(cleared.declared, set(None));
    let cleared: Model = ActiveModel {
        id: set(1),
        declared: cleared.declared,
        ..Default::default()
    }
    .update(&db)
    .await?;
    assert_eq!(cleared.declared, None);
    assert_eq!(cleared.label, weighed.label);

    // A newtype over an `Option` is null as its payload is.
    assert_eq!(<Note as pgorm_query::Nullable>::null(), Value::String(None));
    assert_eq!(<Grams as pgorm_query::Nullable>::null(), Value::Int(None));

    drop(db);
    ctx.delete().await;
    Ok(())
}
