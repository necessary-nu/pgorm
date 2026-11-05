use pgorm::pgorm_query::ArrayType;
use pyo3::prelude::*;
use serde_json::{Value, json};

use crate::UnsupportedCapabilityError;

// [spec:pgorm:req:python.capabilities+2]
fn manifest() -> Value {
    let scalar_kinds = [
        "bool",
        "i8",
        "i16",
        "i32",
        "i64",
        "u32",
        "f32",
        "f64",
        "text",
        "bytes",
        "decimal",
        "uuid",
        "json",
        "date",
        "time",
        "datetime",
        "datetime_utc",
        "ipnetwork",
        "mac_address",
        "vector",
        "interval",
    ]
    .into_iter()
    .chain(crate::values::range_kind_names())
    .collect::<Vec<_>>();
    let column_types = [
        "char",
        "varchar",
        "text",
        "smallint",
        "integer",
        "bigint",
        "real",
        "double",
        "numeric",
        "boolean",
        "date",
        "time",
        "timestamp",
        "timestamptz",
        "interval",
        "bytea",
        "bit",
        "varbit",
        "money",
        "json",
        "jsonb",
        "uuid",
        "vector",
        "cidr",
        "inet",
        "macaddr",
        "ltree",
    ]
    .into_iter()
    .chain(crate::values::range_kind_names())
    .collect::<Vec<_>>();
    let mut manifest = json!({
        "schema_version": 1,
        "package_version": env!("CARGO_PKG_VERSION"),
        "pgorm_version": env!("PGORM_VERSION"),
        "binding": {"name": "pyo3", "version": env!("PGORM_PYO3_VERSION"), "registry_abi": 1},
        "target": env!("PGORM_BINDING_TARGET"),
        "features": ["macros", "with-json", "with-jiff", "with-uuid", "postgres-array", "metrics-fingerprint"],
        "transport": "in-process",
        "operations": {
            "pool": {"rust_api": "pgorm::connect_with", "features": []},
            "pool.acquire": {"rust_api": "pgorm::DatabasePool::get", "features": []},
            "pool.close": {"rust_api": "pgorm::DatabasePool::close", "features": []},
            "pool.clear_statement_caches": {"rust_api": "pgorm::DatabasePool::clear_statement_caches", "features": []},
            "pool.instrumented": {"rust_api": "pgorm::metric::InstrumentedPool::new", "features": []},
            "metrics.fingerprint": {"rust_api": "pgorm::metric::QueryContext::fingerprint", "features": ["metrics-fingerprint"]},
            "connection.ping": {"rust_api": "pgorm::ConnectionTrait::query_one", "features": []},
            "connection.close": {"rust_api": "pgorm::DatabaseConnection::drop", "features": []},
            "pool.listener": {"rust_api": "pgorm::DatabasePool::listener", "features": []},
            "listener.listen": {"rust_api": "pgorm::Listener::listen", "features": []},
            "listener.unlisten": {"rust_api": "pgorm::Listener::unlisten", "features": []},
            "listener.unlisten_all": {"rust_api": "pgorm::Listener::unlisten_all", "features": []},
            "listener.next": {"rust_api": "pgorm::Notifications as Stream", "features": []},
            "listener.close": {"rust_api": "pgorm::Notifications::drop", "features": []},
            "connection.notify": {"rust_api": "pgorm::notify for DatabaseConnection", "features": []},
            "transaction.notify": {"rust_api": "pgorm::notify for DatabaseTransaction", "features": []},
            "value": {"rust_api": "pgorm::pgorm_query::Value", "features": []},
            "value.array": {"rust_api": "pgorm::pgorm_query::Value::Array", "features": ["postgres-array"]},
            "value.json": {"rust_api": "pgorm::pgorm_query::Value::Json", "features": ["with-json"]},
            "value.null": {"rust_api": "pgorm::pgorm_query::Value", "features": []},
            "value.snapshot": {"rust_api": "pgorm_python::values::PyValue", "features": []},
            "value.range": {"rust_api": "pgorm::pgorm_query::{Range, Multirange, Value::Range, Value::Multirange}", "features": []},
            "value.created_range": {"rust_api": "pgorm::pgorm_query::{Range, Multirange} Display/FromStr over RangeSubtype, Value::String, Expr::as_range, ColumnType::{CreatedRange, CreatedMultirange}", "features": []},
            "type_name": {"rust_api": "pgorm::pgorm_query::TypeName", "features": []},
            "model.declare": {"rust_api": "pgorm::pgorm_query::{NamedTable, Value}", "features": [], "python_convenience": true},
            "model.column": {"rust_api": "pgorm::pgorm_query::{Expr, SimpleExpr, Value}", "features": [], "python_convenience": true},
            "model.select": {"rust_api": "pgorm::pgorm_query::SelectStatement", "features": [], "python_convenience": true},
            "model.insert": {"rust_api": "pgorm::pgorm_query::InsertStatement", "features": [], "python_convenience": true},
            "model.update": {"rust_api": "pgorm::pgorm_query::UpdateStatement", "features": [], "python_convenience": true},
            "model.delete": {"rust_api": "pgorm::pgorm_query::DeleteStatement", "features": [], "python_convenience": true},
            "model.records": {"rust_api": "pgorm::ConnectionTrait::{query_all, query_one} / pgorm_python::results", "features": [], "python_convenience": true},
            "model.relation": {"rust_api": "pgorm::RelationDef, join_condition", "features": [], "python_convenience": true},
            "model.graph": {"rust_api": "pgorm::SelectGraph, result_column_name", "features": [], "python_convenience": true},
            "model.cursor": {"rust_api": "pgorm::Cursor", "features": [], "python_convenience": true},
            "model.paginate": {"rust_api": "pgorm::Paginator", "features": [], "python_convenience": true}
        },
        "value_types": ArrayType::kinds().map(|kind| kind.kind_name()).chain(["enum", "created_range", "created_multirange", "array"]).collect::<Vec<_>>(),
        "value_policy": {
            "inferred_integer": "i64", "inferred_float": "f64", "array_dimensions": 1,
            "float32": "exact conversion only", "decimal": "96-bit coefficient, scale 0–28",
            "temporal_precision": "microseconds; finer precision and subsecond offsets rejected",
            "interval": "pgorm.Interval: months, days and microseconds, each signed, never normalised into one another",
            "aware_datetime": "explicit UTC tag requiring a zero offset",
            "enum_storage": "Rust String value with qualified TypeName metadata",
            "created_range_storage": "Rust String of the text form with the qualified type name and subtype; written as CAST(text AS name)",
            "snapshot": "version 1, tagged JSON with integer strings and IEEE float bits"
        },
        "expression_functions": {
            "lower": [1], "upper": [1], "abs": [1], "char_length": [1],
            "count": [1], "count_distinct": [1], "sum": [1], "avg": [1],
            "min": [1], "max": [1], "round": [1, 2], "coalesce": {"min_args": 1},
            "random": [0], "gen_random_uuid": [0], "uuidv4": [0], "uuidv7": [0, 1],
            "uuid_extract_timestamp": [1], "uuid_extract_version": [1],
            "generate_series": [2, 3], "unnest": [1],
            "json_to_record": [1], "jsonb_to_record": [1],
            "json_to_recordset": [1], "jsonb_to_recordset": [1]
        },
        "result_forms": ["pool", "connection", "bool", "expression", "json_input", "pending_merge", "merge", "with", "condition", "compiled", "projection", "ordering", "record", "optional_record", "records", "affected_count", "async_stream", "entity", "entity_query", "entity_model", "active_model", "active_value", "graph", "graph_query", "graph_cursor", "graph_tuple", "model_descriptor", "model_column", "model_query", "model_write", "model_records", "pipeline", "pipeline_expression", "pipeline_binder", "pipeline_source", "pipeline_grouped", "pipeline_window", "source_selection", "selected_sources", "ddl", "create_table", "create_index", "entity_schema", "ddl_column", "ddl_type", "transaction"],
        "result_policy": {
            "scope": "dynamic Record results",
            "decode": "Rust Row::try_get / FromSql, Value conversion with exactness checks",
            "names": "unique output names required; explicit aliases resolve duplicates",
            "timestamptz": "UTC; PostgreSQL does not retain the input timezone",
            "numeric": "exact 96-bit Decimal coefficient and scale 0–28; other values rejected",
            "json": "serde_json i64/u64/f64; numeric value loss and nesting beyond 64 rejected",
            "arrays": "one dimension, lower bound 1, nullable elements; other shapes rejected",
            "ranges": "the six built-in range types and their multiranges; a range type a schema created over any RangeSubtype as its own created_range kind, named by the column's type, held as its text form; created multiranges, which the driver reports as simple types, and arrays of a created range rejected",
            "unsupported": ["domain", "composite", "timetz", "bit", "money"],
            "stream": "one pull at a time over bounded driver buffers; owns connection until EOF or close"
        },
        "entity_policy": {
            "registration": "concrete Rust types compiled into the same native module",
            "decode": "registered Model::from_query_result followed by checked Python Value conversion",
            "input": "SQL ColumnType hints for plain values; explicit Value tags and Rust setters remain authoritative",
            "one": "Rust Select::one/one_opt use LIMIT 1; all returns every selected model",
            "writes": "real ActiveModel insert/update/delete and before/after hooks on a clone",
            "execution": "acquired Connection or Transaction; native asyncio awaitable",
            "stream": false
        },
        "graph_policy": {
            "source_arities": [1, 2, 3, 4, 5, 6, 7],
            "slots": ["Req", "Opt"], "registration_required": true,
            "decode": "SelectGraph / GraphRow with actual registered models and optional slots",
            "result": "bare model for one source; tuple in source order for joined shapes",
            "cursor": "one root column plus Rust's declared primary-key tiebreaks",
            "unsupported": ["unregistered shapes", "source arity beyond seven", "graph.stream", "graph.grouped", "slot-column cursors", "composite order-column cursors", "cursor SQL inspection"]
        },
        "codegen_policy": {
            "workflow": ["pgorm.codegen scaffold", "native application build and install", "pgorm.codegen emit", "final wheel build and install"],
            "input_schema_version": 1,
            "typing_source": "FromQueryResult::expected_columns Rust field spellings; unknown custom types remain Any",
            "compatibility": ["package_version", "pgorm_version", "registry_abi", "features", "compiled entity and graph metadata"],
            "python_conveniences": {
                "model properties": "EntityModel lookup over ModelTrait::get with checked Value conversion",
                "set_<field>": "typed Value construction then ActiveModelTrait::set on a cloned native model",
                "model and active views": "registered native EntityModel and ActiveModel methods",
                "entity query views": "registered native EntityQuery methods over Select<Entity>",
                "graph and cursor views": "registered native EntityGraphQuery and EntityGraphCursor methods over SelectGraph; wrap decoded models in concrete Python views"
            }
        },
        "model_policy": {
            "declarations": "Python metadata over native runtime statements; no compiled entity derives or hooks",
            "field_identity": "each field is read from the result column its statement projected it under: its SQL column, or a graph source's s{i}_ name",
            "values": "declared native Value tags, exact nullability and qualified enum/array identity",
            "scalar_kinds": scalar_kinds,
            "enum": "schema-qualified TypeName required", "array_dimensions": 1,
            "writes": "missing mapping entry is omitted; None is SQL NULL; Value.json(None) is JSON null",
            "one": "strict dynamic Record cardinality; explicit limit selects a first row",
            "unsupported_kinds": ["u64", "char"],
            "stream": true, "automatic_ddl": false
        },
        "tls": {"modes": ["verify-full", "disable"], "default": "verify-full unless DSN explicitly disables TLS", "ca": "PEM, or the platform's trust store"},
        "pipeline_policy": {
            "construction": "owned native Pipeline; runtime table/entity/named pipeline sources",
            "parameters": "synchronous *_with callback; each binding minted once inside its real Rust brand",
            "literals": ["null", "bool", "i32", "i64", "finite f64", "text"],
            "bound_list_max": 32, "literal_list_max": null,
            "qualified_enum_binding": false,
            "source_arities": [1, 2, 3, 4, 5, 6],
            "source_result": "tuple of optional registered models; (None,) is distinct from no row",
            "record_terminals": "all / one / optional on acquired Connection or Transaction; one and optional append take(1)",
            "stream": "dynamic Record through pool.stream or connection.stream",
            "selected_sources_stream": false
        },
        "transaction_policy": {
            "ownership": "scoped Rust owner task holds borrowed DatabaseTransaction; parent reserved through native completion",
            "modes": ["default", "read_write", "read_only", "deferrable"],
            "isolation": ["read_uncommitted", "read_committed", "repeatable_read", "serializable"],
            "nested": "real native savepoints; each parent reserved until its child completes",
            "contexts": "async with only; commit on normal exit; rollback on exceptional exit; a failed ROLLBACK raised in place of the block's exception unless that is a cancellation; uncertain state discarded",
            "cancellation": "cancelled in-flight requests discard the entire connection; a cancelled write may have an unknown outcome",
            "abandoned_idle_rollback_timeout_seconds": 5,
            "registered": ["entity queries", "ActiveModel writes and hooks", "graphs", "cursors", "pipeline source tuples"],
            "implicit_retry": false, "stream": false
        },
        "schema_policy": {
            "construction": "owned Rust DDL builders; no database work at construction or import",
            "execution": "explicit Pool.execute or Connection.execute",
            "values": "Rust DDL literal rendering; inspect.params is empty",
            "runtime_ddl": true, "automatic_ddl": false,
            "registered_entity_ddl": "real Schema methods for a compiled entity; registrations.entities lists available types",
            "index_methods": "any access method the server has, named by identifier",
            "column_types": column_types,
            "named_type": "TypeName retains schema qualification; DataType.array retains element identity",
            "type_changes": "native driver caches type metadata; close and recreate application pools after altering existing enum labels or type names",
            "unsupported": ["schema introspection", "automatic migrations", "raw column options", "concurrent indexes"]
        },
        "registrations": {"entities": [], "graphs": [], "sources": [], "disjoints": [], "one_ofs": []},
        "python": {"abi": "cp314", "free_threading": false, "subinterpreters": false}
    });
    // The release pgorm targets: 18 by default, 19 when it was built with
    // pg-19, through this crate's feature or any other's.
    // [spec:pgorm:req:sql.target+1]
    if pgorm::pgorm_query::POSTGRES_TARGET == 19
        && let Some(features) = manifest["features"].as_array_mut()
    {
        features.push("pg-19".into());
    }
    if let Some(operations) = manifest["operations"].as_object_mut() {
        operations.extend(crate::expressions::capabilities());
        operations.extend(crate::json::capabilities());
        operations.extend(crate::statements::capabilities());
        operations.extend(crate::results::capabilities());
        operations.extend(crate::entities::capabilities());
        for (name, api) in crate::entities::version_terminals
            .into_iter()
            .chain(crate::entities::insert_terminals)
        {
            operations.insert(
                name.to_owned(),
                json!({"rust_api": api, "features": [], "registration_required": true}),
            );
        }
        operations.extend(crate::graphs::capabilities());
        for (name, api) in crate::disjoint::OPERATIONS {
            operations.insert(
                name.to_owned(),
                json!({"rust_api": api, "features": [], "registration_required": name == "disjoint_entity"}),
            );
        }
        for (name, api) in crate::one_of::OPERATIONS {
            operations.insert(
                name.to_owned(),
                json!({"rust_api": api, "features": [], "registration_required": name == "one_of_entity"}),
            );
        }
        operations.extend(crate::pipeline::capabilities());
        operations.extend(crate::schema::capabilities());
        operations.extend(crate::transactions::capabilities());
        operations.extend(crate::copy::capabilities());
        // [spec:pgorm:req:bindings.connections+2]
        operations.insert(
            "pool.statement_cache_channel".to_owned(),
            json!({"rust_api": "pgorm::ManagerConfig::statement_cache_channel", "features": []}),
        );
    }
    if let Some(forms) = manifest["result_forms"].as_array_mut() {
        forms.extend(["copy_from", "copy_to", "copy_count", "copy_stream"].map(Value::from));
    }
    manifest["copy_policy"] = crate::copy::policy();
    manifest["listen_policy"] = crate::listen::policy();
    manifest
}

