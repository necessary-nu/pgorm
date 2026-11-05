//! A one-of reference the application registers: a label on a parcel or a
//! pallet, or on neither, its key held in one nullable column per target.

/// A label: its target's key in `parcel_id` or `pallet_id`, at most one set.
pub mod label {
    use pgorm::entity::prelude::*;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, Entity, Serialize, Deserialize)]
    #[pgorm(
        table_name = "labels",
        schema_name = "python_shipments",
        unique(
            name = "labels_text_parcel",
            nulls_not_distinct,
            columns = (Column::Text, Column::ParcelId)
        )
    )]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub text: String,
        pub parcel_id: Option<i32>,
        pub pallet_id: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = crate::shipping::parcel::Entity,
            from = Column::ParcelId,
            to = crate::shipping::parcel::Column::ShipmentId
        )]
        Parcel,
        #[pgorm(
            belongs_to = crate::shipping::pallet::Entity,
            from = Column::PalletId,
            to = crate::shipping::pallet::Column::Id
        )]
        Pallet,
    }

    // [spec:pgorm:req:bindings.one-of+1/test]
    #[derive(Clone, Copy, Debug, OneOf)]
    #[pgorm(entity = Entity)]
    pub enum Labelled {
        #[pgorm(column = Column::ParcelId)]
        Parcel(crate::shipping::parcel::Entity),
        #[pgorm(column = Column::PalletId)]
        Pallet(crate::shipping::pallet::Entity),
        Loose,
    }
}
