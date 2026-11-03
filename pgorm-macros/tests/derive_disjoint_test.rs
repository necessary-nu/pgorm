//! What `#[derive(Disjoint)]` declares: the kind type and its values, the
//! kinds' entities in declaration order with their keys, the decoding of a
//! kind's model into its variant, an `<Enum>ActiveModel` taken apart, and an
//! `unkinded` declaration's kinds, which no column holds.

use pgorm::entity::prelude::*;
use pgorm::tests_cfg::{
    car, flat,
    listing::{self, Listing, ListingActiveModel, ListingKind},
    shop::{self, Shop, ShopKind},
    storefront, truck,
    vehicle::{self, Vehicle, VehicleActiveModel, VehicleKind},
    web_shop,
};
use pgorm::{ActiveKindVisitor, DisjointActiveModel, KindModel, KindVisitor};

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
        (D::kind_name(kind).to_owned(), Some((table, key)))
    }

    fn unit(self, value: D) -> Described {
        (D::kind_name(value.kind()).to_owned(), None)
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

// [spec:pgorm:def:disjoint.declare+5/test]    the kind type is a PostgreSQL enum named for the enum in snake_case, one value per variant in declaration order, each its variant's name in snake_case
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

// [spec:pgorm:def:disjoint.declare+5/test]    `value`, `enum_name` and `schema_name` in place of the defaults, and `kind` naming the base's column
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
    let listing = Listing::kind_column().expect("a kinded entity has a kind column");
    assert_eq!(listing.column.as_str(), listing::Column::Category.as_str());
    assert_eq!(
        listing.kind_type.get_column_type(),
        ListingKind::db_type().get_column_type()
    );
    let vehicle = Vehicle::kind_column().expect("a kinded entity has a kind column");
    assert_eq!(vehicle.column.as_str(), vehicle::Column::Kind.as_str());
}

// [spec:pgorm:def:disjoint.declare+5/test]    each kind with a table reaches its entity, its table and key columns, in declaration order, a unit kind none; a key may be composite and named otherwise than the base's
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

// [spec:pgorm:def:disjoint.declare+5/test]    a kind's model decodes into its variant, each value names its kind, and a kind entity's key is the base's key type
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
        (D::kind_name(kind).to_owned(), set)
    }

    fn unit(self, kind: D::Kind) -> Self::Output {
        (D::kind_name(kind).to_owned(), Vec::new())
    }
}

// [spec:pgorm:def:disjoint.declare+5/test]    `<Enum>ActiveModel` holds each kind's `ActiveModel`, names its kind, and is taken apart into the kind's `ActiveModel`
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

// [spec:pgorm:def:disjoint.declare+5/test]    a declaration whose types hold passes the run-time check, composite key and named kind column included
#[test]
fn a_sound_declaration_passes_the_check() {
    assert!(pgorm::check_disjoint::<Vehicle>().is_ok());
    assert!(pgorm::check_disjoint::<Listing>().is_ok());
}

/// The vehicle's kinds again, their kind type carrying derives and a serde
/// attribute of its own.
#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(
    base = vehicle::Entity,
    enum_name = "vehicle_kind",
    kind_attrs(
        derive(PartialOrd, Ord, serde::Serialize, serde::Deserialize),
        serde(rename_all = "lowercase")
    )
)]
pub enum Ranked {
    Car,
    Truck,
    Bike,
}

