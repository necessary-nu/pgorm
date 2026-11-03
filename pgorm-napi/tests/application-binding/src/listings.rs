//! An unkinded disjoint enum the application registers, read from tables an
//! existing schema holds with no kind column: a listing is an auction or a
//! sale, each with a table of its own keyed by the listing's, or a draft,
//! which has a row in neither. Nothing there holds a listing to one table, so
//! JavaScript reads it and never writes through it.

/// The base: one row per listing, with no kind column.
pub mod listing {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "listings", schema_name = "napi_listings")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub title: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}

    // [spec:pgorm:req:bindings.disjoint-unkinded/test]
    #[derive(Clone, Debug, PartialEq, Eq, Disjoint)]
    #[pgorm(base = Entity, unkinded)]
    pub enum Listing {
        Auction(super::auction::Model),
        Sale(super::sale::Model),
        Draft,
    }
}

/// The kind `auction`, keyed by its listing's `id` under a name of its own.
pub mod auction {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "auctions", schema_name = "napi_listings")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub listing_id: i32,
        pub reserve: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// The kind `sale`, keyed by a column named as the listing's key is.
pub mod sale {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "sales", schema_name = "napi_listings")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub price: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}
