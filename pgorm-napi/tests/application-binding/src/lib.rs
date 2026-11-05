//! An application crate, apart from the binding's own workspace, that
//! registers its entities with pgorm-napi and builds the one native module
//! JavaScript loads: the binding's whole API and these registrations
//! together, as pgorm-python's application module is built.

pub mod account;
pub mod graphs;
pub mod labels;
pub mod listings;
pub mod membership;
pub mod note;
pub mod parcel;
pub mod shipping;
pub mod sources;
pub mod temper;
pub mod till;

#[cfg(test)]
mod parity;
#[cfg(test)]
mod schema_parity;

use neon::prelude::*;
use pgorm_napi::{RegistrationError, Registry};

/// The application's registrations.
// [spec:pgorm:req:bindings.entities+4/test]
pub fn registry() -> Result<Registry, RegistrationError> {
    let mut registry = Registry::default();
    registry.entity::<account::Entity>("app.Account")?;
    registry.entity::<note::Entity>("app.Note")?;
    registry.entity::<membership::Entity>("app.Membership")?;
    registry.entity::<parcel::Entity>("app.Parcel")?;
    registry.entity::<temper::Entity>("app.Temper")?;
    registry.entity::<till::Entity>("app.Till")?;
    registry.entity::<shipping::shipment::Entity>("app.Shipment")?;
    registry.entity::<shipping::packet::Entity>("app.Packet")?;
    registry.entity::<shipping::pallet::Entity>("app.Pallet")?;
    registry.disjoint::<shipping::shipment::Shipment>("app.Shipments")?;
    registry.entity::<labels::label::Entity>("app.Label")?;
    registry.one_of::<labels::label::Labelled>("app.Labelled")?;
    registry.entity::<listings::listing::Entity>("app.Listing")?;
    registry.entity::<listings::auction::Entity>("app.Auction")?;
    registry.entity::<listings::sale::Entity>("app.Sale")?;
    registry.disjoint::<listings::listing::Listing>("app.Listings")?;
    graphs::register(&mut registry)?;
    sources::register(&mut registry)?;
    Ok(registry)
}

// [spec:pgorm:req:napi.application+2/test]
#[neon::main]
fn main(mut cx: ModuleContext) -> NeonResult<()> {
    let registry = match registry() {
        Ok(registry) => registry,
        Err(error) => return cx.throw_error(error.to_string()),
    };
    pgorm_napi::install(&mut cx, registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:pgorm:req:bindings.entities+4/test]
    #[test]
    fn registry_refuses_duplicate_names_and_entities() -> Result<(), RegistrationError> {
        let mut registry = Registry::default();
        registry.entity::<account::Entity>("app.Account")?;
        assert!(registry.entity::<note::Entity>("app.Account").is_err());
        assert!(registry.entity::<account::Entity>("again.Account").is_err());
        assert!(registry.entity::<note::Entity>("").is_err());
        let unregistered = registry.graph::<note::Entity, (), _>("app.NoteOnly", |_| {
            <note::Entity as pgorm::EntityTrait>::graph()
        });
        assert!(unregistered.is_err());
        registry.entity::<note::Entity>("app.Note")?;
        // [spec:pgorm:req:bindings.disjoint+1/test]    a disjoint enum registers once its base and kind entities have, and a name or an enum registered twice is refused
        assert!(
            registry
                .disjoint::<shipping::shipment::Shipment>("app.Shipments")
                .is_err()
        );
        registry.entity::<shipping::shipment::Entity>("app.Shipment")?;
        registry.entity::<shipping::packet::Entity>("app.Packet")?;
        assert!(
            registry
                .disjoint::<shipping::shipment::Shipment>("app.Shipments")
                .is_err()
        );
        registry.entity::<shipping::pallet::Entity>("app.Pallet")?;
        registry.disjoint::<shipping::shipment::Shipment>("app.Shipments")?;
        assert!(
            registry
                .disjoint::<shipping::shipment::Shipment>("app.Others")
                .is_err()
        );
        // [spec:pgorm:req:bindings.one-of+1/test]    a one-of enum registers once its entity and its targets' entities have, and a name or an enum registered twice is refused
        assert!(
            registry
                .one_of::<labels::label::Labelled>("app.Labelled")
                .is_err()
        );
        registry.entity::<labels::label::Entity>("app.Label")?;
        registry.one_of::<labels::label::Labelled>("app.Labelled")?;
        assert!(
            registry
                .one_of::<labels::label::Labelled>("app.Others")
                .is_err()
        );
        graphs::register(&mut Registry::default()).expect_err("graphs need their entities");
        assert!(
            registry
                .graph::<note::Entity, (), _>("app.NoteOnly", |_| {
                    <note::Entity as pgorm::EntityTrait>::graph()
                })
                .is_ok()
        );
        assert!(
            registry
                .graph::<note::Entity, (), _>("app.NoteOnly", |_| {
                    <note::Entity as pgorm::EntityTrait>::graph()
                })
                .is_err()
        );
        Ok(())
    }
}
