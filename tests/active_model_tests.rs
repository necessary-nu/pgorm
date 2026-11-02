#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, bakery_chain::*, setup::*};
use pgorm::{Error, Schema, TransactionError, entity::prelude::*};
use pgorm_query::{IntoValueTuple, QueryBuilder, ValueTuple};
use pretty_assertions::assert_eq;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Auto-increment single-column key, one nullable column: the shape almost every
/// `ActiveModelTrait` claim is stated against.
mod row {
    use pgorm::entity::prelude::*;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, Entity, Serialize, Deserialize)]
    #[pgorm(table_name = "am_row")]
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

/// Manual (non auto-increment) single-column key: the caller supplies the key,
/// so the operation cannot be read off the model's state.
mod manual_key {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "am_manual_key")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

mod pk2 {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "am_pk2")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id_1: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub id_2: String,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

mod pk3 {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "am_pk3")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id_1: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub id_2: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub id_3: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

mod pk4 {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "am_pk4")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id_1: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub id_2: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub id_3: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub id_4: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// The SQL column name and the `serde` key pull apart: `#[pgorm(column_name)]`
/// renames the column, `serde` keeps reading the field's own name.
mod sql_renamed {
    use pgorm::entity::prelude::*;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, Entity, Serialize, Deserialize)]
    #[pgorm(table_name = "am_sql_renamed")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(column_name = "createdAt")]
        pub created_at: String,
        pub note: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// The other direction: the SQL columns keep their plain names and `serde`
/// renames the keys, per struct and per field.
mod serde_renamed {
    use pgorm::entity::prelude::*;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, Entity, Serialize, Deserialize)]
    #[pgorm(table_name = "am_serde_renamed")]
    #[serde(rename_all = "camelCase")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub created_at: String,
        #[serde(rename = "NOTE")]
        pub note: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

// ---------------------------------------------------------------------------
// ActiveModelTrait: per-column state access
// ---------------------------------------------------------------------------

// [spec:pgorm:req:entity.active-model+4/test]    the per-column state surface —
// `default()` leaves every column `NotSet`, `set` stores a `Value` as `Set`,
// `get` reads without consuming, `take` removes and leaves `NotSet` behind,
// `not_set` clears, `is_not_set` reports, and `reset` / `reset_all` promote
// `Unchanged` to `Set` while leaving `NotSet` alone. `get` and `take` are
// fallible for the reason `set` is, so each read here is an `Ok`
#[test]
fn active_model_column_state_access() {
    // `default()` is every column `NotSet`.
    let mut am = <row::ActiveModel as ActiveModelTrait>::default();
    for col in row::Column::iter() {
        assert!(am.is_not_set(col), "{col:?} should start NotSet");
        assert_eq!(am.get(col), Ok(ActiveValue::NotSet));
    }
    assert!(!am.is_changed());

    // `set` stores a `Value` in the `Set` state; `get` is a non-consuming read.
    am.set(
        row::Column::Name,
        Value::String(Some(Box::new("Apple".to_owned()))),
    )
    .expect("Name takes a String");
    assert!(!am.is_not_set(row::Column::Name));
    assert_eq!(
        am.get(row::Column::Name),
        Ok(set(Value::String(Some(Box::new("Apple".to_owned())))))
    );
    // Reading twice yields the same thing: `get` did not consume.
    assert_eq!(
        am.get(row::Column::Name),
        Ok(set(Value::String(Some(Box::new("Apple".to_owned())))))
    );
    assert!(am.is_changed());

    // `take` removes and returns, leaving `NotSet`.
    let taken = am.take(row::Column::Name);
    assert_eq!(
        taken,
        Ok(set(Value::String(Some(Box::new("Apple".to_owned())))))
    );
    assert!(am.is_not_set(row::Column::Name));

    // `not_set` clears an already-populated attribute.
    am.set(
        row::Column::Name,
        Value::String(Some(Box::new("Pear".to_owned()))),
    )
    .expect("Name takes a String");
    assert!(!am.is_not_set(row::Column::Name));
    am.not_set(row::Column::Name);
    assert!(am.is_not_set(row::Column::Name));
    assert!(!am.is_changed());

    // `reset` promotes exactly one column from `Unchanged` to `Set` and leaves
    // `NotSet` columns untouched.
    let mut am = row::ActiveModel {
        id: ActiveValue::Unchanged(1),
        name: ActiveValue::Unchanged("Apple".to_owned()),
        note: ActiveValue::NotSet,
    };
    assert!(!am.is_changed());
    am.reset(row::Column::Name);
    assert_eq!(
        am,
        row::ActiveModel {
            id: ActiveValue::Unchanged(1),
            name: set("Apple"),
            note: ActiveValue::NotSet,
        }
    );
    am.reset(row::Column::Note);
    assert!(am.note.is_not_set(), "reset must leave NotSet untouched");

    // `reset_all` applies `reset` to every column.
    let all = row::Model {
        id: 1,
        name: "Apple".to_owned(),
        note: Some("crisp".to_owned()),
    }
    .into_active_model()
    .reset_all();
    assert_eq!(
        all,
        row::ActiveModel {
            id: set(1),
            name: set("Apple"),
            note: "crisp".into(),
        }
    );
    assert!(all.is_changed());
}

