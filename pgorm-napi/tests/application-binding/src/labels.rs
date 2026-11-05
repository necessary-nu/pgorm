//! A one-of reference the application registers: a label on a packet or a
//! pallet, or on neither, its key held in one nullable column per target.

/// A label: its target's key in `packet_id` or `pallet_id`, at most one set.
pub mod label {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "labels", schema_name = "napi_shipments")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub text: String,
        pub packet_id: Option<i32>,
        pub pallet_id: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = crate::shipping::packet::Entity,
            from = Column::PacketId,
            to = crate::shipping::packet::Column::ShipmentId
        )]
        Packet,
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
        #[pgorm(column = Column::PacketId)]
        Packet(crate::shipping::packet::Entity),
        #[pgorm(column = Column::PalletId)]
        Pallet(crate::shipping::pallet::Entity),
        Loose,
    }
}
