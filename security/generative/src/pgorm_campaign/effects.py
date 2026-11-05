"""Ordered public asynchronous terminals, transaction scopes and stream cleanup."""

import asyncio

from . import observations


async def fetch(query, mode, connection, p):
    from pgorm.models.queries import ModelQuery, ModelRows
    from pgorm.pipeline import Pipeline, SelectedSources

    for category, prefix in (
        (p.EntityQuery, "pgorm::Select"),
        (p.EntityGraphQuery, "pgorm::SelectGraph"),
        (p.EntityGraphCursor, "pgorm::Cursor"),
        (Pipeline, "pgorm::pipeline::Pipeline"),
        (SelectedSources, "pgorm::pipeline::SelectedSources"),
        (ModelQuery, "pgorm::ConnectionTrait::query_raw"),
        (ModelRows, "pgorm::ConnectionTrait::query_raw"),
    ):
        if isinstance(query, category):
            if not hasattr(query, mode):
                raise p.UnsupportedCapabilityError(
                    "query has no public " + mode + " terminal"
                )
            value = await getattr(query, mode)(connection)
            rust = "one_opt" if mode == "optional" else mode
            path = (
                prefix if category in (ModelQuery, ModelRows) else prefix + "::" + rust
            )
            break
    else:
        value = await getattr(connection, "query" if mode == "all" else mode)(query)
        path = "pgorm::ConnectionTrait::query_raw"
    rows = value if mode == "all" else ([] if value is None else [value])
    return rows, [path]


async def stream(query, data, connection):
    records = []
    cancelled = False
    complete = False
    async with await connection.stream(query) as stream:
        for _ in range(data["take"]):
            try:
                records.append(await anext(stream))
            except StopAsyncIteration:
                complete = True
                break
        if data["cancel"] and not complete:
            pending = asyncio.create_task(anext(stream))
            try:
                await asyncio.sleep(0)
                pending.cancel()
                try:
                    await pending
                except asyncio.CancelledError:
                    cancelled = True
                except StopAsyncIteration:
                    complete = True
            finally:
                if not pending.done():
                    pending.cancel()
                    await asyncio.gather(pending, return_exceptions=True)
    return records, {
        "complete": complete,
        "cancelled": cancelled,
        "closed": stream.closed,
    }


def conflict(data, p):
    """The ON CONFLICT action an upsert declares: update the named columns
    from the proposed row, or, naming none, do nothing."""
    target = p.Conflict.on(*data["conflict"])
    return target.update(*data["update"]) if data["update"] else target.do_nothing()


async def versions(name, inputs, data, connection, p):
    """A version-returning write's rows, each the pair (old, new); old is
    `None` for a row an upsert inserted."""
    if name == "entity.change":
        model = inputs["model"]
        entity = p.entity(model.entity_name)
        change = await entity.update(model).returning_change(connection)
        return [(change.old, change.new)], ["pgorm::UpdateOne::exec_returning_change"]
    if name == "entity.changes":
        update = inputs["entity"].update_many()
        for column, value in zip(data["columns"], inputs["values"], strict=True):
            update = update.set(column, value)
        changes = await update.where(inputs["predicate"]).returning_changes(connection)
        return [(change.old, change.new) for change in changes], [
            "pgorm::UpdateMany::exec_returning_changes"
        ]
    models = inputs["models"]
    entity = p.entity(models[0].entity_name)
    if data["rows"] == "one":
        insert = entity.insert(models[0]).on_conflict(conflict(data, p))
        row = await insert.returning_upsert(connection)
        rows, path = ([] if row is None else [row]), "exec_returning_upsert"
    else:
        insert = entity.insert_many(models).on_conflict(conflict(data, p))
        rows, path = (
            await insert.returning_upserts(connection),
            "exec_returning_upserts",
        )
    return [
        (None, row.model)
        if isinstance(row, p.Upserted.Inserted)
        else (row.change.old, row.change.new)
        for row in rows
    ], ["pgorm::Insert::" + path]


class RolledBack(Exception):
    """Raised through a scope's ``async with`` block, so that it rolls back."""


