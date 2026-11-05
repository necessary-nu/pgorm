use crate::{account, graphs, note};
use pgorm::pgorm_query::{Expr, ExprTrait, Name, Order};
use pgorm::{QueryFilter, QueryOrder, QueryTrait};
use pgorm_python::expressions::Compiled;
use pyo3::{prelude::*, types::PyDict};

// [spec:pgorm:req:bindings.entity-graphs+3/test]
#[test]
fn graph_sql_and_parameters_match_rust() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "pgorm._native")?;
        crate::_native(&module)?;
        for alias in ["notes", "notes \"β\""] {
            for name in ["app.AccountNotes", "app.RequiredNotes"] {
                let graph = module.call_method1("graph", (name,))?;
                let kwargs = PyDict::new(py);
                kwargs.set_item("aliases", vec![alias])?;
                let query = graph.call_method("find", (), Some(&kwargs))?;
                let id = query.call_method1("col", (0, "id"))?;
                let child = query.call_method1("col", (1, "id"))?;
                let query = query
                    .call_method1("where", (id.call_method1("gte", (2,))?,))?
                    .call_method1("order_by", (child.call_method0("desc")?,))?;
                for terminal in ["all", "optional"] {
                    let kwargs = PyDict::new(py);
                    kwargs.set_item("terminal", terminal)?;
                    let native: Compiled =
                        query.call_method("inspect", (), Some(&kwargs))?.extract()?;
                    let filter =
                        Expr::col((Name::runtime("accounts"), account::Column::Id)).gte(2i64);
                    let ordering = Expr::col((Name::runtime(alias), note::Column::Id));
                    let mut rust = if name == "app.AccountNotes" {
                        graphs::optional(&[alias.to_owned()])
                            .filter(filter)
                            .order_by(ordering, Order::Desc)
                            .into_query()
                    } else {
                        graphs::required(&[alias.to_owned()])
                            .filter(filter)
                            .order_by(ordering, Order::Desc)
                            .into_query()
                    };
                    if terminal == "optional" {
                        rust.narrow_limit_to_one();
                    }
                    assert_eq!(
                        (native.sql, native.values),
                        rust.build(),
                        "{name}/{alias}/{terminal}"
                    );
                }
            }
        }
        Ok(())
    })
}

// [spec:pgorm:req:bindings.entity-graphs+3/test]
#[test]
fn graphs_require_sources_and_unique_names() -> PyResult<()> {
    Python::initialize();
    let mut registry = pgorm_python::entities::Registry::default();
    assert!(registry.graph("missing", graphs::optional).is_err());
    registry.entity::<account::Entity>("app.Account")?;
    assert!(registry.graph("missing", graphs::optional).is_err());
    registry.entity::<note::Entity>("app.Note")?;
    registry.graph("notes", graphs::optional)?;
    assert!(registry.graph("notes", graphs::required).is_err());
    assert!(registry.graph("", graphs::optional).is_err());
    Ok(())
}

// [spec:pgorm:req:bindings.entity-graphs+3/test]
// [spec:pgorm:req:bindings.entities+5/test]
#[test]
fn temporal_keys_and_period_relations_match_rust() -> PyResult<()> {
    use crate::{room, stay};
    use pgorm::entity::prelude::{ColumnTrait, Date, EntityTrait, Range};

    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "pgorm._native")?;
        crate::_native(&module)?;
        let datetime = py.import("datetime")?;
        let day = |month: u8, day: u8| datetime.getattr("date")?.call1((2026, month, day));
        let range = module.getattr("Range")?.call1((day(1, 20)?, day(2, 10)?))?;
        let during = Range::from(Date::constant(2026, 1, 20)..Date::constant(2026, 2, 10));

        let graph = module.call_method1("graph", ("app.StayRooms",))?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("aliases", vec!["Room version"])?;
        let query = graph.call_method("find", (), Some(&kwargs))?;
        let period = query.call_method1("col", (0, "during"))?;
        let rate = query.call_method1("col", (1, "rate"))?;
        let query = query
            .call_method1(
                "where",
                (period.call_method1(
                    "eq",
                    (module.getattr("Value")?.call1((&range, "daterange"))?,),
                )?,),
            )?
            .call_method1("order_by", (rate.call_method0("asc")?,))?;
        let native: Compiled = query.call_method0("inspect")?.extract()?;
        let rust = graphs::stay_rooms(&["Room version".to_owned()])
            .filter(Expr::col((Name::runtime("stays"), stay::Column::During)).eq(during.clone()))
            .order_by(
                Expr::col((Name::runtime("Room version"), room::Column::Rate)),
                Order::Asc,
            )
            .into_query()
            .build();
        assert!(native.sql.contains(" && "), "{}", native.sql);
        assert_eq!((native.sql, native.values), rust);

        let entity = module.call_method1("entity", ("app.Room",))?;
        let id = entity.call_method1("col", ("id",))?;
        let valid_at = entity.call_method1("col", ("valid_at",))?;
        let key = id
            .call_method1("eq", (1,))?
            .call_method1("__and__", (valid_at.call_method1("eq", (&range,))?,))?;
        let native: Compiled = entity
            .call_method0("find")?
            .call_method1("where", (key,))?
            .call_method0("inspect")?
            .extract()?;
        let expected = room::Entity::find()
            .filter(room::Column::Id.eq(1).and(room::Column::ValidAt.eq(during)))
            .build();
        assert_eq!((native.sql, native.values), expected);

        let json = py.import("json")?;
        let describe = |name: &str| -> PyResult<serde_json::Value> {
            let info = module
                .call_method1("entity", (name,))?
                .call_method0("describe")?;
            let text: String = json.call_method1("dumps", (info,))?.extract()?;
            serde_json::from_str(&text)
                .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))
        };
        let room = describe("app.Room")?;
        assert_eq!(
            room["primary_key_without_overlaps"],
            serde_json::json!(true)
        );
        assert_eq!(room["primary_key"], serde_json::json!(["id", "valid_at"]));
        let stay = describe("app.Stay")?;
        assert_eq!(
            stay["relations"],
            serde_json::json!([
                {
                    "name": "Room", "type": "has_one",
                    "from": {"schema": "python_entities", "table": "stays"},
                    "to": {"schema": "python_entities", "table": "rooms"},
                    "columns": [["room_id", "id"]], "period": ["during", "valid_at"],
                    "condition_type": "all", "on_condition": false,
                    "enforcement": null, "deferrability": "deferrable_initially_deferred",
                },
                {
                    "name": "Guest", "type": "has_one",
                    "from": {"schema": "python_entities", "table": "stays"},
                    "to": {"schema": "python_entities", "table": "accounts"},
                    "columns": [["guest_id", "id"]], "period": null,
                    "condition_type": "all", "on_condition": false,
                    "enforcement": "not_enforced", "deferrability": null,
                },
            ])
        );
        assert_eq!(
            describe("app.Account")?["primary_key_without_overlaps"],
            serde_json::json!(false)
        );
        Ok(())
    })
}