// [spec:pgorm:req:entity.active-model+4/test]    `get_primary_key_value` reads
// `PrimaryKeyArity::ARITY` values into one `ValueTuple`, whatever the arity, and
// returns `None` when any key component is `NotSet`
#[test]
fn active_model_get_primary_key_value() {
    // Arity 1. `Unchanged` counts as holding a value.
    let am = row::ActiveModel {
        id: ActiveValue::Unchanged(7),
        name: set("Apple"),
        note: ActiveValue::NotSet,
    };
    assert_eq!(
        am.get_primary_key_value(),
        Some(ValueTuple::from(vec![Value::Int(Some(7))]))
    );

    // Arity 2, in primary-key iteration order.
    let am = pk2::ActiveModel {
        id_1: set(1),
        id_2: set("two"),
        name: ActiveValue::NotSet,
    };
    assert_eq!(
        am.get_primary_key_value(),
        Some(ValueTuple::from(vec![
            Value::Int(Some(1)),
            Value::String(Some(Box::new("two".to_owned())))
        ]))
    );

    // Arity 3.
    let am = pk3::ActiveModel {
        id_1: set(1),
        id_2: set(2),
        id_3: set(3),
        name: ActiveValue::NotSet,
    };
    assert_eq!(
        am.get_primary_key_value(),
        Some(ValueTuple::from(vec![
            Value::Int(Some(1)),
            Value::Int(Some(2)),
            Value::Int(Some(3))
        ]))
    );

    // Arity 4 reads through the same walk as every other arity.
    let am = pk4::ActiveModel {
        id_1: set(1),
        id_2: set(2),
        id_3: set(3),
        id_4: set(4),
        name: ActiveValue::NotSet,
    };
    assert_eq!(
        am.get_primary_key_value(),
        Some(ValueTuple::from(vec![
            Value::Int(Some(1)),
            Value::Int(Some(2)),
            Value::Int(Some(3)),
            Value::Int(Some(4)),
        ]))
    );

    // A tuple gathered from an iterator is the same value as one a Rust tuple
    // builds: the collapsed representation has no second spelling for a key.
    assert_eq!(
        am.get_primary_key_value(),
        Some((1i32, 2i32, 3i32, 4i32).into_value_tuple())
    );

    // Any `NotSet` key component collapses the whole thing to `None`.
    let am = row::ActiveModel {
        id: ActiveValue::NotSet,
        name: set("Apple"),
        note: ActiveValue::NotSet,
    };
    assert_eq!(am.get_primary_key_value(), None);

    let am = pk2::ActiveModel {
        id_1: set(1),
        id_2: ActiveValue::NotSet,
        name: ActiveValue::NotSet,
    };
    assert_eq!(am.get_primary_key_value(), None);
}

// ---------------------------------------------------------------------------
// ActiveValue: the three-state machine
// ---------------------------------------------------------------------------

// [spec:pgorm:def:entity.active-model.active-value+1/test]    the three variants,
// `Default::default()` == `NotSet`, the crate-root `NotSet` re-export, the
// variant-sensitive `PartialEq`, and the statement-level consequence: only `Set`
// reaches the INSERT/UPDATE column list while an `Unchanged` primary key still
// drives the WHERE clause
#[test]
fn active_value_three_state_machine() {
    // Default is NotSet.
    assert_eq!(ActiveValue::<i32>::default(), ActiveValue::NotSet);

    // `NotSet` is re-exported at the crate root, so struct literals read naturally.
    let am = row::ActiveModel {
        id: NotSet,
        name: set("Apple"),
        note: NotSet,
    };
    assert!(am.id.is_not_set());
    assert!(am.note.is_not_set());

    // PartialEq is equal only for identical variants carrying equal payloads.
    assert_eq!(ActiveValue::set(1), ActiveValue::set(1));
    assert_eq!(ActiveValue::Unchanged(1), ActiveValue::Unchanged(1));
    assert_eq!(ActiveValue::<i32>::NotSet, ActiveValue::<i32>::NotSet);
    assert_ne!(ActiveValue::set(1), ActiveValue::set(2));
    assert_ne!(ActiveValue::set(1), ActiveValue::Unchanged(1));
    assert_ne!(ActiveValue::set(1), ActiveValue::<i32>::NotSet);
    assert_ne!(ActiveValue::Unchanged(1), ActiveValue::<i32>::NotSet);

    // Only `Set` columns are written. `note` is NotSet, so it is absent entirely.
    assert_eq!(
        Insert::one(row::ActiveModel {
            id: NotSet,
            name: set("Apple"),
            note: NotSet,
        })
        .build()
        .0,
        r#"INSERT INTO "am_row" ("name") VALUES ($1)"#
    );

    // On UPDATE, an `Unchanged` primary key drives the WHERE clause without
    // appearing in SET, and `Unchanged` non-key columns are not written either.
    assert_eq!(
        Update::one(row::ActiveModel {
            id: ActiveValue::Unchanged(1),
            name: set("Apple"),
            note: ActiveValue::Unchanged(Some("old".to_owned())),
        })
        .expect("the primary key is unchanged, not unset")
        .build()
        .0,
        r#"UPDATE "am_row" SET "name" = $1 WHERE "am_row"."id" = $2"#
    );
}

