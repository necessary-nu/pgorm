"""Lazy graph resolution follows effect dependencies and native binder scopes."""

from . import (
    catalog,
    dispatch_expr,
    dispatch_models,
    dispatch_pipeline,
    dispatch_query,
    dispatch_schema,
    observations,
)
from .program import CALLBACK_INPUTS


class Resolution:
    def __init__(self, program, p):
        self.program = program
        self.p = p
        self.nodes = {node["id"]: node for node in program["nodes"]}
        self.cache = {}
        self.results = {}
        self.trace = []

    def result(self, name, data):
        value = self.results[data["step"]][data["row"]]
        if name == "entity.result":
            if "source" in data:
                value = value[data["source"]]
            if not isinstance(value, self.p.EntityModel):
                raise self.p.DecodeError(
                    "result reference requires a registered EntityModel"
                )
            return value, ["pgorm::ModelTrait"]
        value = value.tagged(data["column"])
        if value.snapshot()["type"] != data["type"]:
            raise self.p.DecodeError(
                "result reference type differs from the declared value tag"
            )
        return value, ["pgorm_driver::Row::try_get"]

    # [spec:pgorm:req:generative.execution]
    def get(self, reference, *, scope="root", binder=None, local=None):
        node = self.nodes[reference]
        if node["scope"] not in ("root", scope):
            raise RuntimeError("executor attempted to escape a binder scope")
        cache = self.cache if node["scope"] == "root" else local
        if cache is None:
            raise RuntimeError("private expression has no owning callback cache")
        if reference in cache:
            return cache[reference]
        event = {
            "id": reference,
            "operation": node["op"],
            "scope": node["scope"],
            "status": "attempted",
        }
        self.trace.append(event)
        try:
            value, paths = self.dispatch(node, scope, binder, local)
            if not paths:
                raise RuntimeError("active dispatch did not identify a native path")
            event.update(
                status="constructed",
                native_paths=paths,
                observation=observations.construction(value, self.p),
            )
            cache[reference] = value
            return value
        except Exception as error:
            event.update(status="error", observation=observations.error(error))
            raise

    def dispatch(self, node, scope, binder, local):
        name, data = node["op"], node["data"]
        if name in CALLBACK_INPUTS:
            return dispatch_pipeline.stage(node, self.get, self.p)
        if name in ("result.value", "entity.result"):
            return self.result(name, data)

        def resolve(reference):
            return self.get(reference, scope=scope, binder=binder, local=local)

        inputs = {
            key: [resolve(ref) for ref in value]
            if isinstance(value, list)
            else resolve(value)
            for key, value in node["inputs"].items()
        }
        family = catalog.OPERATIONS[name].family
        if name.startswith("pipeline."):
            return dispatch_pipeline.dispatch(name, inputs, data, self.p, binder)
        if name.startswith("schema."):
            return dispatch_schema.dispatch(name, inputs, data, self.p)
        if name == "entity.page":
            return dispatch_query.dispatch(name, inputs, data, self.p)
        if family in ("models", "entities", "graph"):
            return dispatch_models.dispatch(
                name, inputs, data, self.p, self.program["fixture"]
            )
        if name.startswith(("select", "insert", "update", "delete", "write.", "raw.")):
            return dispatch_query.dispatch(name, inputs, data, self.p)
        return dispatch_expr.dispatch(name, inputs, data, self.p)
