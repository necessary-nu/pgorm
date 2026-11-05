//! A form deriving `IntoActiveModel` in a crate other than its entity's,
//! naming the entity's active model by path, with fields of every column
//! value type a derive makes.
#![allow(dead_code)]

use pgorm::entity::prelude::*;
use pgorm::tests_cfg::{active_enums::Tea, cake, lunch_set};
use pgorm::{JsonValueType, set};
use serde::{Deserialize, Serialize};

/// A form for an entity another crate declares.
// [spec:pgorm:sem:macros.derive.active-model+8/test]    a form in another crate
// converts into the entity's active model, named by path
#[derive(IntoActiveModel)]
#[pgorm(active_model = cake::ActiveModel)]
struct CakeForm {
    name: String,
}

/// The same, over an `ActiveEnum` column.
#[derive(IntoActiveModel)]
#[pgorm(active_model = lunch_set::ActiveModel)]
struct LunchForm {
    name: String,
    tea: Tea,
}

mod parcel {
    use super::{Deserialize, JsonValueType, Serialize, Tea};
    use pgorm::entity::prelude::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, ValueType)]
    pub struct Grams(pub i32);

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonValueType)]
    pub struct Label {
        pub text: String,
    }

    #[derive(Clone, Debug, PartialEq, CreatedRange)]
    #[pgorm(range_name = "floatrange")]
    pub struct Span(pub Range<f64>);

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "parcel")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub weight: Grams,
        pub label: Label,
        pub span: Span,
        pub tea: Option<Tea>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A form whose fields are of the types the value derives make.
// [spec:pgorm:sem:macros.derive.active-model+8/test]    a field of an
// ActiveEnum, ValueType, JsonValueType or CreatedRange type sets its column
#[derive(IntoActiveModel)]
#[pgorm(active_model = parcel::ActiveModel)]
struct ParcelForm {
    weight: parcel::Grams,
    label: parcel::Label,
    span: parcel::Span,
    tea: Option<Tea>,
}

#[test]
fn a_form_converts_from_another_crate() {
    let cake = CakeForm {
        name: "Apple".to_owned(),
    }
    .into_active_model();
    assert_eq!(cake.name, set("Apple"));
    assert!(cake.id.is_not_set());

    let lunch = LunchForm {
        name: "Elevenses".to_owned(),
        tea: Tea::BreakfastTea,
    }
    .into_active_model();
    assert_eq!(lunch.tea, set(Tea::BreakfastTea));
}

#[test]
fn a_form_carries_every_derived_value_type() {
    let span = parcel::Span(Range::Bounds {
        lower: std::ops::Bound::Included(1.0),
        upper: std::ops::Bound::Excluded(2.0),
    });
    let parcel = ParcelForm {
        weight: parcel::Grams(250),
        label: parcel::Label {
            text: "fragile".to_owned(),
        },
        span: span.clone(),
        tea: None,
    }
    .into_active_model();
    assert_eq!(parcel.weight, set(parcel::Grams(250)));
    assert_eq!(
        parcel.label,
        set(parcel::Label {
            text: "fragile".to_owned()
        })
    );
    assert_eq!(parcel.span, set(span));
    assert!(parcel.tea.is_not_set(), "a `None` leaves its column out");
    assert!(parcel.id.is_not_set());
}