// [spec:pgorm:sem:entity.active-model.active-value.ops/test]    the accessor
// surface: `set` / `unchanged` / `not_set` constructors, the matching
// `is_set` / `is_unchanged` / `is_not_set` predicates, `take`, `try_as_ref`,
// `as_ref`, `into_value`, `into_wrapped_value`, `reset` and `set_if_not_equals`
#[test]
fn active_value_accessors() {
    // Constructors line up with the variants.
    assert_eq!(ActiveValue::set(1), ActiveValue::set(1));
    assert_eq!(ActiveValue::unchanged(1), ActiveValue::Unchanged(1));
    assert_eq!(ActiveValue::<i32>::not_set(), ActiveValue::NotSet);

    // Predicates mirror the variants exactly, one true apiece.
    let set = ActiveValue::set(1);
    assert!(set.is_set() && !set.is_unchanged() && !set.is_not_set());
    let unchanged = ActiveValue::unchanged(1);
    assert!(!unchanged.is_set() && unchanged.is_unchanged() && !unchanged.is_not_set());
    let not_set = ActiveValue::<i32>::not_set();
    assert!(!not_set.is_set() && !not_set.is_unchanged() && not_set.is_not_set());

    // `take` yields Some for Set and Unchanged, None for NotSet, and always
    // leaves NotSet behind.
    let mut v = ActiveValue::set(1);
    assert_eq!(v.take(), Some(1));
    assert_eq!(v, ActiveValue::NotSet);

    let mut v = ActiveValue::Unchanged(2);
    assert_eq!(v.take(), Some(2));
    assert_eq!(v, ActiveValue::NotSet);

    let mut v = ActiveValue::<i32>::NotSet;
    assert_eq!(v.take(), None);
    assert_eq!(v, ActiveValue::NotSet);

    // `unwrap` returns the inner value for both populated variants.
    assert_eq!(ActiveValue::set(1).unwrap(), 1);
    assert_eq!(ActiveValue::Unchanged(2).unwrap(), 2);

    // `try_as_ref` is the non-panicking borrow; `as_ref` is the panicking one.
    assert_eq!(ActiveValue::set(1).try_as_ref(), Some(&1));
    assert_eq!(ActiveValue::Unchanged(1).try_as_ref(), Some(&1));
    assert_eq!(ActiveValue::<i32>::NotSet.try_as_ref(), None);
    assert_eq!(std::convert::AsRef::as_ref(&ActiveValue::set(1)), &1);
    assert_eq!(std::convert::AsRef::as_ref(&ActiveValue::Unchanged(1)), &1);

    // `into_value` erases the variant, keeping only presence.
    assert_eq!(ActiveValue::set(1).into_value(), Some(Value::Int(Some(1))));
    assert_eq!(
        ActiveValue::Unchanged(1).into_value(),
        Some(Value::Int(Some(1)))
    );
    assert_eq!(ActiveValue::<i32>::NotSet.into_value(), None);

    // `into_wrapped_value` keeps the variant while widening to ActiveValue<Value>.
    assert_eq!(
        ActiveValue::set(1).into_wrapped_value(),
        ActiveValue::set(Value::Int(Some(1)))
    );
    assert_eq!(
        ActiveValue::Unchanged(1).into_wrapped_value(),
        ActiveValue::Unchanged(Value::Int(Some(1)))
    );
    assert_eq!(
        ActiveValue::<i32>::NotSet.into_wrapped_value(),
        ActiveValue::NotSet
    );

    // `reset` promotes Unchanged -> Set, is a no-op on Set, leaves NotSet alone.
    let mut v = ActiveValue::Unchanged(1);
    v.reset();
    assert_eq!(v, ActiveValue::set(1));
    let mut v = ActiveValue::set(1);
    v.reset();
    assert_eq!(v, ActiveValue::set(1));
    let mut v = ActiveValue::<i32>::NotSet;
    v.reset();
    assert_eq!(v, ActiveValue::NotSet);
}

// [spec:pgorm:sem:entity.active-model.active-value.ops/test]    `set_if_not_equals`
// only stays quiet for `Unchanged` with an equal payload, so `is_changed` on the
// surrounding ActiveModel tracks actual differences rather than assignments
#[test]
fn active_value_set_if_not_equals() {
    // Unchanged + equal payload: no transition.
    let mut v = ActiveValue::Unchanged("old");
    v.set_if_not_equals("old");
    assert!(v.is_unchanged());
    assert_eq!(v, ActiveValue::Unchanged("old"));

    // Unchanged + different payload: becomes Set.
    v.set_if_not_equals("new");
    assert_eq!(v, set("new"));

    // `Set` with an equal payload is still re-Set — only `Unchanged` is special.
    let mut v = set("same");
    v.set_if_not_equals("same");
    assert_eq!(v, set("same"));

    // NotSet always becomes Set.
    let mut v = ActiveValue::<&str>::NotSet;
    v.set_if_not_equals("x");
    assert_eq!(v, set("x"));

    // The whole-ActiveModel consequence: writing back an identical value leaves
    // `is_changed` false, writing a genuinely new one flips it.
    let mut am = row::Model {
        id: 1,
        name: "Apple".to_owned(),
        note: None,
    }
    .into_active_model();
    assert!(!am.is_changed());
    am.name.set_if_not_equals("Apple".to_owned());
    assert!(!am.is_changed());
    am.name.set_if_not_equals("Pear".to_owned());
    assert!(am.is_changed());
}

// [spec:pgorm:sem:entity.active-model.active-value.ops/test]    `unwrap` panics on NotSet
#[test]
#[should_panic(expected = "Cannot unwrap ActiveValue::NotSet")]
fn active_value_unwrap_panics_on_not_set() {
    let _ = ActiveValue::<i32>::NotSet.unwrap();
}

// [spec:pgorm:sem:entity.active-model.active-value.ops/test]    `as_ref` panics on NotSet
#[test]
#[should_panic(expected = "Cannot borrow ActiveValue::NotSet")]
fn active_value_as_ref_panics_on_not_set() {
    let not_set = ActiveValue::<i32>::NotSet;
    let _: &i32 = std::convert::AsRef::as_ref(&not_set);
}

// ---------------------------------------------------------------------------
// From-sugar
// ---------------------------------------------------------------------------

// [spec:pgorm:req:entity.active-model.from-sugar+1/test]    the blanket
// `From<V> for ActiveValue<V>` produces `Set` (pgorm's divergence from upstream
// SeaORM), and `From<ActiveValue<V>> for ActiveValue<Option<V>>` lifts into a
// nullable column position preserving the variant
#[test]
fn active_value_from_sugar() {
    // A plain value converts straight into the `Set` state.
    let v: ActiveValue<i32> = 1.into();
    assert_eq!(v, set(1));
    let v: ActiveValue<String> = "Apple".to_owned().into();
    assert_eq!(v, set("Apple"));
    let v: ActiveValue<bool> = true.into();
    assert_eq!(v, set(true));

    // Which is what makes this ActiveModel literal legal without `set(...)`.
    let am = row::ActiveModel {
        id: NotSet,
        name: "Apple".to_owned().into(),
        note: Some("crisp".to_owned()).into(),
    };
    assert_eq!(
        am,
        row::ActiveModel {
            id: NotSet,
            name: set("Apple"),
            note: "crisp".into(),
        }
    );

    // Lifting into a nullable position preserves the variant in all three cases.
    let lifted: ActiveValue<Option<i32>> = ActiveValue::set(1).into();
    assert_eq!(lifted, ActiveValue::set(Some(1)));
    let lifted: ActiveValue<Option<i32>> = ActiveValue::Unchanged(1).into();
    assert_eq!(lifted, ActiveValue::Unchanged(Some(1)));
    let lifted: ActiveValue<Option<i32>> = ActiveValue::<i32>::NotSet.into();
    assert_eq!(lifted, ActiveValue::NotSet);
}