/// Return a fresh, versioned description of this native build's public surface.
#[pyfunction(pass_module)]
pub(crate) fn capabilities<'py>(module: &Bound<'py, PyModule>) -> PyResult<Bound<'py, PyAny>> {
    let mut manifest = manifest();
    let registry_object = module.getattr("_registry")?;
    let registry = registry_object.extract::<PyRef<'_, crate::entities::NativeRegistry>>()?;
    manifest["registrations"]["entities"] = registry.0.describe().into();
    manifest["registrations"]["graphs"] = registry
        .0
        .graphs
        .values()
        .map(|graph| graph.info().describe())
        .collect::<Vec<_>>()
        .into();
    manifest["registrations"]["disjoints"] = registry
        .0
        .disjoints
        .by_name
        .values()
        .map(|disjoint| disjoint.description.clone())
        .collect::<Vec<_>>()
        .into();
    manifest["registrations"]["one_ofs"] = registry
        .0
        .one_ofs
        .by_name
        .values()
        .map(|one_of| one_of.description.clone())
        .collect::<Vec<_>>()
        .into();
    manifest["registrations"]["sources"] = registry
        .0
        .sources
        .values()
        .map(|source| source.info().describe())
        .collect::<Vec<_>>()
        .into();
    module
        .py()
        .import("json")?
        .call_method1("loads", (manifest.to_string(),))
}

/// Fail explicitly when this build does not expose the requested operation.
#[pyfunction]
pub(crate) fn require_capability(operation: &str) -> PyResult<()> {
    if manifest()["operations"].get(operation).is_some() {
        Ok(())
    } else {
        Err(UnsupportedCapabilityError::new_err(format!(
            "this pgorm build does not support operation {operation:?}"
        )))
    }
}
