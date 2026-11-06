//! The application's disjoint enums, which its generated Python module
//! carries: a delivery is a parcel or a pallet, each with a table of its own
//! keyed by the delivery's, or a letter, which has none; and an offer, read
//! from tables an existing schema holds with no kind column, is an auction or
//! a sale, or a draft, with a row in neither.

/// The base of the kinded `Delivery`: one row per delivery, its kind in
/// `kind`.
pub mod delivery {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "deliveries", schema_name = "python_courier")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub kind: DeliveryKind,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}

    #[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
    #[pgorm(base = Entity, schema_name = "python_courier")]
    pub enum Delivery {
        Parcel(super::parcel::Model),
        Letter,
        Pallet(super::pallet::Model),
    }
}

/// The kind `parcel`, keyed by its delivery's `id` under a name of its own.
pub mod parcel {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "parcels", schema_name = "python_courier")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub delivery_id: i32,
        pub weight_g: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The kind `pallet`, keyed by a column named as the delivery's key is.
pub mod pallet {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "pallets", schema_name = "python_courier")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub volume_l: i16,
        pub fragile: Option<bool>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The base of the unkinded `Offer`: one row per offer, with no kind column.
pub mod offer {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "offers", schema_name = "python_courier")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub title: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}

    #[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
    #[pgorm(base = Entity, unkinded)]
    pub enum Offer {
        Auction(super::auction::Model),
        Sale(super::sale::Model),
        Draft,
    }
}

/// The kind `auction`, keyed by its offer's `id` under a name of its own.
pub mod auction {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "auctions", schema_name = "python_courier")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub offer_id: i32,
        pub reserve: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The kind `sale`, keyed by a column named as the offer's key is.
pub mod sale {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "sales", schema_name = "python_courier")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub price: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// A shipping label, on a parcel or on a pallet and never on both: a one-of
/// reference the application registers beside its entities.
pub mod label {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "labels", schema_name = "python_courier")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub parcel_id: Option<i32>,
        pub pallet_id: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::parcel::Entity,
            from = Column::ParcelId,
            to = super::parcel::Column::DeliveryId
        )]
        Parcel,
        #[pgorm(
            belongs_to = super::pallet::Entity,
            from = Column::PalletId,
            to = super::pallet::Column::Id
        )]
        Pallet,
    }

    /// What a label is on.
    #[derive(Clone, Copy, Debug, pgorm::OneOf)]
    #[pgorm(entity = Entity)]
    pub enum Target {
        #[pgorm(column = Column::ParcelId)]
        Parcel(super::parcel::Entity),
        #[pgorm(column = Column::PalletId)]
        Pallet(super::pallet::Entity),
    }
}
