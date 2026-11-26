"""Runtime descriptors, registered Entity/ActiveModel and SelectGraph paths."""


def column_kind(kind, p):
    if isinstance(kind, dict):
        schema, name = kind["enum"]
        return p.TypeName(name, schema=schema), False
    array = kind.endswith("[]")
    base = kind[:-2] if array else kind
    return {"timestamp": "datetime", "timestamptz": "datetime_utc"}.get(
        base, base
    ), array


def descriptor(table, fields, fixture, p):
    from pgorm.models import Column, Model

    definition = next(
        (
            item
            for item in fixture["tables"]
            if (item["schema"], item["name"]) == (table.schema, table.name)
        ),
        None,
    )
    if definition is None:
        raise p.ConstructionError("runtime model requires declared fixture metadata")
    columns = {column["name"]: column for column in definition["columns"]}
    mapping = fields if fields is not None else {name: name for name in columns}
    declarations = {}
    for field, physical in mapping.items():
        column = columns[physical]
        kind, array = column_kind(column["kind"], p)
        declarations[field] = Column(
            kind,
            name=physical,
            array=array,
            nullable=column["nullable"],
            primary_key=column["primary"],
        )
    return Model(table, declarations)


def dispatch(name, i, d, p, fixture):
    match name:
        case "model":
            return descriptor(i["table"], d.get("fields"), fixture, p), [
                "pgorm_query::TableRef",
                "pgorm_query::ColumnType",
            ]
        case "model.column":
            return i["model"].col(d["name"]).expr(), ["pgorm_query::Expr::col"]
        case "model.select":
            query = i["model"].select()
            if "predicate" in i:
                query = query.where(i["predicate"])
            if i["order"]:
                query = query.order_by(*i["order"])
            return query, ["pgorm_query::Query::select"]
        case "model.write":
            method = d["method"]
            values = dict(zip(d["columns"], i["values"], strict=True))
            if method == "delete" and values:
                raise p.ConstructionError("delete cannot carry assignments")
            if method == "insert" and "predicate" in i:
                raise p.ConstructionError("insert cannot carry a predicate")
            query = (
                i["model"].delete()
                if method == "delete"
                else getattr(i["model"], method)(values)
            )
            if "predicate" in i:
                query = query.where(i["predicate"])
            return query, ["pgorm_query::Query::" + method]
        case "model.returning":
            return i["query"].returning(*d["columns"]), ["pgorm_query::ReturningClause"]
        case "entity":
            return p.entity(d["name"]), ["pgorm::Entity"]
        case "entity.column":
            return i["entity"].col(d["name"]).expr(), ["pgorm::Column::into_expr"]
        case "entity.predicate":
            return getattr(i["entity"].col(d["column"]), d["operator"])(i["value"]), [
                "pgorm::Column::" + d["operator"]
            ]
        case "entity.find":
            return i["entity"].find(), ["pgorm::Entity::find"]
        case "entity.filter" | "graph.filter":
            return i["query"].where(i["predicate"]), ["pgorm::QueryFilter::filter"]
        case "entity.order" | "graph.order":
            return i["query"].order_by(*i["keys"]), ["pgorm::QueryOrder::order_by"]
        case "entity.active":
            return i["entity"].active(), ["pgorm::ActiveModelBehavior::new"]
        case "entity.into_active":
            return i["model"].into_active(), [
                "pgorm::IntoActiveModel::into_active_model"
            ]
        case "active.set":
            if d["state"] == "set":
                return i["model"].set(d["column"], i["value"]), [
                    "pgorm::ActiveModel::set"
                ]
            if "value" in i:
                raise p.ConstructionError("reset and not_set cannot receive a value")
            return getattr(i["model"], d["state"])(d["column"]), [
                "pgorm::ActiveModel::" + d["state"]
            ]
        case "graph":
            return p.graph(d["name"]), ["pgorm::SelectGraph"]
        case "graph.find":
            query = i["graph"].find(aliases=d["aliases"])
            paths = ["pgorm::Entity::graph"]
            if d["aliases"]:
                method = (
                    "join_one_as"
                    if i["graph"].name == "campaign.RequiredNotes"
                    else "join_maybe_as"
                )
                paths.extend(["pgorm::SelectGraph::" + method] * len(d["aliases"]))
            return query, paths
        case "graph.column":
            return i["query"].col(d["source"], d["column"]), ["pgorm_query::Expr::col"]
        case "graph.cursor":
            return i["query"].cursor(d["column"]), ["pgorm::SelectGraph::cursor_by"]
        case "cursor.bound":
            method = d["side"] + "_with"
            return getattr(i["cursor"], method)(*i["values"]), [
                "pgorm::Cursor::" + method
            ]
        case "cursor.page":
            cursor = getattr(i["cursor"], d["side"])(d["count"])
            return getattr(cursor, d["direction"])(), [
                "pgorm::Cursor::" + d["side"],
                "pgorm::Cursor::" + d["direction"],
            ]
    raise RuntimeError("inactive registered/model dispatch: " + name)
