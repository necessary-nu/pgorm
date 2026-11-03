//! A disjoint enum the application registers: a shipment is a packet or a
//! pallet, each with a table of its own keyed by the shipment's, or a letter,
//! which has none.

/// The base: one row per shipment, its kind in `kind`.
pub mod shipment {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "shipments", schema_name = "napi_shipments")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: ShipmentKind,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}

    // [spec:pgorm:req:bindings.disjoint+1/test]
    #[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
    #[pgorm(base = Entity, schema_name = "napi_shipments")]
    pub enum Shipment {
        Packet(super::packet::Model),
        Letter,
        Pallet(super::pallet::Model),
    }
}

/// The kind `packet`, keyed by its shipment's `id` under a name of its own.
pub mod packet {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "packets", schema_name = "napi_shipments")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub shipment_id: i32,
        pub weight_g: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The kind `pallet`, keyed by a column named as the base's key is.
pub mod pallet {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "pallets", schema_name = "napi_shipments")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub volume_l: i16,
        pub fragile: Option<bool>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}
