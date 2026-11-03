//! What `#[derive(Disjoint)]` declares: the kind type and its values, the
//! kinds' entities in declaration order with their keys, the decoding of a
//! kind's model into its variant, and an `<Enum>ActiveModel` taken apart.

use pgorm::entity::prelude::*;
use pgorm::tests_cfg::{
    car, truck,
    vehicle::{self, Vehicle, VehicleActiveModel, VehicleKind},
};
use pgorm::{ActiveKindVisitor, DisjointActiveModel, KindModel, KindVisitor};

/// A base keyed by two columns, its kind column under a name of its own, and
/// a kind type named and placed by hand.
pub mod listing {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "listing", schema_name = "market")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub tenant_id: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i64,
        pub category: super::ListingKind,
        pub title: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A kind keyed by the listing's two columns under other names.
pub mod flat {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "flat", schema_name = "market")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub tenant: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub listing: i64,
        pub rooms: i16,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
#[pgorm(
    base = listing::Entity,
    kind = listing::Column::Category,
    enum_name = "listing_category",
    schema_name = "market"
)]
pub enum Listing {
    #[pgorm(value = "apartment")]
    Flat(flat::Model),
    BuildingPlot,
}

/// Each kind visited, as its value, its entity's table and its key columns
/// with their types.
struct Describe;

type Described = (String, Option<(String, Vec<(String, String)>)>);

impl<D: Disjoint> KindVisitor<D> for Describe {
    type Output = Described;

    fn table<M: KindModel<D>>(self, kind: D::Kind) -> Described {
        type Key<M> = <<M as ModelTrait>::Entity as EntityTrait>::PrimaryKey;
        let key = Key::<M>::iter()
            .map(|key| {
                let column = key.into_column();
                let column_type = format!("{:?}", column.def().get_column_type());
                (column.as_str().to_owned(), column_type)
            })
            .collect();
        let table = M::Entity::default().table_name().to_owned();
        (kind.to_value(), Some((table, key)))
    }

    fn unit(self, kind: D::Kind) -> Described {
        (kind.to_value(), None)
    }
}

fn describe<D: Disjoint>() -> Vec<Described> {
    D::Kind::iter()
        .map(|kind| D::visit_kind(kind, Describe))
        .collect()
}

fn key_column(column: &str, column_type: &str) -> (String, String) {
    (column.to_owned(), column_type.to_owned())
}

// [spec:pgorm:def:disjoint.declare+2/test]    the kind type is a PostgreSQL enum named for the enum in snake_case, one value per variant in declaration order, each its variant's name in snake_case
#[test]
fn kind_type_is_a_postgres_enum_of_variants() {
    assert_eq!(VehicleKind::name().to_string(), "vehicle_kind");
    assert_eq!(VehicleKind::values(), ["car", "truck", "bike"]);
    assert_eq!(
        VehicleKind::iter().collect::<Vec<_>>(),
        [VehicleKind::Car, VehicleKind::Truck, VehicleKind::Bike]
    );
    match VehicleKind::db_type().get_column_type() {
        ColumnType::Enum { name, schema, .. } => {
            assert_eq!(name.to_string(), "vehicle_kind");
            assert!(schema.is_none());
        }
        other => panic!("expected a PostgreSQL enum, got {other:?}"),
    }
    assert_eq!(
        VehicleKind::try_from_value(&"truck".to_owned()),
        Ok(VehicleKind::Truck)
    );
    assert!(VehicleKind::try_from_value(&"boat".to_owned()).is_err());
    assert_eq!(
        vehicle::Column::Kind.def().get_column_type(),
        VehicleKind::db_type().get_column_type()
    );
}

// [spec:pgorm:def:disjoint.declare+2/test]    `value`, `enum_name` and `schema_name` in place of the defaults, and `kind` naming the base's column
#[test]
fn a_disjoint_says_what_differs_from_the_defaults() {
    assert_eq!(ListingKind::name().to_string(), "listing_category");
    assert_eq!(ListingKind::values(), ["apartment", "building_plot"]);
    match ListingKind::db_type().get_column_type() {
        ColumnType::Enum { schema, .. } => assert_eq!(
            schema.as_ref().map(|schema| schema.to_string()).as_deref(),
            Some("market")
        ),
        other => panic!("expected a PostgreSQL enum, got {other:?}"),
    }
    assert_eq!(
        Listing::kind_column().as_str(),
        listing::Column::Category.as_str()
    );
    assert_eq!(
        Vehicle::kind_column().as_str(),
        vehicle::Column::Kind.as_str()
    );
}

