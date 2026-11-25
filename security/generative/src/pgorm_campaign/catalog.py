"""Closed typed instruction catalog; each entry names its actual Rust API path."""

from dataclasses import dataclass

VERSION = 1


@dataclass(frozen=True)
class Operation:
    output: str
    inputs: dict[str, str]
    data: dict[str, object]
    rust: tuple[str, ...]
    family: str


def op(output, inputs, data, rust, family):
    return Operation(output, inputs, data, tuple(rust.split(";")), family)


COMPARISONS = ("eq", "ne", "lt", "lte", "gt", "gte")
BINARY = COMPARISONS + ("add", "sub", "mul", "div", "mod", "and", "or")
SCALAR = "expr|value"
PREDICATE = "expr|condition"
WRITE = "insert|update|delete"
QUERY = "select|insert|update|delete|raw|ddl|pipeline|entity_query|graph_query|cursor|sources|model_query|model_write"

# [spec:pgorm:def:generative.program]
# [spec:pgorm:req:generative.matrix]
OPERATIONS = {
    "value": op("value", {}, {"value": "value"}, "pgorm_query::Value", "types"),
    "name": op("name", {"value": "value"}, {}, "pgorm_query::Name", "names"),
    "result.value": op(
        "value",
        {},
        {"step": "id", "row": "nonnegative", "column": "identifier", "type": "type"},
        "pgorm_driver::Row::try_get",
        "sequences",
    ),
    "table": op(
        "table",
        {"name": "name?"},
        {"name": "identifier?", "schema": "identifier?", "alias": "identifier?"},
        "pgorm_query::TableRef",
        "names",
    ),
    "expr.column": op(
        "expr",
        {"table": "table?", "name": "name?"},
        {"name": "identifier?"},
        "pgorm_query::Expr::col",
        "names",
    ),
    "expr.value": op(
        "expr",
        {"value": "value"},
        {"mode": ("literal", "bound")},
        "pgorm_query::SimpleExpr::Constant;pgorm_query::Expr::value",
        "types",
    ),
    "expr.binary": op(
        "expr",
        {"left": SCALAR, "right": SCALAR},
        {"operator": BINARY},
        "pgorm_query::SimpleExpr;pgorm_query::Expr",
        "conditions",
    ),
    "expr.unary": op(
        "expr",
        {"value": "expr"},
        {"operator": ("not", "is_null", "is_not_null")},
        "pgorm_query::SimpleExpr;pgorm_query::Expr",
        "conditions",
    ),
    "expr.membership": op(
        "expr",
        {"value": "expr", "items": "value*"},
        {"negated": "bool"},
        "pgorm_query::Expr::is_in;pgorm_query::Expr::is_not_in",
        "conditions",
    ),
    "expr.pattern": op(
        "expr",
        {"value": "expr", "pattern": "value"},
        {
            "method": (
                "contains_text",
                "starts_with",
                "ends_with",
                "like",
                "not_like",
                "ilike",
                "not_ilike",
            ),
            "escape": "character?",
        },
        "pgorm_query::SimpleExpr;pgorm_query::Func;pgorm_query::LikeExpr",
        "patterns",
    ),
    "expr.call": op(
        "expr",
        {"arguments": "expr|value*"},
        {"name": "string"},
        "pgorm_query::Func",
        "names",
    ),
    "expr.cast": op(
        "expr",
        {"value": "expr"},
        {"name": "identifier", "schema": "identifier?", "array": "bool?"},
        "pgorm_query::SimpleExpr::cast_as_type;pgorm_query::TypeName",
        "names",
    ),
    "expr.alias": op(
        "projection",
        {"value": "expr", "name": "name?"},
        {"name": "identifier?"},
        "pgorm_query::SelectStatement::expr_as",
        "names",
    ),
    "expr.order": op(
        "order",
        {"value": "expr"},
        {"direction": ("asc", "desc"), "nulls": ("first", "last", "default")},
        "pgorm_query::OrderedStatement",
        "names",
    ),
    "condition": op(
        "condition",
        {"items": "expr|condition*"},
        {"mode": ("all", "any"), "negated": "bool?"},
        "pgorm_query::Condition",
        "conditions",
    ),
    "select": op(
        "select",
        {"columns": "expr|projection*"},
        {},
        "pgorm_query::Query::select;pgorm_query::SelectStatement::exprs",
        "crud",
    ),
    "select.from": op(
        "select",
        {"query": "select", "table": "table"},
        {},
        "pgorm_query::SelectStatement::from",
        "crud",
    ),
    "select.filter": op(
        "select",
        {"query": "select", "predicate": PREDICATE},
        {},
        "pgorm_query::ConditionalStatement::cond_where",
        "conditions",
    ),
    "select.join": op(
        "select",
        {"query": "select", "table": "table", "on": PREDICATE + "?"},
        {"kind": ("inner", "left", "right", "full", "cross")},
        "pgorm_query::SelectStatement::join;pgorm_query::SelectStatement::cross_join",
        "crud",
    ),
    "select.group": op(
        "select",
        {"query": "select", "keys": "expr*"},
        {},
        "pgorm_query::SelectStatement::group_by_columns",
        "crud",
    ),
    "select.having": op(
        "select",
        {"query": "select", "predicate": PREDICATE},
        {},
        "pgorm_query::SelectStatement::cond_having",
        "crud",
    ),
    "select.order": op(
        "select",
        {"query": "select", "keys": "order*"},
        {},
        "pgorm_query::OrderedStatement",
        "crud",
    ),
    "select.page": op(
        "select",
        {"query": "select"},
        {"limit": "nonnegative?", "offset": "nonnegative?"},
        "pgorm_query::SelectStatement::limit;pgorm_query::SelectStatement::offset",
        "crud",
    ),
    "select.distinct": op(
        "select",
        {"query": "select"},
        {},
        "pgorm_query::SelectStatement::distinct",
        "crud",
    ),
    "insert": op(
        "insert",
        {"table": "table"},
        {"columns": "identifiers"},
        "pgorm_query::Query::insert;pgorm_query::InsertStatement::into_table",
        "crud",
    ),
    "insert.row": op(
        "insert",
        {"query": "insert", "values": "expr|value*"},
        {},
        "pgorm_query::InsertStatement::values",
        "crud",
    ),
    "insert.defaults": op(
        "insert",
        {"query": "insert"},
        {},
        "pgorm_query::InsertStatement::or_default_values",
        "crud",
    ),
    "insert.conflict": op(
        "insert",
        {"query": "insert"},
        {
            "keys": "identifiers",
            "action": ("nothing", "update"),
            "columns": "identifiers?",
        },
        "pgorm_query::OnConflict",
        "crud",
    ),
    "update": op(
        "update", {"table": "table"}, {}, "pgorm_query::Query::update", "crud"
    ),
    "update.set": op(
        "update",
        {"query": "update", "value": SCALAR},
        {"column": "identifier"},
        "pgorm_query::UpdateStatement::value",
        "crud",
    ),
    "delete": op(
        "delete", {"table": "table"}, {}, "pgorm_query::Query::delete", "crud"
    ),
    "write.filter": op(
        "same",
        {"query": "update|delete", "predicate": PREDICATE},
        {},
        "pgorm_query::ConditionalStatement::cond_where",
        "crud",
    ),
    "write.all": op(
        "same",
        {"query": "update|delete"},
        {},
        "pgorm_query::UpdateStatement;pgorm_query::DeleteStatement",
        "crud",
    ),
    "write.returning": op(
        "same",
        {"query": WRITE, "columns": "expr|projection*"},
        {},
        "pgorm_query::ReturningClause",
        "crud",
    ),
    "raw.template": op(
        "raw",
        {"parameters": "value*"},
        {"text": "string"},
        "pgorm_query::inject_parameters",
        "patterns",
    ),
    "model": op(
        "model",
        {"table": "table"},
        {"fields": "field_map?"},
        "pgorm_query::TableRef;pgorm_query::ColumnType",
        "models",
    ),
    "model.column": op(
        "expr",
        {"model": "model"},
        {"name": "identifier"},
        "pgorm_query::Expr::col",
        "models",
    ),
    "model.select": op(
        "model_query",
        {"model": "model", "predicate": PREDICATE + "?", "order": "order*"},
        {},
        "pgorm_query::Query::select",
        "models",
    ),
    "model.write": op(
        "model_write",
        {"model": "model", "values": "value*", "predicate": PREDICATE + "?"},
        {"method": ("insert", "update", "delete"), "columns": "identifiers"},
        "pgorm_query::Query::insert;pgorm_query::Query::update;pgorm_query::Query::delete",
        "models",
    ),
    "model.returning": op(
        "model_query",
        {"query": "model_write"},
        {"columns": "identifiers"},
        "pgorm_query::ReturningClause",
        "models",
    ),
    "entity": op("entity", {}, {"name": "registration"}, "pgorm::Entity", "entities"),
    "entity.column": op(
        "expr",
        {"entity": "entity"},
        {"name": "identifier"},
        "pgorm::Column::into_expr",
        "entities",
    ),
    "entity.predicate": op(
        "expr",
        {"entity": "entity", "value": "value"},
        {"column": "identifier", "operator": COMPARISONS},
        "pgorm::Column",
        "entities",
    ),
    "entity.find": op(
        "entity_query", {"entity": "entity"}, {}, "pgorm::Entity::find", "entities"
    ),
    "entity.filter": op(
        "entity_query",
        {"query": "entity_query", "predicate": PREDICATE},
        {},
        "pgorm::QueryFilter::filter",
        "entities",
    ),
    "entity.order": op(
        "entity_query",
        {"query": "entity_query", "keys": "order*"},
        {},
        "pgorm::QueryOrder",
        "entities",
    ),
    "entity.page": op(
        "entity_query",
        {"query": "entity_query"},
        {"limit": "nonnegative?", "offset": "nonnegative?"},
        "pgorm::QuerySelect",
        "entities",
    ),
    "entity.active": op(
        "active",
        {"entity": "entity"},
        {},
        "pgorm::ActiveModelBehavior::new",
        "entities",
    ),
    "entity.result": op(
        "entity_model",
        {},
        {"step": "id", "row": "nonnegative", "source": "nonnegative?"},
        "pgorm::Model",
        "entities",
    ),
    "entity.into_active": op(
        "active",
        {"model": "entity_model"},
        {},
        "pgorm::IntoActiveModel::into_active_model",
        "entities",
    ),
    "active.set": op(
        "active",
        {"model": "active", "value": "value?"},
        {"column": "identifier", "state": ("set", "reset", "not_set")},
        "pgorm::ActiveValue;pgorm::ActiveModel",
        "entities",
    ),
    "graph": op("graph", {}, {"name": "registration"}, "pgorm::SelectGraph", "graph"),
    "graph.find": op(
        "graph_query",
        {"graph": "graph"},
        {"aliases": "identifiers"},
        "pgorm::Entity::graph;pgorm::SelectGraph::join_maybe_as;pgorm::SelectGraph::join_one_as",
        "graph",
    ),
    "graph.column": op(
        "expr",
        {"query": "graph_query"},
        {"source": "nonnegative", "column": "identifier"},
        "pgorm_query::Expr::col",
        "graph",
    ),
    "graph.filter": op(
        "graph_query",
        {"query": "graph_query", "predicate": PREDICATE},
        {},
        "pgorm::QueryFilter::filter",
        "graph",
    ),
    "graph.order": op(
        "graph_query",
        {"query": "graph_query", "keys": "order*"},
        {},
        "pgorm::QueryOrder::order_by",
        "graph",
    ),
    "graph.cursor": op(
        "cursor",
        {"query": "graph_query"},
        {"column": "identifier"},
        "pgorm::SelectGraph::cursor_by",
        "graph",
    ),
    "cursor.bound": op(
        "cursor",
        {"cursor": "cursor", "values": "value*"},
        {"side": ("before", "after")},
        "pgorm::Cursor::before_with;pgorm::Cursor::after_with",
        "graph",
    ),
    "cursor.page": op(
        "cursor",
        {"cursor": "cursor"},
        {"side": ("first", "last"), "count": "positive", "direction": ("asc", "desc")},
        "pgorm::Cursor::first;pgorm::Cursor::last",
        "graph",
    ),
    "pipeline.source": op(
        "source",
        {"source": "table|entity|pipeline"},
        {"alias": "identifier?"},
        "pgorm::pipeline::IntoSource;pgorm::pipeline::Source::named",
        "pipeline",
    ),
    "pipeline.from": op(
        "pipeline",
        {"source": "table|entity|pipeline|source"},
        {},
        "pgorm::pipeline::Pipeline::from",
        "pipeline",
    ),
    "pipeline.column": op(
        "pexpr",
        {},
        {"source": "identifier", "column": "identifier"},
        "pgorm::pipeline::col",
        "pipeline",
    ),
    "pipeline.alias": op(
        "pexpr", {}, {"name": "identifier"}, "pgorm::pipeline::alias", "pipeline"
    ),
    "pipeline.value": op(
        "pexpr", {"value": "value"}, {}, "pgorm::pipeline::literal", "pipeline"
    ),
    "pipeline.bind": op(
        "pexpr", {"value": "value"}, {}, "pgorm::pipeline::Binder::bind", "pipeline"
    ),
    "pipeline.binary": op(
        "pexpr",
        {"left": "pexpr", "right": "pexpr"},
        {"operator": BINARY},
        "pgorm::pipeline::ExprOps",
        "pipeline",
    ),
    "pipeline.unary": op(
        "pexpr",
        {"value": "pexpr"},
        {"operator": ("not", "neg", "is_null", "is_not_null", "asc", "desc")},
        "pgorm::pipeline::ExprOps",
        "pipeline",
    ),
    "pipeline.membership": op(
        "pexpr",
        {"value": "pexpr", "items": "pexpr*"},
        {},
        "pgorm::pipeline::ExprOps::in_array",
        "pipeline",
    ),
    "pipeline.function": op(
        "pexpr",
        {"arguments": "pexpr*"},
        {
            "name": (
                "sum",
                "min",
                "max",
                "average",
                "count",
                "count_distinct",
                "count_rows",
                "row_number",
                "first",
                "last",
                "rank",
                "rank_dense",
            )
        },
        "pgorm::pipeline::sum;pgorm::pipeline::count;pgorm::pipeline::row_number",
        "pipeline",
    ),
    "pipeline.cast": op(
        "pexpr",
        {"value": "pexpr"},
        {"name": "string"},
        "pgorm::pipeline::ExprOps::cast",
        "pipeline",
    ),
    "pipeline.named": op(
        "pexpr",
        {"value": "pexpr"},
        {"name": "identifier"},
        "pgorm::pipeline::ExprOps::as_",
        "pipeline",
    ),
    "pipeline.filter": op(
        "pipeline",
        {"query": "pipeline", "predicate": "pexpr"},
        {"binder": "id?"},
        "pgorm::pipeline::Pipeline::filter;pgorm::pipeline::Pipeline::filter_with",
        "pipeline",
    ),
    "pipeline.derive": op(
        "pipeline",
        {"query": "pipeline", "columns": "pexpr*"},
        {"binder": "id?"},
        "pgorm::pipeline::Pipeline::derive;pgorm::pipeline::Pipeline::derive_with",
        "pipeline",
    ),
    "pipeline.select": op(
        "pipeline",
        {"query": "pipeline", "columns": "pexpr*"},
        {"binder": "id?"},
        "pgorm::pipeline::Pipeline::select;pgorm::pipeline::Pipeline::select_with",
        "pipeline",
    ),
    "pipeline.group": op(
        "grouped",
        {"query": "pipeline", "keys": "pexpr*"},
        {"binder": "id?"},
        "pgorm::pipeline::Pipeline::group;pgorm::pipeline::Pipeline::group_with",
        "pipeline",
    ),
    "pipeline.aggregate": op(
        "pipeline",
        {"query": "grouped", "columns": "pexpr*"},
        {"binder": "id?"},
        "pgorm::pipeline::Grouped::aggregate;pgorm::pipeline::Grouped::aggregate_with",
        "pipeline",
    ),
    "pipeline.window": op(
        "pipeline",
        {
            "query": "pipeline",
            "columns": "pexpr*",
            "partition": "pexpr*",
            "order": "pexpr*",
        },
        {"binder": "id?", "start": "integer?", "end": "integer?"},
        "pgorm::pipeline::Pipeline::window;pgorm::pipeline::Over",
        "pipeline",
    ),
    "pipeline.sort": op(
        "pipeline",
        {"query": "pipeline", "keys": "pexpr*"},
        {"binder": "id?"},
        "pgorm::pipeline::Pipeline::sort;pgorm::pipeline::Pipeline::sort_with",
        "pipeline",
    ),
    "pipeline.take": op(
        "pipeline",
        {"query": "pipeline"},
        {"start": "nonnegative", "end": "nonnegative"},
        "pgorm::pipeline::Pipeline::take_range",
        "pipeline",
    ),
    "pipeline.join": op(
        "pipeline",
        {"query": "pipeline", "source": "source|pipeline|table|entity", "on": "pexpr"},
        {"kind": ("inner", "left", "right", "full"), "binder": "id?"},
        "pgorm::pipeline::Pipeline::join;pgorm::pipeline::Pipeline::join_with",
        "pipeline",
    ),
    "pipeline.set": op(
        "pipeline",
        {"query": "pipeline", "source": "pipeline"},
        {"method": ("append", "intersect", "remove")},
        "pgorm::pipeline::Pipeline::append;pgorm::pipeline::Pipeline::intersect;pgorm::pipeline::Pipeline::remove",
        "pipeline",
    ),
    "pipeline.distinct": op(
        "pipeline",
        {"query": "pipeline"},
        {},
        "pgorm::pipeline::Pipeline::distinct",
        "pipeline",
    ),
    "pipeline.sources": op(
        "sources",
        {"query": "pipeline"},
        {"name": "registration", "qualifiers": "identifiers"},
        "pgorm::pipeline::Pipeline::select_sources",
        "pipeline",
    ),
    "schema.create": op(
        "ddl",
        {"table": "table"},
        {"columns": "fixture_columns"},
        "pgorm_query::Table::create;pgorm_query::ColumnDef",
        "schema",
    ),
    "schema.drop": op(
        "ddl", {"table": "table"}, {}, "pgorm_query::Table::drop", "schema"
    ),
    "schema.rename": op(
        "ddl",
        {"table": "table"},
        {"name": "identifier", "column": "identifier?"},
        "pgorm_query::Table::rename;pgorm_query::TableAlterStatement::rename_column",
        "schema",
    ),
    "schema.index": op(
        "ddl",
        {"table": "table"},
        {"name": "identifier", "columns": "identifiers", "unique": "bool"},
        "pgorm_query::Index::create",
        "schema",
    ),
    "schema.enum": op(
        "ddl",
        {},
        {"name": "identifier", "schema": "identifier", "labels": "strings"},
        "pgorm_query::extension::postgres::Type::create",
        "schema",
    ),
    "schema.enum_change": op(
        "ddl",
        {},
        {
            "name": "identifier",
            "schema": "identifier",
            "method": ("add", "rename", "drop"),
            "value": "string?",
            "new_value": "string?",
        },
        "pgorm_query::extension::postgres::Type",
        "schema",
    ),
}


