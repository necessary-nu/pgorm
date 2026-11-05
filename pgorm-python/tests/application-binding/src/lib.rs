//! Independent application crate: no pgorm repository fixture entities.

pub mod account;
pub mod booking;
pub mod graphs;
pub mod labels;
pub mod listings;
pub mod membership;
pub mod note;
pub mod old;
pub mod room;
pub mod shipping;
pub mod sources;
pub mod stay;
pub mod temper;
pub mod tenant_note;
pub mod till;

#[cfg(test)]
mod graph_parity;
#[cfg(test)]
mod parity;
#[cfg(test)]
mod schema_parity;

use pyo3::prelude::*;

// [spec:pgorm:req:bindings.entities+6/test]
#[pymodule(gil_used = true)]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let mut registry = pgorm_python::entities::Registry::default();
    registry.entity::<account::Entity>("app.Account")?;
    registry.entity::<note::Entity>("app.Note")?;
    registry.entity::<membership::Entity>("app.Membership")?;
    registry.entity::<booking::Entity>("app.Booking")?;
    registry.entity::<room::Entity>("app.Room")?;
    registry.entity::<stay::Entity>("app.Stay")?;
    registry.entity::<old::Entity>("app.Old")?;
    registry.entity::<temper::Entity>("app.Temper")?;
    registry.entity::<till::Entity>("app.Till")?;
    registry.entity::<tenant_note::Entity>("app.TenantNote")?;
    registry.entity_in(
        "app.AcmeTenantNote",
        tenant_note::Entity {
            tenant: "python_acme",
        },
    )?;
    registry.entity::<shipping::shipment::Entity>("app.Shipment")?;
    registry.entity::<shipping::parcel::Entity>("app.Parcel")?;
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
    pgorm_python::install(module, registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:pgorm:req:bindings.entities+6/test]
    #[test]
    fn registry_refuses_name_and_type_collisions() -> PyResult<()> {
        Python::initialize();
        let mut registry = pgorm_python::entities::Registry::default();
        registry.entity::<account::Entity>("app.Account")?;
        assert!(registry.entity::<note::Entity>("app.Account").is_err());
        assert!(
            registry
                .entity::<account::Entity>("duplicate.Account")
                .is_err()
        );
        registry.entity::<note::Entity>("app.Note")?;
        // [spec:pgorm:req:bindings.entities+6/test]    a type registers once per table its
        // instances name, so one table twice is still a collision
        registry.entity::<tenant_note::Entity>("app.TenantNote")?;
        let acme = tenant_note::Entity {
            tenant: "python_acme",
        };
        registry.entity_in("app.AcmeTenantNote", acme)?;
        assert!(registry.entity_in("app.AcmeAgain", acme).is_err());
        assert!(
            registry
                .entity::<tenant_note::Entity>("app.DefaultAgain")
                .is_err()
        );
        // [spec:pgorm:req:bindings.disjoint+1/test]    a disjoint enum registers once its base and kind entities have, and a name or an enum registered twice is refused
        assert!(
            registry
                .disjoint::<shipping::shipment::Shipment>("app.Shipments")
                .is_err()
        );
        registry.entity::<shipping::shipment::Entity>("app.Shipment")?;
        registry.entity::<shipping::parcel::Entity>("app.Parcel")?;
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
        Python::attach(|py| {
            let module = PyModule::new(py, "pgorm._native")?;
            pgorm_python::install(&module, registry)?;
            assert!(pgorm_python::install(&module, Default::default()).is_err());
            let manifest = module.call_method0("capabilities")?;
            assert_eq!(
                manifest
                    .get_item("registrations")?
                    .get_item("entities")?
                    .len()?,
                8
            );
            assert_eq!(
                manifest
                    .get_item("registrations")?
                    .get_item("disjoints")?
                    .len()?,
                1
            );
            assert_eq!(
                manifest
                    .get_item("registrations")?
                    .get_item("one_ofs")?
                    .len()?,
                1
            );
            Ok(())
        })
    }
}
