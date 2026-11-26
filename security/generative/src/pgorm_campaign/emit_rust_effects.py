"""Effect steps and transaction scopes over the named pgorm terminals."""

from .emit_rust_models import TYPED_ROWS, graph_root
from .emit_rust_values import (
    PL,
    REPLAY,
    Q,
    UnsupportedInstruction,
    boolean,
    literal,
)

ISOLATION = {
    "read_committed": "ReadCommitted",
    "repeatable_read": "RepeatableRead",
    "serializable": "Serializable",
}
# Which pgorm terminal the Python binding selects for a fetch, by query type.
FETCH_PREFIX = {
    "entity_query": "pgorm::Select",
    "graph_query": "pgorm::SelectGraph",
    "cursor": "pgorm::Cursor",
    "pipeline": "pgorm::pipeline::Pipeline",
    "sources": "pgorm::pipeline::SelectedSources",
}
# Statement shapes a runtime model descriptor lowers to, which are the ordinary
# builder statements and carry the ordinary `build`.
MODEL_QUERIES = ("model_query", "model_write")


# [spec:pgorm:req:generative.replay]
class EffectEmitter:
    """Emit one scheduled effect, recording what the step observed."""

    def compile_query(self, reference):
        """The `(sql, values)` pair an effect runs, built by the named builder."""
        kind = self.types[reference]
        query = self.use(reference)
        if kind in ("select", "insert", "update", "delete") + MODEL_QUERIES:
            return f"{query}.build()"
        if kind == "raw":
            return query
        if kind == "ddl":
            return f"({query}, {Q}::Values(Vec::new()))"
        if kind in ("pipeline", "sources"):
            self.helpers.add("compiled")
            return f"compiled({query}.into_sql())?"
        if kind in ("entity_query", "graph_query"):
            # `Statement::build` is what the binding's `inspect` reaches for a
            # compiled query, and it takes the query by reference.
            return f"pgorm::Statement::build(&{query})"
        if kind == "cursor":
            raise UnsupportedInstruction(
                "pgorm::Cursor exposes no statement builder, so it cannot be inspected"
            )
        raise UnsupportedInstruction("no standalone Rust terminal for a " + kind)

    def row_observation(self, reference):
        """One decoded row of a typed read, in the shape the oracle produces."""
        kind = self.types[reference]
        if kind == "entity_query":
            registration = self.registration(reference)
            entity = self.entity_type(registration)
            return (
                f"{REPLAY}::observe::model::<{entity}>(row, {literal(registration)})?"
            )
        if kind in ("graph_query", "cursor"):
            name, _, slots = self.graph_shape(reference)
            registration = graph_root(name)
            root = self.entity_type(registration)
            if not slots:
                # A root-only graph decodes to the model itself, and the binding
                # reports it as the record rather than as a one-slot tuple.
                return (
                    f"{REPLAY}::observe::model::<{root}>(row, {literal(registration)})?"
                )
            items = [
                f"{REPLAY}::observe::model::<{root}>(&row.0, {literal(registration)})?"
            ]
            for index, (registration, required) in enumerate(slots, 1):
                entity = self.entity_type(registration)
                name = literal(registration)
                items.append(
                    f"{REPLAY}::observe::model::<{entity}>(&row.{index}, {name})?"
                    if required
                    else f"{REPLAY}::observe::maybe::<{entity}>"
                    f"(row.{index}.as_ref(), {name})?"
                )
            return f"{REPLAY}::observe::tuple(vec![{', '.join(items)}])"
        if kind == "sources":
            _, _, entities = self.source_shape(reference)
            items = []
            for index, registration in enumerate(entities):
                entity = self.entity_type(registration)
                # A single listed source decodes to a bare `Option<Model>`, but
                # it is still reported as a tuple: an absent one-source row is
                # `(None,)`, which is not the same as no row at all.
                slot = "row" if len(entities) == 1 else f"row.{index}"
                items.append(
                    f"{REPLAY}::observe::maybe::<{entity}>"
                    f"({slot}.as_ref(), {literal(registration)})?"
                )
            return f"{REPLAY}::observe::tuple(vec![{', '.join(items)}])"
        raise UnsupportedInstruction("no compiled row observation for a " + kind)

    def typed_fetch(self, step, pad, connection):
        """Read a terminal that decodes compiled models rather than raw rows."""
        d, i = step["data"], step["inputs"]
        reference, mode = i["query"], d["mode"]
        kind = self.types[reference]
        query = self.use(reference)
        lines = []
        if kind == "cursor":
            if mode != "all":
                raise UnsupportedInstruction(
                    "pgorm::Cursor has no public " + mode + " terminal"
                )
            lines.append(f"{pad}    let mut cursor = {query};")
            lines.append(f"{pad}    let rows = cursor.all(&{connection}).await?;")
        else:
            if kind == "graph_query" and mode == "one":
                raise UnsupportedInstruction(
                    "pgorm::SelectGraph has no public one terminal"
                )
            terminal = "one_opt" if mode == "optional" else mode
            call = f"{query}.{terminal}(&{connection}).await?"
            if mode == "all":
                lines.append(f"{pad}    let rows = {call};")
            elif mode == "one":
                lines.append(f"{pad}    let rows = vec![{call}];")
            else:
                lines.append(
                    f"{pad}    let rows = {call}.into_iter().collect::<Vec<_>>();"
                )
        lines.append(f"{pad}    let mut observed = Vec::with_capacity(rows.len());")
        lines.append(f"{pad}    for row in &rows {{")
        lines.append(f"{pad}        observed.push({self.row_observation(reference)});")
        lines.append(f"{pad}    }}")
        lines.append(
            f"{pad}    let observation = {REPLAY}::observe::collected(observed);"
        )
        return lines

    def fetch_paths(self, reference, mode):
        kind = self.types[reference]
        if kind == "pipeline":
            return [f"{PL}::Pipeline::into_sql", "pgorm::Connection::query_raw"]
        if kind in FETCH_PREFIX:
            method = "one_opt" if mode == "optional" else mode
            return [FETCH_PREFIX[kind] + "::" + method]
        return ["pgorm::Connection::query_raw"]

    def effect(self, step, lines, indent):
        name, d, i = step["op"], step["data"], step["inputs"]
        connection = self.connection(step["scope"])
        pad = indent
        if name == "fetch":
            keep = step["id"] in self.results
            if self.types[i["query"]] in TYPED_ROWS:
                body = self.typed_fetch(step, pad, connection)
            else:
                method = {
                    "all": "query_all",
                    "one": "query_one",
                    "optional": "query_opt",
                }[d["mode"]]
                wrap = {
                    "all": None,
                    "one": "vec![rows]",
                    "optional": "rows.into_iter().collect::<Vec<_>>()",
                }[d["mode"]]
                self.helpers.add("params")
                body = [
                    f"{pad}    let (sql, values) = {self.compile_query(i['query'])};",
                    f"{pad}    let held = holders(&values);",
                    (
                        f"{pad}    let rows = pgorm::Connection::{method}("
                        f"&{connection}, &sql, &params(&held)).await?;"
                    ),
                ]
                if wrap is not None:
                    body.append(f"{pad}    let rows = {wrap};")
                body.append(
                    f"{pad}    let observation = {REPLAY}::observe::rows(&rows)?;"
                )
            body.append(
                f"{pad}    Ok((observation, rows))"
                if keep
                else f"{pad}    Ok(observation)"
            )
            self.record(step, body, lines, pad, rows=keep)
        elif name == "stream":
            self.helpers.add("params")
            keep = step["id"] in self.results
            body = [
                f"{pad}    let (sql, values) = {self.compile_query(i['query'])};",
                f"{pad}    let held = holders(&values);",
                (
                    f"{pad}    let opened = pgorm::Connection::query_raw("
                    f"&{connection}, &sql, params(&held)).await?;"
                ),
                (
                    f"{pad}    let drained = {REPLAY}::stream::drain(opened, "
                    f"{int(d['take'])}usize, {boolean(d['cancel'])}).await?;"
                ),
                (
                    f"{pad}    let observation = {REPLAY}::observe::stream("
                    "&drained.rows, drained.complete, drained.cancelled, drained.closed)?;"
                ),
                f"{pad}    let rows = drained.rows;",
            ]
            body.append(
                f"{pad}    Ok((observation, rows))"
                if keep
                else f"{pad}    Ok(observation)"
            )
            self.record(step, body, lines, pad, rows=keep)
        elif name == "active.write":
            registration = self.registration(i["model"])
            model = self.use(i["model"])
            if d["method"] == "delete":
                body = [
                    (
                        f"{pad}    let count = pgorm::ActiveModel::delete("
                        f"{model}, &{connection}).await?;"
                    ),
                    f"{pad}    Ok({REPLAY}::observe::count(count))",
                ]
                self.record(step, body, lines, pad)
            else:
                keep = step["id"] in self.results
                entity = self.entity_type(registration)
                body = [
                    (
                        f"{pad}    let written = pgorm::ActiveModel::{d['method']}("
                        f"{model}, &{connection}).await?;"
                    ),
                    f"{pad}    let observation = {REPLAY}::observe::collected(vec![",
                    (
                        f"{pad}        {REPLAY}::observe::model::<{entity}>("
                        f"&written, {literal(registration)})?,"
                    ),
                    f"{pad}    ]);",
                ]
                body.append(
                    f"{pad}    Ok((observation, vec![written]))"
                    if keep
                    else f"{pad}    Ok(observation)"
                )
                self.record(step, body, lines, pad, rows=keep)
        elif name in ("entity.change", "entity.changes", "entity.upsert"):
            self.record(step, self.versions(step, connection, pad), lines, pad)
        elif name == "execute":
            self.helpers.add("params")
            body = [
                f"{pad}    let (sql, values) = {self.compile_query(i['query'])};",
                f"{pad}    let held = holders(&values);",
                (
                    f"{pad}    let count = pgorm::Connection::execute("
                    f"&{connection}, &sql, &params(&held)).await?;"
                ),
                f"{pad}    Ok({REPLAY}::observe::count(count))",
            ]
            self.record(step, body, lines, pad)
        elif name == "inspect":
            self.helpers.add("tagged")
            body = [
                f"{pad}    let (sql, values) = {self.compile_query(i['query'])};",
                f"{pad}    Ok({REPLAY}::observe::compiled(&sql, &tagged(&values))?)",
            ]
            self.record(step, body, lines, pad)
        else:
            raise UnsupportedInstruction("no standalone Rust source for " + name)

    def versions(self, step, connection, pad):
        """A version-returning write: each row its pair, old (absent for a
        row an upsert inserted) then new, read through the Rust terminal."""
        name, d, i = step["op"], step["data"], step["inputs"]
        key = {"entity.change": "model", "entity.changes": "entity"}.get(name)
        registration = self.registration(i[key] if key else i["models"][0])
        entity = self.entity_type(registration)
        label = literal(registration)
        pair = (
            f"{REPLAY}::observe::tuple(vec![{REPLAY}::observe::model::<{entity}>("
            f"&change.old, {label})?, {REPLAY}::observe::model::<{entity}>("
            f"&change.new, {label})?])"
        )
        if name == "entity.change":
            return [
                (
                    f"{pad}    let change = pgorm::Update::one({self.use(i['model'])})?"
                    f".exec_returning_change(&{connection}).await?;"
                ),
                f"{pad}    Ok({REPLAY}::observe::collected(vec![{pair}]))",
            ]
        if name == "entity.changes":
            body = [
                f"{pad}    let mut update = pgorm::Update::many(<{entity} as Default>::default());"
            ]
            for column, value in zip(d["columns"], i["values"], strict=True):
                body.append(
                    f"{pad}    let column = {self.entity_column(registration, column)};"
                )
                body.append(
                    f"{pad}    update = update.col_expr(column, pgorm::Column::save_as("
                    f"&column, {Q}::Expr::val({self.use(value)})));"
                )
            body.extend(
                [
                    (
                        f"{pad}    let changes = pgorm::QueryFilter::filter(update, "
                        f"{self.predicate(i['predicate'])})"
                        f".exec_returning_changes(&{connection}).await?;"
                    ),
                    f"{pad}    let mut rows = Vec::new();",
                    f"{pad}    for change in changes {{",
                    f"{pad}        rows.push({pair});",
                    f"{pad}    }}",
                    f"{pad}    Ok({REPLAY}::observe::collected(rows))",
                ]
            )
            return body
        target = (
            f"{Q}::OnConflict::column({Q}::Name::runtime({literal(d['conflict'][0])}))"
        )
        for column in d["conflict"][1:]:
            target += f".and_column({Q}::Name::runtime({literal(column)}))"
        if d["update"]:
            action = target + "".join(
                f".update_column({Q}::Name::runtime({literal(column)}))"
                for column in d["update"]
            )
            action = f"{Q}::OnConflict::from({action})"
        else:
            action = target + ".do_nothing()"
        models = ", ".join(self.use(model) for model in i["models"])
        if d["rows"] == "one":
            if len(i["models"]) != 1:
                raise UnsupportedInstruction("an upsert of one row takes one model")
            insert = (
                f"pgorm::Insert::one({models}).on_conflict({action})"
                f".exec_returning_upsert(&{connection}).await?.into_iter().collect::<Vec<_>>()"
            )
        else:
            insert = (
                f"pgorm::Insert::many(vec![{models}]).on_conflict({action})"
                f".exec_returning_upserts(&{connection}).await?"
            )
        return [
            f"{pad}    let written = {insert};",
            f"{pad}    let mut rows = Vec::new();",
            f"{pad}    for row in written {{",
            f"{pad}        rows.push(match row {{",
            (
                f"{pad}            pgorm::Upserted::Inserted(model) => {REPLAY}::observe::tuple(vec!["
                f"{REPLAY}::observe::absent(), {REPLAY}::observe::model::<{entity}>(&model, {label})?]),"
            ),
            f"{pad}            pgorm::Upserted::Updated(change) => {pair},",
            f"{pad}        }});",
            f"{pad}    }}",
            f"{pad}    Ok({REPLAY}::observe::collected(rows))",
        ]

    def record(self, step, body, lines, pad, *, rows=False):
        rendered = ", ".join(literal(path) for path in self.paths(step))
        lines.append(f"{pad}let outcome: Result<_, Error> = async {{")
        lines.extend(body)
        lines.append(f"{pad}}}")
        lines.append(f"{pad}.await;")
        identity = f"{literal(step['id'])}, {literal(step['op'])}"
        # Rows are kept only where a later result reference reads them back.
        binding = f"{pad}let r_{step['id']} = " if rows else f"{pad}"
        lines.append(binding + "match outcome {")
        if rows:
            lines.append(
                f"{pad}    Ok((observation, rows)) => {{ report.observed("
                f"{identity}, &[{rendered}], observation); rows }}"
            )
            lines.append(
                f"{pad}    Err(error) => {{ report.failed({identity}, &[], "
                "&harness.observe(&error)); Vec::new() }"
            )
            lines.append(f"{pad}}};")
        else:
            lines.append(
                f"{pad}    Ok(observation) => report.observed({identity}, "
                f"&[{rendered}], observation),"
            )
            lines.append(
                f"{pad}    Err(error) => report.failed({identity}, &[], "
                "&harness.observe(&error)),"
            )
            lines.append(f"{pad}}}")

    def paths(self, step):
        name, d, i = step["op"], step["data"], step["inputs"]
        if name == "fetch":
            return self.fetch_paths(i["query"], d["mode"])
        if name == "execute":
            return ["pgorm::Connection::execute_raw"]
        if name == "stream":
            return ["pgorm::Connection::query_raw", "pgorm_driver::RowStream"]
        if name == "active.write":
            return ["pgorm::ActiveModel::" + d["method"]]
        if name == "entity.change":
            return ["pgorm::UpdateOne::exec_returning_change"]
        if name == "entity.changes":
            return ["pgorm::UpdateMany::exec_returning_changes"]
        if name == "entity.upsert":
            terminal = "upsert" if d["rows"] == "one" else "upserts"
            return ["pgorm::Insert::exec_returning_" + terminal]
        if name == "inspect":
            if self.types[i["query"]] in ("pipeline", "sources"):
                return [f"{PL}::Pipeline::into_sql"]
            return ["pgorm_query::QueryStatementBuilder"]
        if name == "begin":
            return [
                "pgorm::DatabaseConnection::transaction_with"
                if step["scope"] == "root"
                else "pgorm::Transact::transaction"
            ]
        if name in ("commit", "rollback"):
            return ["pgorm::Transact::transaction"]
        raise UnsupportedInstruction("no standalone Rust source for " + name)

    def transaction_mode(self, data):
        mode, isolation = data["mode"], data["isolation"]
        if mode == "default":
            if isolation != "default":
                raise UnsupportedInstruction(
                    "a default transaction mode carries no isolation level"
                )
            return "pgorm::TransactionMode::Default"
        level = (
            "None"
            if isolation == "default"
            else f"Some(pgorm::IsolationLevel::{ISOLATION[isolation]})"
        )
        variant = "ReadWrite" if mode == "read_write" else "ReadOnly"
        return f"pgorm::TransactionMode::{variant} {{ isolation: {level} }}"

    def begin(self, step, lines, indent, remaining):
        """A transaction scope: its steps run inside the closure pgorm opens
        the transaction around, and the step that closes it is what the
        closure returns — `Ok` to commit, `Err` to roll back."""
        pad = indent
        inner = pad + "    "
        child = self.connection(step["data"]["child"])
        identity = f"{literal(step['id'])}, {literal(step['op'])}"
        paths = ", ".join(literal(path) for path in self.paths(step))
        scope_name = literal(step["data"]["child"])
        begun, opened, rolled = (
            f"{name}_{step['id']}" for name in ("begun", "opened", "rolled_back")
        )
        if step["scope"] == "root":
            mode = self.transaction_mode(step["data"])
            call = (
                f"pgorm::DatabaseConnection::transaction_with(&mut "
                f"{self.connection(step['scope'])}, {mode}, async |{child}| {{"
            )
        else:
            call = (
                f"pgorm::Transact::transaction(&mut *"
                f"{self.connection(step['scope'])}, async |{child}| {{"
            )
        lines.append(f"{pad}{{")
        # Both outcomes of opening the scope sit here, ahead of its steps: one
        # is reported as the closure starts, the other once the call returns
        # without having run it.
        lines.append(
            f"{inner}let {begun} = |report: &mut Report, error: Option<pgorm::Error>| "
            "match error {"
        )
        lines.append(
            f"{inner}    None => report.observed({identity}, &[{paths}], "
            f"{REPLAY}::observe::transaction({scope_name})),"
        )
        lines.append(
            f"{inner}    Some(error) => report.failed({identity}, &[], "
            "&harness.observe(&Error::from(error))),"
        )
        lines.append(f"{inner}}};")
        lines.append(f"{inner}let mut {opened} = false;")
        # The closure's `?` fails the whole program, as it would outside a
        # scope; a scope that ends without committing returns an error of its
        # own, which `rolled` tells apart from those.
        body = []
        consumed, closing = self.steps(remaining, body, inner + "    ")
        commits = closing is not None and closing["op"] == "commit"
        if not commits:
            lines.append(f"{inner}let mut {rolled} = false;")
        lines.append(f"{inner}let ended = {call}")
        lines.append(f"{inner}    {opened} = true;")
        lines.append(f"{inner}    {begun}(report, None);")
        lines.extend(body)
        if commits:
            lines.append(f"{inner}    Ok::<(), Error>(())")
        else:
            lines.append(f"{inner}    {rolled} = true;")
            lines.append(
                f"{inner}    Err::<(), Error>(Error::from(pgorm::Error::Custom("
                '"the scope rolls back".to_owned())))'
            )
        lines.append(f"{inner}}})")
        lines.append(f"{inner}.await;")
        lines.append(f"{inner}match ended {{")
        lines.append(
            f"{inner}    Err(pgorm::TransactionError::Connection(error)) if !{opened} => "
            f"{begun}(report, Some(error)),"
        )
        guard = "" if commits else f" if !{rolled}"
        lines.append(
            f"{inner}    Err(pgorm::TransactionError::Transaction(error)){guard} => "
            "return Err(error),"
        )
        lines.append(
            f"{inner}    Err(pgorm::TransactionError::RollbackFailed {{ cause, .. }}){guard} => "
            "return Err(cause),"
        )
        if closing is not None:
            closed = f"{literal(closing['id'])}, {literal(closing['op'])}"
            rendered = ", ".join(literal(path) for path in self.paths(closing))
            observed = (
                f"report.observed({closed}, &[{rendered}], {REPLAY}::observe::unit())"
            )
            if commits:
                lines.append(f"{inner}    Ok(()) => {observed},")
                lines.append(
                    f"{inner}    Err(pgorm::TransactionError::Connection(error)) => "
                    f"report.failed({closed}, &[], &harness.observe(&Error::from(error))),"
                )
            else:
                lines.append(
                    f"{inner}    Err(pgorm::TransactionError::Transaction(_)) => {observed},"
                )
                lines.append(
                    f"{inner}    Err(pgorm::TransactionError::RollbackFailed {{ rollback, .. }}) => "
                    f"report.failed({closed}, &[], &harness.observe(&Error::from(rollback))),"
                )
                lines.append(f"{inner}    _ => {{}}")
        else:
            lines.append(f"{inner}    _ => {{}}")
        lines.append(f"{inner}}}")
        lines.append(f"{pad}}}")
        return consumed

    def steps(self, remaining, lines, indent):
        """Emit steps until the scope opened by the caller closes: how many it
        consumed, and the commit or rollback that closed it."""
        consumed = 0
        while remaining:
            step = remaining[0]
            consumed += 1
            remaining = remaining[1:]
            if step["op"] in ("commit", "rollback"):
                return consumed, step
            if step["op"] == "begin":
                nested = self.begin(step, lines, indent, remaining)
                consumed += nested
                remaining = remaining[nested:]
                continue
            outer, self.current = self.current, set()
            body = []
            self.effect(step, body, indent)
            used, self.current = self.current, outer | self.current
            self.declare(used, lines, indent)
            lines.extend(body)
        return consumed, None