// [spec:pgorm:def:disjoint.declare+5/test]    `kind_attrs` gives the kind type each attribute as written on it, derives and their helper attributes alike
#[test]
fn kind_attrs_reach_the_kind_type() {
    assert!(RankedKind::Car < RankedKind::Truck);
    assert!(RankedKind::Truck < RankedKind::Bike);
    assert_eq!(
        serde_json::to_string(&RankedKind::Truck).expect("serializes"),
        r#""truck""#
    );
    assert_eq!(
        serde_json::from_str::<RankedKind>(r#""bike""#).expect("deserializes"),
        RankedKind::Bike
    );
    assert_eq!(RankedKind::values(), VehicleKind::values());
}

// [spec:pgorm:sem:disjoint.unkinded/test]    an `unkinded` declaration has no kind column and no kind type: `<Enum>Kind` iterates its kinds in declaration order, each named as its variant in snake_case, a kind with a table reaches its entity and key, its unit kind none, and the check refuses nothing
#[test]
fn an_unkinded_declaration_has_no_kind_column() {
    assert!(Shop::kind_column().is_none());
    assert_eq!(
        ShopKind::iter().collect::<Vec<_>>(),
        [ShopKind::WebShop, ShopKind::Storefront, ShopKind::Dormant]
    );
    assert_eq!(
        ShopKind::iter().map(Shop::kind_name).collect::<Vec<_>>(),
        ["web_shop", "storefront", "dormant"]
    );
    assert_eq!(
        describe::<Shop>(),
        [
            (
                "web_shop".to_owned(),
                Some((
                    "web_shop".to_owned(),
                    vec![key_column("shop_id", "Integer")]
                ))
            ),
            (
                "storefront".to_owned(),
                Some(("storefront".to_owned(), vec![key_column("id", "Integer")]))
            ),
            ("dormant".to_owned(), None),
        ]
    );
    let site = web_shop::Model {
        shop_id: 3,
        url: "https://example.com".to_owned(),
    };
    assert_eq!(
        <web_shop::Model as KindModel<Shop>>::KIND,
        ShopKind::WebShop
    );
    assert_eq!(
        <web_shop::Model as KindModel<Shop>>::into_variant(site.clone()),
        Shop::WebShop(site)
    );
    let street = storefront::Model {
        id: 4,
        street: "High Street".to_owned(),
    };
    assert_eq!(Shop::Storefront(street).kind(), ShopKind::Storefront);
    assert_eq!(Shop::Dormant.kind(), ShopKind::Dormant);
    assert_eq!(kind_key::<Shop, web_shop::Model>(5), 5);
    assert!(pgorm::check_disjoint::<Shop>().is_ok());
    assert!(shop::Column::iter().all(|column| column.as_str() != "kind"));
}

/// The shop's kinds again, the kind enum carrying derives of its own.
#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = shop::Entity, unkinded, kind_attrs(derive(PartialOrd, Ord)))]
pub enum RankedShop {
    WebShop(web_shop::Model),
    Dormant,
}

// [spec:pgorm:sem:disjoint.unkinded/test]    `kind_attrs` reaches an unkinded entity's kind enum as it does a kind type
#[test]
fn kind_attrs_reach_the_unkinded_kinds() {
    assert!(RankedShopKind::WebShop < RankedShopKind::Dormant);
    assert_eq!(RankedShop::kind_name(RankedShopKind::Dormant), "dormant");
}

/// The vehicle's kinds as configured: a variant `#[cfg]` removes is no kind,
/// and a `cfg_attr` gives a kind's `value`.
#[derive(Clone, Debug, PartialEq, Disjoint)]
#[pgorm(base = vehicle::Entity, enum_name = "vehicle_kind")]
pub enum Configured {
    #[cfg(not(test))]
    Car(car::Model),
    Truck(truck::Model),
    #[cfg_attr(test, pgorm(value = "bike"))]
    Cycle,
}

// [spec:pgorm:def:disjoint.declare+5/test]    a variant `#[cfg]` removes is no kind, of the kind type, the visitor or `<Enum>ActiveModel`, and a `cfg_attr` key is read as written
#[test]
fn configured_variants_are_the_kinds() {
    assert_eq!(
        ConfiguredKind::iter().collect::<Vec<_>>(),
        [ConfiguredKind::Truck, ConfiguredKind::Cycle]
    );
    assert_eq!(ConfiguredKind::values(), ["truck", "bike"]);
    assert_eq!(
        describe::<Configured>(),
        [
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
    assert_eq!(ConfiguredActiveModel::Cycle.kind(), ConfiguredKind::Cycle);
}

/// Kinds every variant of which ends with the enum's name, in a module the
/// crate does not export, where clippy's `enum_variant_names` lints them: the
/// program allows it on the enum it writes, and the enums the derives
/// generate from it allow it themselves.
mod named_as_their_enum {
    use pgorm::entity::prelude::*;
    use pgorm::tests_cfg::{shop, storefront, truck, vehicle, web_shop};

    #[allow(clippy::enum_variant_names)]
    #[derive(Clone, Debug, PartialEq, Disjoint)]
    #[pgorm(base = vehicle::Entity, enum_name = "vehicle_kind")]
    pub enum Vehicle {
        TruckVehicle(truck::Model),
        BikeVehicle,
        BoatVehicle,
    }

    #[allow(clippy::enum_variant_names)]
    #[derive(Clone, Debug, PartialEq, Disjoint)]
    #[pgorm(base = shop::Entity, unkinded)]
    pub enum Shop {
        WebShop(web_shop::Model),
        StorefrontShop(storefront::Model),
        ClosedShop,
    }
}

// [spec:pgorm:def:disjoint.declare+5/test]    the kind type, `<Enum>ActiveModel` and an unkinded entity's kind enum allow clippy's `enum_variant_names`, which their variants, repeating the enum's, would otherwise trip though the enum allows it
// [spec:pgorm:sem:macros.derive.active-enum.expansion+6/test]    and so does the kind type's `{Enum}Variant`, its variants the values escaped sharing a postfix
#[test]
fn variants_named_as_their_enum_compile_clean() {
    use named_as_their_enum::{Shop, ShopKind, Vehicle, VehicleActiveModel, VehicleKind};
    assert_eq!(
        VehicleKind::values(),
        ["truck_vehicle", "bike_vehicle", "boat_vehicle"]
    );
    assert_eq!(Vehicle::BikeVehicle.kind(), VehicleKind::BikeVehicle);
    assert_eq!(
        VehicleActiveModel::BikeVehicle.kind(),
        VehicleKind::BikeVehicle
    );
    assert_eq!(
        ShopKind::iter().map(Shop::kind_name).collect::<Vec<_>>(),
        ["web_shop", "storefront_shop", "closed_shop"]
    );
}