class Effects:
    def __init__(self, resolution, connection, acquire):
        self.resolution = resolution
        self.p = resolution.p
        self.scopes = {"root": connection}
        self.acquire = acquire

    async def run(self, steps, events, scoped=False):
        """Perform ``steps`` in order, recording each in ``events``, until the
        commit or rollback that closes the scope the caller opened: that
        step and its event, or ``None`` for both once the steps run out."""
        while steps:
            step = steps.popleft()
            event = {"id": step["id"], "operation": step["op"], "status": "attempted"}
            events.append(event)
            if step["op"] in ("commit", "rollback"):
                return step, event
            if step["op"] == "begin":
                await self.transaction(step, event, steps, events)
            else:
                try:
                    value, paths = await self.perform(step)
                    if not paths:
                        raise RuntimeError("effect has no active native dispatch path")
                    event.update(
                        status="observed", observation=value, native_paths=paths
                    )
                except self.p.PgormError as error:
                    event.update(status="error", observation=observations.error(error))
            await self.reacquire(scoped)
        return None, None

    async def reacquire(self, scoped):
        if not self.scopes["root"].closed:
            return
        # Stream cancellation can discard the root connection.
        if scoped:
            raise RuntimeError("connection closed with outstanding transactions")
        self.scopes["root"] = await self.acquire()

    async def transaction(self, step, event, steps, events):
        """A transaction scope: the steps up to the one closing it run inside
        the ``async with`` block that opens it, and that step is how the block
        exits, normally to commit and by raising to roll back."""
        d = step["data"]
        parent = self.scopes[step["scope"]]
        if step["scope"] == "root":
            isolation = None if d["isolation"] == "default" else d["isolation"]
            block = parent.transaction(mode=d["mode"], isolation=isolation)
            path = "pgorm::DatabaseConnection::transaction_with"
        else:
            block, path = parent.transaction(), "pgorm::TransactionTrait::transaction"
        closed = event
        try:
            async with block as child:
                self.scopes[d["child"]] = child
                event.update(
                    status="observed",
                    observation={"kind": "transaction", "scope": d["child"]},
                    native_paths=[path],
                )
                closing, closing_event = await self.run(steps, events, True)
                if closing_event is not None:
                    closed = closing_event
                if closing is None or closing["op"] == "rollback":
                    raise RolledBack
        except RolledBack:
            pass
        except self.p.PgormError as error:
            closed.update(status="error", observation=observations.error(error))
            return
        if closed is not event:
            closed.update(
                status="observed",
                observation={"kind": "unit"},
                native_paths=["pgorm::TransactionTrait::transaction"],
            )

    async def perform(self, step):
        name, d = step["op"], step["data"]
        inputs = {
            key: [self.resolution.get(item) for item in ref]
            if isinstance(ref, list)
            else self.resolution.get(ref)
            for key, ref in step["inputs"].items()
        }
        connection = self.scopes[step["scope"]]
        query = inputs.get("query")
        match name:
            case "fetch":
                rows, paths = await fetch(query, d["mode"], connection, self.p)
                self.resolution.results[step["id"]] = rows
                return {
                    "kind": "rows",
                    "rows": [observations.row(row) for row in rows],
                }, paths
            case "execute":
                from pgorm.models import (
                    ModelDelete,
                    ModelInsert,
                    ModelInsertMany,
                    ModelUpdate,
                )

                value = (
                    await query.execute(connection)
                    if isinstance(
                        query, (ModelInsert, ModelInsertMany, ModelUpdate, ModelDelete)
                    )
                    else await connection.execute(query)
                )
                return {"kind": "count", "value": value}, [
                    "pgorm::ConnectionTrait::execute_raw"
                ]
            case "active.write":
                value = await getattr(inputs["model"], d["method"])(connection)
                result = (
                    {"kind": "count", "value": value}
                    if d["method"] == "delete"
                    else {"kind": "rows", "rows": [observations.row(value)]}
                )
                self.resolution.results[step["id"]] = (
                    [] if d["method"] == "delete" else [value]
                )
                return result, ["pgorm::ActiveModelTrait::" + d["method"]]
            case "entity.change" | "entity.changes" | "entity.upsert":
                rows, paths = await versions(name, inputs, d, connection, self.p)
                self.resolution.results[step["id"]] = rows
                return {
                    "kind": "rows",
                    "rows": [observations.row(row) for row in rows],
                }, paths
            case "stream":
                rows, state = await stream(query, d, connection)
                self.resolution.results[step["id"]] = rows
                return {
                    "kind": "rows",
                    "rows": [observations.row(row) for row in rows],
                    "stream": state,
                }, ["pgorm::ConnectionTrait::query_raw", "tokio_postgres::RowStream"]
            case "inspect":
                if not hasattr(query, "inspect"):
                    raise self.p.UnsupportedCapabilityError(
                        "query has no public inspect terminal"
                    )
                from pgorm.pipeline import Pipeline, SelectedSources

                prefix = (
                    "pgorm::pipeline::Pipeline::into_sql"
                    if isinstance(query, (Pipeline, SelectedSources))
                    else "pgorm_query::QueryStatementBuilder"
                )
                return observations.compiled(query.inspect()), [prefix]
        raise RuntimeError("inactive effect dispatch: " + name)