// [spec:pgorm:req:entity.active-model.from-sugar+1/test]    the three targeted
// borrowed-to-owned conversions the blanket cannot express, and that they cohere
// with it: `&str` still reaches `ActiveValue<&str>` when that is what is asked for
#[test]
fn active_value_borrowed_from_impls() {
    let owned: ActiveValue<String> = "Apple".into();
    assert_eq!(owned, ActiveValue::Set("Apple".to_owned()));

    let nullable: ActiveValue<Option<String>> = "Apple".into();
    assert_eq!(nullable, ActiveValue::Set(Some("Apple".to_owned())));

    let bytes: ActiveValue<Vec<u8>> = (&b"hi"[..]).into();
    assert_eq!(bytes, ActiveValue::Set(b"hi".to_vec()));

    // The blanket did not lose its `&str` case to the new impls: each of them
    // names a target the blanket does not produce for the same source.
    let borrowed: ActiveValue<&str> = "Apple".into();
    assert_eq!(borrowed, ActiveValue::Set("Apple"));
}

// [spec:pgorm:req:entity.active-model.from-sugar+1/test]    the free `set(..)`
// applies `Into<V>` at the call site, so a literal reaches an owning column with
// no `.to_owned()`, while `ActiveValue::set` and the `Set` variant pin `V` exactly
// [spec:pgorm:def:entity.active-model.active-value+1/test]    which is why `set` is
// the documented construction spelling and `Set` the pattern-matching one
#[test]
fn active_value_set_free_function() {
    // The conversion happens at the call site: `&str` into a `String` column.
    let am = row::ActiveModel {
        id: set(1),
        name: set("Apple"),
        note: NotSet,
    };
    assert_eq!(am.name, ActiveValue::Set("Apple".to_owned()));

    // Reflexively, it is the plain constructor: an argument already of the field
    // type goes through `Into`'s identity impl.
    assert_eq!(set::<String, _>("Apple".to_owned()), am.name);
    assert_eq!(set::<i32, _>(1), am.id);

    // And the state it produces is `Set`, matchable as the variant.
    assert!(matches!(am.id, ActiveValue::Set(1)));
}

// ---------------------------------------------------------------------------
// IntoActiveModel / IntoActiveValue
// ---------------------------------------------------------------------------

mod row_dto {
    pub use super::row::*;
    use pgorm::entity::prelude::*;

    /// `Option<V>` fields: `Some` writes, `None` leaves the column alone.
    #[derive(IntoActiveModel)]
    pub struct NewRow {
        pub name: String,
        pub note: Option<String>,
    }

    /// `Option<Option<V>>` fields: the extra rung that can write an explicit NULL.
    #[derive(IntoActiveModel)]
    pub struct UpdateRow {
        pub note: Option<Option<String>>,
    }
}
use row_dto::{NewRow, UpdateRow};