// [spec:pgorm:def:disjoint.declare+2/test]    each kind with a table reaches its entity, its table and key columns, in declaration order, a unit kind none; a key may be composite and named otherwise than the base's
#[test]
fn kinds_reach_their_entities_in_order() {
    assert_eq!(
        describe::<Vehicle>(),
        [
            (
                "car".to_owned(),
                Some(("car".to_owned(), vec![key_column("id", "Integer")]))
            ),
            (
                "truck".to_owned(),
                Some((
                    "truck".to_owned(),
                    vec![key_column("vehicle_id", "Integer")]
                ))
            ),
            ("bike".to_owned(), None),
        ]
    );
    assert_eq!(
        describe::<Listing>(),
        [
            (
                "apartment".to_owned(),
                Some((
                    "flat".to_owned(),
                    vec![
                        key_column("tenant", "Integer"),
                        key_column("listing", "BigInteger")
                    ]
                ))
            ),
            ("building_plot".to_owned(), None),
        ]
    );
}

/// The kind's key is the base's key, as a type: a base key passes for a kind
/// key with no conversion.
fn kind_key<D: Disjoint, M: KindModel<D>>(
    key: <<D::Base as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType,
) -> <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType {
    key
}

// [spec:pgorm:def:disjoint.declare+2/test]    a kind's model decodes into its variant, each value names its kind, and a kind entity's key is the base's key type
#[test]
fn a_kind_model_is_its_variant() {
    let sedan = car::Model { id: 1, seats: 5 };
    assert_eq!(<car::Model as KindModel<Vehicle>>::KIND, VehicleKind::Car);
    assert_eq!(sedan.clone().into_variant(), Vehicle::Car(sedan.clone()));
    let lorry = truck::Model {
        vehicle_id: 2,
        axles: 3,
        payload_kg: 18_000,
    };
    assert_eq!(
        <truck::Model as KindModel<Vehicle>>::KIND,
        VehicleKind::Truck
    );

    assert_eq!(Vehicle::Car(sedan).kind(), VehicleKind::Car);
    assert_eq!(Vehicle::Truck(lorry).kind(), VehicleKind::Truck);
    assert_eq!(Vehicle::Bike.kind(), VehicleKind::Bike);

    let flat = flat::Model {
        tenant: 7,
        listing: 9,
        rooms: 3,
    };
    assert_eq!(flat.clone().into_variant(), Listing::Flat(flat));
    assert_eq!(Listing::BuildingPlot.kind(), ListingKind::BuildingPlot);

    assert_eq!(kind_key::<Vehicle, truck::Model>(4), 4);
    assert_eq!(kind_key::<Listing, flat::Model>((7, 9)), (7, 9));
}

/// An active model taken apart: its kind, and the columns its kind's row
/// sets.
struct SetColumns;

impl<D: Disjoint> ActiveKindVisitor<D> for SetColumns {
    type Output = (String, Vec<String>);

    fn table<M: KindModel<D>>(
        self,
        kind: D::Kind,
        active: <M::Entity as EntityTrait>::ActiveModel,
    ) -> Self::Output {
        let set = <M::Entity as EntityTrait>::Column::iter()
            .filter(|column| !active.is_not_set(*column))
            .map(|column| column.as_str().to_owned())
            .collect();
        (kind.to_value(), set)
    }

    fn unit(self, kind: D::Kind) -> Self::Output {
        (kind.to_value(), Vec::new())
    }
}

// [spec:pgorm:def:disjoint.declare+2/test]    `<Enum>ActiveModel` holds each kind's `ActiveModel`, names its kind, and is taken apart into the kind's `ActiveModel`
#[test]
fn an_active_model_is_taken_apart() {
    let truck = VehicleActiveModel::Truck(truck::ActiveModel {
        axles: set(2_i16),
        payload_kg: set(3_500),
        ..Default::default()
    });
    assert_eq!(truck.kind(), VehicleKind::Truck);
    assert_eq!(
        truck.visit(SetColumns),
        (
            "truck".to_owned(),
            vec!["axles".to_owned(), "payload_kg".to_owned()]
        )
    );
    assert_eq!(VehicleActiveModel::Bike.kind(), VehicleKind::Bike);
    assert_eq!(
        VehicleActiveModel::Bike.visit(SetColumns),
        ("bike".to_owned(), Vec::new())
    );
    let flat = ListingActiveModel::Flat(flat::ActiveModel {
        rooms: set(4_i16),
        ..Default::default()
    });
    assert_eq!(flat.kind(), ListingKind::Flat);
    assert_eq!(
        flat.visit(SetColumns),
        ("apartment".to_owned(), vec!["rooms".to_owned()])
    );
}

// [spec:pgorm:def:disjoint.declare+2/test]    a declaration whose types hold passes the run-time check, composite key and named kind column included
#[test]
fn a_sound_declaration_passes_the_check() {
    assert!(pgorm::check_disjoint::<Vehicle>().is_ok());
    assert!(pgorm::check_disjoint::<Listing>().is_ok());
}
