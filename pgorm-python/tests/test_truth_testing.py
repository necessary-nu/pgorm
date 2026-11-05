"""Truth testing across every class the installed package defines."""

import importlib
import pkgutil
import unittest

import pgorm as p
from pgorm import _native, pipeline as pl, schema as s
from pgorm.schema import changes
from pgorm._registered import disjoints as disjoint_views, entities as views, graphs as graph_views, versions as version_views
from pgorm.codegen.types import FieldType
from pgorm.models import (
    Column, Model, ModelRecord, Relation, cursors, disjoint_query, graphs, model, one_of, queries,
    record, writes,
)
from pgorm.runtime import PoolStatus

# Classes whose instances keep Python's own truth, grouped by why that is safe.
EXEMPT = {
    # Values: data compared with ==, which answers a Python bool, so `and`,
    # `or` and `if` over them stay ordinary Python. A multirange is false when
    # it holds no range, as any container is.
    p.Value, p.Range, p.Multirange, p.CreatedRange, p.CreatedMultirange, p.Interval,
    p.TypeName, p.Identifier,
    # Rows, models and their write state, and what a write answers: data read
    # after SQL ran. A record or a model is a mapping, false with no fields.
    p.Record, p.Field, p.EntityModel, p.ActiveModel, p.ActiveValue, p.ActiveState,
    p.Change, p.Upserted, ModelRecord, PoolStatus, views.ModelView, views.ActiveView,
    version_views.Change, version_views.Inserted, version_views.Updated,
    # Pools, connections, transactions and the private native handles under
    # them and under the streams: live resources, not SQL, and true as any
    # open resource is. The streams themselves refuse, since a stream's truth
    # would say nothing of whether rows remain.
    p.Pool, p.Connection, p.Transaction,
    _native._Pool, _native._Connection, _native._Transaction, _native._Stream,
    _native._CopyIn, _native._CopyOut, _native._Listener, _native._Disjoint,
    # Registrations and declarations: they name what a query reads and are
    # true as any declared object is; the queries they start refuse.
    p.Entity, p.EntityGraph, Model, Column, Relation, pl.SourceSelection, pl.Binder,
    model.UniqueKey, model._DeclaredKey,
    views.EntityView, graph_views.GraphView, FieldType,
    p.Disjoint, p.DisjointEntity, disjoint_query._Disjoint,
    p.OneOf, p.OneOfEntity, one_of._OneOf, one_of._Reader, _native._OneOf,
    disjoint_views.DisjointView, disjoint_views.WritableDisjointView, disjoint_views._Decoded,
    # The private state a model query, graph, cursor or paginator holds and
    # builds its statements from: data the application never handles, held by
    # a query that refuses.
    queries.Recipe, queries.Joined, graphs.State, graphs.Table, graphs.Edge,
    cursors.Keyed, cursors.Position, cursors.Pageable, record.Source,
}

# Refusing classes only a registered application or a live server can build:
# registered_entities.py, registered_graphs.py, registered_sources.py,
# registered_schema.py, test_listen.py, test_streams.py and test_copy.py
# truth-test their instances.
BUILT_ELSEWHERE = {
    p.EntityColumn, p.EntityQuery, p.EntityUpdate, p.EntityUpdateMany, p.EntityInsert,
    p.EntityInsertMany, p.EntityGraphQuery, p.EntityGraphCursor, pl.SelectedSources, s.EntitySchema,
    views.QueryView, graph_views.GraphQueryView, graph_views.GraphCursorView,
    version_views.UpdateView, version_views.UpdateManyView, version_views.InsertView,
    version_views.InsertManyView, p.Notification, p.Listener, p.RowStream,
    p.CopyOutStream, p.CopyRowStream,
}

# Bases the model writes share and no program builds: every write refuses
# through the __bool__ they give it, and each is sampled through a write.
BASES = {writes._Write, writes._Insert, writes._Guarded, changes._Part}


def defined_classes():
    """Every class a module of the package defines or exports."""
    found = set()
    for module in pkgutil.walk_packages(p.__path__, "pgorm."):
        if module.name.endswith("__main__"):
            continue
        for value in vars(importlib.import_module(module.name)).values():
            if isinstance(value, type) and value.__module__.split(".")[0] == "pgorm":
                found.add(value)
    # An exception is true as every Python exception is; it carries no SQL.
    return {kind for kind in found if not issubclass(kind, BaseException)}