// [spec:pgorm:req:entity.active-model.into+3/test]    the blanket identity
// `IntoActiveModel` impl, the derived `Model -> ActiveModel` conversion putting
// every field in `Unchanged`, and the `IntoActiveValue` state mapping for
// `Option<V>`, `Option<Option<V>>` and the plain scalar impls
#[test]
fn into_active_model_and_into_active_value() {
    // Blanket identity impl for anything that is already an ActiveModel.
    let am = row::ActiveModel {
        id: set(1),
        name: set("Apple"),
        note: NotSet,
    };
    assert_eq!(am.clone().into_active_model(), am);

    // A derived Model converts with every field `Unchanged` — including the
    // nullable column holding `None`, which is `Unchanged(None)`, not `NotSet`.
    assert_eq!(
        row::Model {
            id: 1,
            name: "Apple".to_owned(),
            note: None,
        }
        .into_active_model(),
        row::ActiveModel {
            id: ActiveValue::Unchanged(1),
            name: ActiveValue::Unchanged("Apple".to_owned()),
            note: ActiveValue::Unchanged(None),
        }
    );

    // `IntoActiveValue` for plain scalars always yields `Set`.
    assert_eq!(true.into_active_value(), set(true));
    assert_eq!(1i8.into_active_value(), set(1i8));
    assert_eq!(1i16.into_active_value(), set(1i16));
    assert_eq!(1i32.into_active_value(), set(1i32));
    assert_eq!(1i64.into_active_value(), set(1i64));
    assert_eq!(1u32.into_active_value(), set(1u32));
    assert_eq!(1u64.into_active_value(), set(1u64));
    assert_eq!(1.5f32.into_active_value(), set(1.5f32));
    assert_eq!(1.5f64.into_active_value(), set(1.5f64));
    assert_eq!("s".into_active_value(), set("s"));
    assert_eq!("s".to_owned().into_active_value(), set("s"));
    assert_eq!(vec![1u8].into_active_value(), set(vec![1u8]));

    // `Option<V>`: Some -> set(Some(v)), None -> NotSet. There is deliberately
    // no way to spell "write NULL" through this impl.
    assert_eq!(Some(1i32).into_active_value(), set(Some(1i32)));
    assert_eq!(
        None::<i32>.into_active_value(),
        ActiveValue::<Option<i32>>::NotSet
    );

    // `Option<Option<V>>` adds the missing rung: Some(None) -> set(None) nulls
    // the column, None -> NotSet leaves it alone.
    assert_eq!(Some(Some(1i32)).into_active_value(), set(Some(1i32)));
    assert_eq!(Some(None::<i32>).into_active_value(), set(None::<i32>));
    assert_eq!(
        None::<Option<i32>>.into_active_value(),
        ActiveValue::<Option<i32>>::NotSet
    );

    // Which is exactly what `IntoActiveModel` puts to work.
    assert_eq!(
        NewRow {
            name: "Apple".to_owned(),
            note: Some("crisp".to_owned()),
        }
        .into_active_model(),
        row::ActiveModel {
            id: NotSet,
            name: set("Apple"),
            note: "crisp".into(),
        }
    );
    assert_eq!(
        NewRow {
            name: "Apple".to_owned(),
            note: None,
        }
        .into_active_model(),
        row::ActiveModel {
            id: NotSet,
            name: set("Apple"),
            note: NotSet,
        }
    );
    assert_eq!(
        UpdateRow { note: Some(None) }.into_active_model(),
        row::ActiveModel {
            id: NotSet,
            name: NotSet,
            note: set(None),
        }
    );
    assert_eq!(
        UpdateRow { note: None }.into_active_model(),
        row::ActiveModel {
            id: NotSet,
            name: NotSet,
            note: NotSet,
        }
    );
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

// [spec:pgorm:req:entity.active-model.json+4/test]    `from_json` normalises per
// column — keys present in the JSON object become `Set`, everything else `NotSet`
// — and a deserialization failure surfaces as `Error`
#[test]
fn active_model_from_json() {
    use serde_json::json;

    // Every key present -> every column Set.
    assert_eq!(
        row::ActiveModel::from_json(json!({
            "id": 1,
            "name": "Apple",
            "note": "crisp",
        }))
        .unwrap(),
        row::ActiveModel {
            id: set(1),
            name: set("Apple"),
            note: "crisp".into(),
        }
    );

    // An absent key is NotSet, even though the Model itself had to be built with
    // some value for it. An explicit JSON null is present, so it is set(None).
    assert_eq!(
        row::ActiveModel::from_json(json!({
            "id": 1,
            "name": "Apple",
            "note": null,
        }))
        .unwrap(),
        row::ActiveModel {
            id: set(1),
            name: set("Apple"),
            note: set(None),
        }
    );

    // A missing required field is a deserialization error surfaced as Error.
    let err = row::ActiveModel::from_json(json!({ "id": 1 })).unwrap_err();
    assert!(
        matches!(err, Error::Json(_)),
        "expected Error::Json, got {err:?}"
    );
}

// [spec:pgorm:req:entity.active-model.json+4/test]    `set_from_json` applies the
// same normalisation in place but MUST NOT alter the primary key: the key states
// are taken before the overwrite and restored after, so `Set` / `Unchanged`
// payloads survive and `NotSet` stays `NotSet` whatever the JSON said
#[test]
fn active_model_set_from_json_preserves_primary_key() {
    use serde_json::json;

    // Starting from NotSet: the JSON's "id" is ignored, id stays NotSet.
    let mut am = <row::ActiveModel as ActiveModelTrait>::default();
    am.set_from_json(json!({
        "id": 99,
        "name": "Apple",
        "note": "crisp",
    }))
    .unwrap();
    assert_eq!(
        am,
        row::ActiveModel {
            id: NotSet,
            name: set("Apple"),
            note: "crisp".into(),
        }
    );

    // Starting from set(1): the original key value survives the overwrite.
    let mut am = row::ActiveModel {
        id: set(1),
        name: NotSet,
        note: NotSet,
    };
    am.set_from_json(json!({
        "id": 99,
        "name": "Apple",
    }))
    .unwrap();
    assert_eq!(
        am,
        row::ActiveModel {
            id: set(1),
            name: set("Apple"),
            note: NotSet,
        }
    );

    // Starting from Unchanged(1): the value survives too. It is restored via
    // `set`, so the state lands on `Set` — the payload is what is preserved.
    let mut am = row::ActiveModel {
        id: ActiveValue::Unchanged(1),
        name: NotSet,
        note: NotSet,
    };
    am.set_from_json(json!({
        "id": 99,
        "name": "Apple",
    }))
    .unwrap();
    assert_eq!(am.id.into_value(), Some(Value::Int(Some(1))));
    assert_eq!(am.name, set("Apple"));
}

// [spec:pgorm:req:entity.active-model.json+4/test]    a `set_from_json` that
// fails leaves the model bit-for-bit as it was — primary key included. The
// conversion happens on a fresh model and only a conversion that succeeded is
// moved into `self`, so nothing partial can be observed
#[test]
fn set_from_json_failure_leaves_the_model_untouched() {
    use serde_json::json;

    let untouched = row::ActiveModel {
        id: set(1),
        name: set("Apple"),
        note: ActiveValue::Unchanged(Some("crisp".to_owned())),
    };

    // A type mismatch: `name` is a String, and no amount of primary-key
    // bookkeeping happens before the deserializer says so.
    let mut am = untouched.clone();
    let err = am
        .set_from_json(json!({ "id": 99, "name": 7, "note": "tart" }))
        .unwrap_err();
    assert!(
        matches!(err, Error::Json(_)),
        "expected Error::Json, got {err:?}"
    );
    assert_eq!(am, untouched);

    // A missing required field fails the same way, with the same guarantee: the
    // primary key that would name this row for a subsequent `update` survives.
    let mut am = untouched.clone();
    assert!(am.set_from_json(json!({ "note": "tart" })).is_err());
    assert_eq!(am, untouched);
    assert_eq!(am.id, set(1));
}

// [spec:pgorm:req:entity.active-model.json+4/test]    presence detection reads
// `serde`'s key namespace, not the SQL column names: a model's own serialization
// round-trips back through `from_json` / `set_from_json` with every column `Set`,
// whichever side renamed what
// [spec:pgorm:def:entity.traits.column+7/test]    which is what `json_key`
// reports, next to the SQL name `as_str` reports
#[test]
fn active_model_json_reads_serde_keys() {
    // `#[pgorm(column_name)]` moves the SQL name and leaves the `serde` key put.
    assert_eq!(sql_renamed::Column::CreatedAt.as_str(), "createdAt");
    assert_eq!(sql_renamed::Column::CreatedAt.json_key(), "created_at");

    let model = sql_renamed::Model {
        id: 1,
        created_at: "2026-01-01".to_owned(),
        note: Some("crisp".to_owned()),
    };
    let serialized = serde_json::to_value(&model).unwrap();
    assert_eq!(
        sql_renamed::ActiveModel::from_json(serialized.clone()).unwrap(),
        sql_renamed::ActiveModel {
            id: set(1),
            created_at: set("2026-01-01"),
            note: set(Some("crisp".to_owned())),
        }
    );

    // In place, the primary key aside — nothing is dropped to `NotSet`.
    let mut am = sql_renamed::ActiveModel {
        id: set(7),
        created_at: NotSet,
        note: NotSet,
    };
    am.set_from_json(serialized).unwrap();
    assert_eq!(
        am,
        sql_renamed::ActiveModel {
            id: set(7),
            created_at: set("2026-01-01"),
            note: set(Some("crisp".to_owned())),
        }
    );

    // `#[serde(rename_all)]` and `#[serde(rename)]` move the key the other way,
    // and the SQL names stay where the entity put them.
    assert_eq!(serde_renamed::Column::CreatedAt.as_str(), "created_at");
    assert_eq!(serde_renamed::Column::CreatedAt.json_key(), "createdAt");
    assert_eq!(serde_renamed::Column::Note.as_str(), "note");
    assert_eq!(serde_renamed::Column::Note.json_key(), "NOTE");

    let model = serde_renamed::Model {
        id: 2,
        created_at: "2026-02-02".to_owned(),
        note: None,
    };
    let serialized = serde_json::to_value(&model).unwrap();
    assert_eq!(
        serde_renamed::ActiveModel::from_json(serialized).unwrap(),
        serde_renamed::ActiveModel {
            id: set(2),
            created_at: set("2026-02-02"),
            note: set(None),
        }
    );

    // An entity that renames nothing is unaffected: the two namespaces agree.
    for col in row::Column::iter() {
        assert_eq!(col.as_str(), col.json_key());
    }
    let model = row::Model {
        id: 3,
        name: "Apple".to_owned(),
        note: None,
    };
    assert_eq!(
        row::ActiveModel::from_json(serde_json::to_value(&model).unwrap()).unwrap(),
        row::ActiveModel {
            id: set(3),
            name: set("Apple"),
            note: set(None),
        }
    );
}

// ---------------------------------------------------------------------------
// Persistence (DB-backed)
// ---------------------------------------------------------------------------

async fn create_row_table<C>(db: &C) -> Result<(), Error>
where
    C: ConnectionTrait,
{
    let stmt = Schema::new().create_table_from_entity(row::Entity)?;
    db.execute(&stmt.to_string(), &[]).await?;
    Ok(())
}

// [spec:pgorm:req:entity.active-model.persistence+2/test]    `insert` comes back
// with the database-generated key populated (the single `INSERT ... RETURNING`
// round trip), `update` returns the freshly written `Model`, and `delete`
// returns the rows-affected count, keyed on the primary key
#[pgorm_macros::test]
async fn active_model_persistence() -> Result<(), Error> {
    let ctx = TestContext::new("active_model_persistence").await;
    let db = ctx.db.get().await?;
    create_row_table(&db).await?;

    // The client never supplies `id`; it comes back from the RETURNING clause of
    // the very same statement, fully populated alongside every other column.
    let inserted = row::ActiveModel {
        id: NotSet,
        name: set("Apple"),
        note: "crisp".into(),
    }
    .insert(&db)
    .await?;
    assert!(inserted.id > 0, "primary key must come back from RETURNING");
    assert_eq!(inserted.name, "Apple");
    assert_eq!(inserted.note, Some("crisp".to_owned()));

    // A second insert takes the next sequence value: the first really did read
    // back what the database generated rather than echoing the input.
    let second = row::ActiveModel {
        id: NotSet,
        name: set("Pear"),
        note: NotSet,
    }
    .insert(&db)
    .await?;
    assert_eq!(second.id, inserted.id + 1);
    // `note` was NotSet, so the column defaulted to NULL rather than being written.
    assert_eq!(second.note, None);

    // `update` is keyed on the primary key and returns the fresh Model.
    let mut am = inserted.clone().into_active_model();
    am.name = set("Apple Pie");
    let updated = am.update(&db).await?;
    assert_eq!(
        updated,
        row::Model {
            id: inserted.id,
            name: "Apple Pie".to_owned(),
            note: Some("crisp".to_owned()),
        }
    );
    // And the row on disk really changed.
    assert_eq!(
        row::Entity::find_by_id(inserted.id).one(&db).await?,
        updated
    );

    // `delete` reports the rows removed and removes exactly the keyed row.
    let res: u64 = updated.clone().into_active_model().delete(&db).await?;
    assert_eq!(res, 1);
    assert_eq!(
        row::Entity::find_by_id(inserted.id).one_opt(&db).await?,
        None
    );
    assert_eq!(row::Entity::find().all(&db).await?.len(), 1);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:entity.active-model.save+1/test]    the operation is the
// caller's choice, not a reading of the model: `update` accepts a primary key in
// either the `Set` or the `Unchanged` state, the two states an inference would
// have had to tell apart
// [spec:pgorm:req:entity.active-model.persistence+2/test]
#[pgorm_macros::test]
async fn update_accepts_set_or_unchanged_key() -> Result<(), Error> {
    let ctx = TestContext::new("update_accepts_set_or_unchanged_key").await;
    let db = ctx.db.get().await?;
    create_row_table(&db).await?;

    let inserted = row::ActiveModel {
        id: NotSet,
        name: set("Apple"),
        note: NotSet,
    }
    .insert(&db)
    .await?;
    let id = inserted.id;

    // An `Unchanged` key, as it comes back from `IntoActiveModel`.
    let mut am = inserted.into_active_model();
    assert_eq!(am.id, ActiveValue::Unchanged(id));
    am.name = set("Apple Pie");
    let updated = am.update(&db).await?;
    assert_eq!(updated.name, "Apple Pie");

    // A `Set` key, as a caller writes it by hand. Same statement, same row.
    let updated = row::ActiveModel {
        id: set(id),
        name: set("Apple Tart"),
        note: NotSet,
    }
    .update(&db)
    .await?;
    assert_eq!(updated.name, "Apple Tart");

    // Neither wrote a second row.
    assert_eq!(
        row::Entity::find().all(&db).await?,
        [row::Model {
            id,
            name: "Apple Tart".to_owned(),
            note: None,
        }]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:entity.active-model.save+1/test]    a manually keyed entity
// creates rows through `insert` — the case an inference over the key state could
// never reach, since the caller has to populate the key to name the row at all —
// while `update` against a key that matches nothing still fails
#[pgorm_macros::test]
async fn manual_key_insert_and_update_are_explicit() -> Result<(), Error> {
    let ctx = TestContext::new("manual_key_insert_and_update").await;
    let db = ctx.db.get().await?;
    let stmt = Schema::new().create_table_from_entity(manual_key::Entity)?;
    db.execute(&stmt.to_string(), &[]).await?;

    // `update` on a key that matches no row reports it rather than creating one.
    let err = manual_key::ActiveModel {
        id: set(1),
        name: set("Apple"),
    }
    .update(&db)
    .await
    .unwrap_err();
    assert!(
        matches!(err, Error::RecordNotFound),
        "expected RecordNotFound, got {err:?}"
    );
    assert_eq!(manual_key::Entity::find().all(&db).await?.len(), 0);

    // The caller states the intent, so a populated manual key inserts.
    let inserted = manual_key::ActiveModel {
        id: set(1),
        name: set("Apple"),
    }
    .insert(&db)
    .await?;
    assert_eq!(
        inserted,
        manual_key::Model {
            id: 1,
            name: "Apple".to_owned(),
        }
    );

    // And the same key now updates the row it names.
    let updated = manual_key::ActiveModel {
        id: set(1),
        name: set("Apple Pie"),
    }
    .update(&db)
    .await?;
    assert_eq!(updated.name, "Apple Pie");
    assert_eq!(manual_key::Entity::find().all(&db).await?.len(), 1);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Behavior hooks (DB-backed)
// ---------------------------------------------------------------------------

static HOOK_LOG: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn hook_log() -> Vec<String> {
    HOOK_LOG.lock().unwrap().clone()
}

fn record(entry: &str) {
    HOOK_LOG.lock().unwrap().push(entry.to_owned());
}

mod hooked {
    use super::record;
    use pgorm::entity::prelude::async_trait;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "am_hooked")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    #[async_trait::async_trait]
    impl ActiveModelBehavior for ActiveModel {
        async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, Error>
        where
            C: ConnectionTrait,
        {
            record(&format!("before_save(insert={insert})"));
            Ok(self)
        }

        async fn after_save<C>(
            model: <Self::Entity as EntityTrait>::Model,
            _db: &C,
            insert: bool,
        ) -> Result<<Self::Entity as EntityTrait>::Model, Error>
        where
            C: ConnectionTrait,
        {
            record(&format!("after_save(insert={insert}, id={})", model.id));
            Ok(model)
        }

        async fn before_delete<C>(self, _db: &C) -> Result<Self, Error>
        where
            C: ConnectionTrait,
        {
            record("before_delete");
            Ok(self)
        }

        async fn after_delete<C>(self, _db: &C) -> Result<Self, Error>
        where
            C: ConnectionTrait,
        {
            record(&format!("after_delete(name_set={})", self.name.is_set()));
            Ok(self)
        }
    }
}

// [spec:pgorm:req:entity.active-model.hooks+1/test]    the fixed hook ordering:
// `insert` calls `before_save(.., true)` then `after_save(model, .., true)`;
// `update` runs the same pair with `insert: false`; `delete` calls
// `before_delete` then `after_delete` on a clone of the pre-delete active model
#[pgorm_macros::test]
async fn active_model_hook_ordering() -> Result<(), Error> {
    let ctx = TestContext::new("active_model_hook_ordering").await;
    let db = ctx.db.get().await?;
    let stmt = Schema::new().create_table_from_entity(hooked::Entity)?;
    db.execute(&stmt.to_string(), &[]).await?;

    HOOK_LOG.lock().unwrap().clear();

    let model = hooked::ActiveModel {
        id: NotSet,
        name: set("Apple"),
    }
    .insert(&db)
    .await?;
    assert_eq!(
        hook_log(),
        [
            "before_save(insert=true)".to_owned(),
            format!("after_save(insert=true, id={})", model.id),
        ]
    );

    HOOK_LOG.lock().unwrap().clear();
    let mut am = model.clone().into_active_model();
    am.name = set("Apple Pie");
    am.update(&db).await?;
    assert_eq!(
        hook_log(),
        [
            "before_save(insert=false)".to_owned(),
            format!("after_save(insert=false, id={})", model.id),
        ]
    );

    // `delete` hands `after_delete` a clone of the active model as it stood
    // before the delete, so its attributes are still populated.
    HOOK_LOG.lock().unwrap().clear();
    let res = hooked::ActiveModel {
        id: set(model.id),
        name: set("Apple Pie"),
    }
    .delete(&db)
    .await?;
    assert_eq!(res, 1);
    assert_eq!(
        hook_log(),
        [
            "before_delete".to_owned(),
            "after_delete(name_set=true)".to_owned(),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

mod hook_abort {
    use pgorm::entity::prelude::async_trait;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "am_hook_abort")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    #[async_trait::async_trait]
    impl ActiveModelBehavior for ActiveModel {
        async fn before_save<C>(self, _db: &C, _insert: bool) -> Result<Self, Error>
        where
            C: ConnectionTrait,
        {
            Err(Error::Custom("before_save refused".to_owned()))
        }

        async fn before_delete<C>(self, _db: &C) -> Result<Self, Error>
        where
            C: ConnectionTrait,
        {
            Err(Error::Custom("before_delete refused".to_owned()))
        }
    }
}

// [spec:pgorm:req:entity.active-model.hooks+1/test]    an `Err` from a *before*
// hook aborts the operation: nothing is written, nothing is removed. Also pins the
// pass-through defaults and `new()` delegating to `ActiveModelTrait::default()`
#[pgorm_macros::test]
async fn active_model_hook_error_aborts() -> Result<(), Error> {
    let ctx = TestContext::new("active_model_hook_error_aborts").await;
    let db = ctx.db.get().await?;
    let stmt = Schema::new().create_table_from_entity(hook_abort::Entity)?;
    db.execute(&stmt.to_string(), &[]).await?;

    // `insert` aborts before executing.
    let err = hook_abort::ActiveModel {
        id: NotSet,
        name: set("Apple"),
    }
    .insert(&db)
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "Custom Error: before_save refused");
    assert_eq!(hook_abort::Entity::find().all(&db).await?.len(), 0);

    // Seed a row through the statement API, which does not run hooks.
    Insert::one(hook_abort::ActiveModel {
        id: NotSet,
        name: set("Apple"),
    })
    .exec(&db)
    .await?;
    let seeded = hook_abort::Entity::find().one(&db).await?;

    // `update` aborts too — `before_save` is shared by both paths.
    let err = hook_abort::ActiveModel {
        id: ActiveValue::Unchanged(seeded.id),
        name: set("Apple Pie"),
    }
    .update(&db)
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "Custom Error: before_save refused");
    assert_eq!(hook_abort::Entity::find().one(&db).await?.name, "Apple");

    // `delete` aborts before executing, so the row survives.
    let err = hook_abort::ActiveModel {
        id: set(seeded.id),
        name: NotSet,
    }
    .delete(&db)
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "Custom Error: before_delete refused");
    assert_eq!(hook_abort::Entity::find().all(&db).await?.len(), 1);

    drop(db);
    ctx.delete().await;
    Ok(())
}

mod after_hook_abort {
    use pgorm::entity::prelude::async_trait;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "am_after_hook_abort")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    #[async_trait::async_trait]
    impl ActiveModelBehavior for ActiveModel {
        async fn after_save<C>(_model: Model, _db: &C, _insert: bool) -> Result<Model, Error>
        where
            C: ConnectionTrait,
        {
            Err(Error::Custom("after_save refused".to_owned()))
        }

        async fn after_delete<C>(self, _db: &C) -> Result<Self, Error>
        where
            C: ConnectionTrait,
        {
            Err(Error::Custom("after_delete refused".to_owned()))
        }
    }
}

