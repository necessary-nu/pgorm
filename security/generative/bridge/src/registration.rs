use crate::{account, graphs, note, room, stay};
use pgorm::Entity;
use pgorm_python::entities::Registry;
use pyo3::prelude::*;

// [spec:pgorm:req:generative.execution]
pub fn register(registry: &mut Registry) -> PyResult<()> {
    registry.entity::<account::Entity>("campaign.Account")?;
    registry.entity::<note::Entity>("campaign.Note")?;
    registry.entity::<room::Entity>("campaign.Room")?;
    registry.entity::<stay::Entity>("campaign.Stay")?;
    registry
        .graph::<account::Entity, (), _>("campaign.AccountOnly", |_| account::Entity::graph())?;
    registry.graph("campaign.OptionalNotes", graphs::optional)?;
    registry.graph("campaign.RequiredNotes", graphs::required)?;
    registry.graph("campaign.SelfJoin", graphs::self_join)?;
    registry.graph("campaign.Arity3", graphs::arity3)?;
    registry.graph("campaign.Arity4", graphs::arity4)?;
    registry.graph("campaign.Arity5", graphs::arity5)?;
    registry.graph("campaign.Arity6", graphs::arity6)?;
    registry.graph("campaign.Arity7", graphs::arity7)?;
    registry.graph("campaign.StayRooms", graphs::stay_rooms)?;
    registry.graph("campaign.StayGuests", graphs::stay_guests)?;
    registry.sources::<(account::Entity,)>("campaign.Sources1")?;
    registry.sources::<(account::Entity, note::Entity)>("campaign.Sources2")?;
    registry.sources::<(account::Entity, note::Entity, note::Entity)>("campaign.Sources3")?;
    registry.sources::<(account::Entity, note::Entity, note::Entity, note::Entity)>(
        "campaign.Sources4",
    )?;
    registry.sources::<(
        account::Entity,
        note::Entity,
        note::Entity,
        note::Entity,
        note::Entity,
    )>("campaign.Sources5")?;
    registry.sources::<(
        account::Entity,
        note::Entity,
        note::Entity,
        note::Entity,
        note::Entity,
        note::Entity,
    )>("campaign.Sources6")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::types::PyDict;

    // [spec:pgorm:req:generative.execution/test]
    #[test]
    fn graph_shapes_keep_optional_slots() -> PyResult<()> {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "pgorm._native")?;
            crate::_native(&module)?;
            for arity in 3..=7 {
                let graph = module.call_method1("graph", (format!("campaign.Arity{arity}"),))?;
                let info = graph.call_method0("describe")?;
                let sources = info.get_item("sources")?;
                assert_eq!(sources.len()?, arity);
                assert_eq!(
                    sources.get_item(0)?.get_item("slot")?.extract::<String>()?,
                    "root"
                );
                for index in 1..arity {
                    assert_eq!(
                        sources
                            .get_item(index)?
                            .get_item("slot")?
                            .extract::<String>()?,
                        "Opt"
                    );
                }
            }
            Ok(())
        })
    }

    // [spec:pgorm:req:generative.execution/test]
    #[test]
    fn graph_factory_checks_and_quotes_aliases() -> PyResult<()> {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "pgorm._native")?;
            crate::_native(&module)?;
            let graph = module.call_method1("graph", ("campaign.OptionalNotes",))?;
            let options = PyDict::new(py);
            options.set_item("aliases", ["n\" 雪"])?;
            let query = graph.call_method("find", (), Some(&options))?;
            let sql = query
                .call_method0("inspect")?
                .getattr("sql")?
                .extract::<String>()?;
            assert!(sql.contains("\"n\"\" 雪\""));
            assert!(sql.contains("LEFT JOIN"));
            options.set_item("aliases", Vec::<String>::new())?;
            assert!(graph.call_method("find", (), Some(&options)).is_err());
            options.set_item("aliases", ["one", "two"])?;
            assert!(graph.call_method("find", (), Some(&options)).is_err());
            Ok(())
        })
    }

    // [spec:pgorm:req:generative.execution/test]
    #[test]
    fn source_tuples_require_the_registered_models() -> PyResult<()> {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "pgorm._native")?;
            crate::_native(&module)?;
            for arity in 1..=6 {
                let sources = module
                    .call_method1("pipeline_sources", (format!("campaign.Sources{arity}"),))?;
                let names = sources
                    .call_method0("describe")?
                    .get_item("entities")?
                    .extract::<Vec<String>>()?;
                assert_eq!(names.len(), arity);
                assert_eq!(names[0], "campaign.Account");
                assert!(names[1..].iter().all(|name| name == "campaign.Note"));
            }
            assert!(
                module
                    .call_method1("pipeline_sources", ("campaign.Sources7",))
                    .is_err()
            );
            assert!(
                module
                    .call_method1("entity", ("campaign.Missing",))
                    .is_err()
            );
            Ok(())
        })
    }
}