def statement_samples():
    table, source = p.Table("account"), p.Table("staged")
    query = p.select(p.col("id")).from_(table)
    pending = p.merge(table, source, table.col("id") == source.col("id"))
    rows = p.FrameType.Rows
    ranked = p.window_function("rank")
    return [
        table, query, p.insert(table), p.update(table), p.delete(table), p.RawSQL("SELECT 1"),
        query.inspect(), pending, pending.when_matched(p.MergeAction.delete()),
        p.TableFunction(p.call("generate_series", 1, 3)),
        p.TableFunction(p.call("generate_series", 1, 3)).alias("series"),
        p.GroupingElement.empty(), p.Join.Inner, p.SampleMethod.Bernoulli,
        p.LockStrength.Update, p.LockWait.Nowait,
        p.With("recent", query), p.ReturningRow.Old, p.SearchOrder.Depth,
        p.Search(p.SearchOrder.Breadth, p.col("id"), "ord"), p.Cycle(p.col("id"), "looped", "path"),
        p.Conflict.on_constraint("account_pkey"),
        p.Conflict.do_nothing(), p.Conflict.on("id"), p.Conflict.on("id").update("name"),
        p.MergeAction.delete(), p.Overriding.SystemValue,
        p.MergeAction.update("name", "x"), p.MergeAction.insert("id", 1),
        rows, p.FrameExclusion.Ties, rows.preceding(1), rows.current_row(), rows.following(1),
        rows.preceding(1).and_current_row(), p.Window(), p.ExtendedWindow("w"),
        ranked, ranked.over(p.Window()),
        p.CopyFrom(table, "text"), p.CopyTo(table, "csv"),
    ]


def expression_samples():
    expression = p.col("id") > 1
    return [
        expression, p.Condition.all(expression), expression.as_("big"), expression.asc(),
        p.LikePattern("a%"), p.Direction.Asc, p.Nulls.First, p.format_json(p.col("doc")),
        p.JsonKind.Object, p.JsonExistsBehavior.Error, p.JsonValueBehavior.Null,
        p.JsonQueryBehavior.Null, p.JsonDefault("x"), p.JsonTableBehavior.Empty,
        p.JsonTableColumn.ordinality("n"), p.any_(p.col("ids")),
        p.case_when(expression, 1), p.case_of(p.col("id")), p.case_of(p.col("id")).when(1, 2),
        _native._DeclaredColumn(p.col("id")),
    ]


def other_samples():
    table = p.Table("items")
    model = Model("items", {"id": Column("i32", primary_key=True)})
    return [
        s.DataType("integer"), s.ColumnDef("id", "integer"), s.create_table(table),
        s.create_index(table, "id"), s.drop_table(table), s.create_domain("d", "integer"),
        s.alter_table(table), s.alter_table(table).drop_column("id"), s.create_type("t"),
        s.alter_type("t"), s.alter_type("t").drop_attribute("a"),
        s.alter_type("t").rename_attribute("a", "b"), s.alter_domain("d"),
        s.create_sequence("q"), s.alter_sequence("q"), s.alter_sequence("q").restart(),
        s.create_view(table, p.select(p.col("id")).from_(table)),
        s.create_recursive_view(table, "id", p.select(p.col("id")).from_(table)),
        s.create_materialized_view(table, p.select(p.col("id")).from_(table)),
        s.create_table_as(table, p.select(p.col("id")).from_(table)),
        pl.from_(table), pl.col("items", "id"), pl.from_(table).group(pl.col("items", "id")),
        pl.Over(), pl.source(table),
        model.col("id"), model.find(), model.delete(), model.insert({"id": 1}), model.insert_many([]),
        model.update({"id": 2}), model.insert({"id": 1}).returning("id"),
        model.graph(), model.find().cursor("id"), model.find().paginate(2),
        *disjoint_samples(),
    ]


def disjoint_samples():
    kind = p.TypeName("item_kind", schema="app")
    base = Model(
        "items",
        {"id": Column("i32", primary_key=True), "kind": Column(kind, values=["plain"])},
        schema="app",
    )
    items = p.Disjoint("item", base, {"plain": None}, schema_name="app")
    query = items.find()
    change = s.add_disjoint_kind(items, "plain")
    return [
        query, query.cursor("id"), query.paginate(2), items.insert({}, "plain"),
        change, change.steps[0], s.DisjointGap("plain", p.Table("items")),
    ]


class TruthTesting(unittest.TestCase):
    # [spec:pgorm:req:python.truth-testing+3/test]
    def test_every_query_class_refuses_truth_testing(self):
        samples = statement_samples() + expression_samples() + other_samples()
        for sample in samples:
            with self.subTest(sample=sample), self.assertRaises(p.ConstructionError):
                bool(sample)
        # A class the package adds must refuse and be sampled above, or be
        # listed as exempt with its reason, so none evaluates as true unnoticed.
        defined = defined_classes()
        sampled = {type(sample) for sample in samples}
        self.assertEqual(defined - sampled - EXEMPT - BUILT_ELSEWHERE - BASES, set())
        for base in BASES:
            with self.subTest(base=base):
                self.assertIn(base, defined)
                self.assertTrue(any(issubclass(kind, base) for kind in sampled))
        self.assertEqual((EXEMPT | BUILT_ELSEWHERE) - defined, set())
        self.assertEqual(sampled & EXEMPT, set())
        for kind in BUILT_ELSEWHERE:
            with self.subTest(kind=kind):
                self.assertIn("__bool__", vars(kind))

    # [spec:pgorm:req:python.truth-testing+3/test]
    def test_exempt_values_keep_python_truth(self):
        self.assertTrue(p.Value(0))
        self.assertTrue(p.Identifier("id"))
        self.assertFalse(p.Multirange([]))
        self.assertTrue(p.Multirange([p.Range(1, 2)]))
        self.assertTrue(p.ConstructionError("refused"))


if __name__ == "__main__":
    unittest.main()
