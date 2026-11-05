//! Parity: the schema of each registered entity, written here by pgorm's own
//! `Schema` with the application's entities directly, is `tests/schema-parity.json`
//! — the file the JavaScript suite holds `fromEntity` to in both runtimes —
//! and a table `Schema` refuses is its refusal there.

use std::collections::BTreeMap;

use pgorm::{EntityTrait, Schema};
use serde_json::{Value as Json, json};

use crate::{account, booking, labels, listings, membership, note, parcel, shipping, temper, till};

/// The entity's table, enum, index and comment statements as SQL, or the
/// message `Schema::create_table_from_entity` refuses its table with.
fn generated<E: EntityTrait>() -> Json {
    let schema = Schema::new();
    match schema.create_table_from_entity(E::default()) {
        Ok(table) => json!({
            "table": table.to_string(),
            "enums": schema.create_enum_from_entity(E::default()).iter().map(ToString::to_string).collect::<Vec<_>>(),
            "indexes": schema.create_index_from_entity(E::default()).iter().map(ToString::to_string).collect::<Vec<_>>(),
            "comments": schema.create_comments_from_entity(E::default()).iter().map(ToString::to_string).collect::<Vec<_>>(),
        }),
        Err(error) => json!({ "refused": error.to_string() }),
    }
}

// [spec:pgorm:req:bindings.schema-from-entity/test]
#[test]
fn entity_schemas_match_the_golden_file() {
    let golden: BTreeMap<String, Json> =
        serde_json::from_str(include_str!("../tests/schema-parity.json"))
            .unwrap_or_else(|error| panic!("the golden file is JSON: {error}"));
    let built = BTreeMap::from([
        ("app.Account".to_owned(), generated::<account::Entity>()),
        (
            "app.Membership".to_owned(),
            generated::<membership::Entity>(),
        ),
        ("app.Note".to_owned(), generated::<note::Entity>()),
        ("app.Parcel".to_owned(), generated::<parcel::Entity>()),
        ("app.Temper".to_owned(), generated::<temper::Entity>()),
        ("app.Till".to_owned(), generated::<till::Entity>()),
        ("app.Booking".to_owned(), generated::<booking::Entity>()),
        (
            "app.Listing".to_owned(),
            generated::<listings::listing::Entity>(),
        ),
        (
            "app.Auction".to_owned(),
            generated::<listings::auction::Entity>(),
        ),
        ("app.Sale".to_owned(), generated::<listings::sale::Entity>()),
        (
            "app.Shipment".to_owned(),
            generated::<shipping::shipment::Entity>(),
        ),
        (
            "app.Packet".to_owned(),
            generated::<shipping::packet::Entity>(),
        ),
        (
            "app.Pallet".to_owned(),
            generated::<shipping::pallet::Entity>(),
        ),
        ("app.Label".to_owned(), generated::<labels::label::Entity>()),
    ]);

    let account = &built["app.Account"];
    for part in ["enums", "indexes", "comments"] {
        assert!(
            account[part]
                .as_array()
                .is_some_and(|list| !list.is_empty()),
            "the account exercises {part}"
        );
    }
    assert!(
        built["app.Temper"]["refused"].is_string(),
        "the temper's table is refused"
    );
    assert_eq!(built, golden);
}