EFFECTS = {
    "fetch": op(
        "rows",
        {"query": QUERY},
        {"mode": ("all", "one", "optional"), "ordered": "bool"},
        "pgorm::Connection::query_raw;pgorm::Selector;pgorm::SelectGraph;pgorm::pipeline::Pipeline",
        "execution",
    ),
    "execute": op(
        "count",
        {"query": "insert|update|delete|raw|ddl|model_write"},
        {},
        "pgorm::Connection::execute_raw",
        "execution",
    ),
    "active.write": op(
        "model",
        {"model": "active"},
        {"method": ("insert", "update", "delete")},
        "pgorm::ActiveModel",
        "entities",
    ),
    # The write terminals that read a written row's two versions: each row is
    # a pair, the row before the write (absent for a row an upsert inserted)
    # and the row the write left. No ActiveModel hook runs around them.
    "entity.change": op(
        "rows",
        {"model": "active"},
        {},
        "pgorm::Update::one;pgorm::UpdateOne::exec_returning_change",
        "entities",
    ),
    "entity.changes": op(
        "rows",
        {"entity": "entity", "values": "value*", "predicate": PREDICATE},
        {"columns": "identifiers"},
        "pgorm::Update::many;pgorm::UpdateMany::col_expr;pgorm::UpdateMany::exec_returning_changes",
        "entities",
    ),
    "entity.upsert": op(
        "rows",
        {"models": "active*"},
        {"rows": ("one", "many"), "conflict": "identifiers", "update": "identifiers"},
        "pgorm::Insert::on_conflict;pgorm::Insert::exec_returning_upsert;pgorm::Insert::exec_returning_upserts",
        "entities",
    ),
    "begin": op(
        "transaction",
        {},
        {
            "child": "id",
            "mode": ("default", "read_only", "read_write"),
            "isolation": (
                "default",
                "read_committed",
                "repeatable_read",
                "serializable",
            ),
        },
        "pgorm::DatabaseConnection::transaction_with",
        "sequences",
    ),
    "commit": op("unit", {}, {}, "pgorm::Transact::transaction", "sequences"),
    "rollback": op("unit", {}, {}, "pgorm::Transact::transaction", "sequences"),
    "stream": op(
        "rows",
        {"query": "select|raw|pipeline"},
        {"take": "nonnegative", "cancel": "bool", "ordered": "bool"},
        "pgorm::Connection::query_raw;pgorm_driver::RowStream",
        "sequences",
    ),
    "inspect": op(
        "compiled",
        {"query": QUERY + "|expr|condition|pexpr"},
        {},
        "pgorm_query::QueryStatementBuilder;pgorm::pipeline::Pipeline::into_sql",
        "construction",
    ),
}