// [spec:pgorm:req:entity.active-model.hooks+1/test]    an `Err` from an *after*
// hook is not an abort: the statement already ran on the connection it was
// handed, so the row is written (or gone) and the caller gets the `Err` anyway.
// A caller who needs the two to stand or fall together supplies a transaction
// and lets the `Err` reach the rollback
#[pgorm_macros::test]
async fn active_model_after_hook_error_keeps_the_write() -> Result<(), Error> {
    let ctx = TestContext::new("active_model_after_hook_error").await;
    let mut db = ctx.db.get().await?;
    let stmt = Schema::new().create_table_from_entity(after_hook_abort::Entity)?;
    db.execute(&stmt.to_string(), &[]).await?;

    // `insert` returns the hook's `Err` — with the row already inserted.
    let err = after_hook_abort::ActiveModel {
        id: NotSet,
        name: set("Apple"),
    }
    .insert(&db)
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "Custom Error: after_save refused");
    let written = after_hook_abort::Entity::find().all(&db).await?;
    assert_eq!(written.len(), 1, "the insert is not rolled back");
    assert_eq!(written[0].name, "Apple");

    // `update` likewise: `after_save` refuses, the new value is already stored.
    let err = after_hook_abort::ActiveModel {
        id: ActiveValue::Unchanged(written[0].id),
        name: set("Apple Pie"),
    }
    .update(&db)
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "Custom Error: after_save refused");
    assert_eq!(
        after_hook_abort::Entity::find().one(&db).await?.name,
        "Apple Pie"
    );

    // And `after_delete` does not bring the row back.
    let err = after_hook_abort::ActiveModel {
        id: set(written[0].id),
        name: NotSet,
    }
    .delete(&db)
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "Custom Error: after_delete refused");
    assert_eq!(after_hook_abort::Entity::find().all(&db).await?.len(), 0);

    // The atomicity the hooks do not provide is the caller's to ask for: run the
    // operation in a transaction and the `Err` leaves it uncommitted.
    let outcome = db
        .transaction(async |txn| {
            after_hook_abort::ActiveModel {
                id: NotSet,
                name: set("Pear"),
            }
            .insert(&txn)
            .await
        })
        .await;
    match outcome {
        Err(TransactionError::Transaction(err)) => {
            assert_eq!(err.to_string(), "Custom Error: after_save refused")
        }
        other => panic!("expected the hook's refusal, got {other:?}"),
    }
    assert_eq!(after_hook_abort::Entity::find().all(&db).await?.len(), 0);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:entity.active-model.hooks+1/test]    `new()` defaults to
// `ActiveModelTrait::default()`, and the un-overridden hooks are pass-throughs —
// `row::ActiveModel` overrides nothing and still inserts, updates and deletes
#[test]
fn active_model_behavior_new_defaults() {
    let new = <row::ActiveModel as ActiveModelBehavior>::new();
    assert_eq!(new, <row::ActiveModel as ActiveModelTrait>::default());
    assert_eq!(
        new,
        row::ActiveModel {
            id: NotSet,
            name: NotSet,
            note: NotSet,
        }
    );
    // `Default::default()` is wired to the same place.
    assert_eq!(new, <row::ActiveModel as std::default::Default>::default());
}
