"""The installed package offers the surface both bindings share, spelled as Python spells it."""

import importlib
import inspect
import json
import keyword
from pathlib import Path
import unittest

import pgorm as p
import pgorm.pipeline as pl

SURFACE = json.loads(
    (Path(__file__).resolve().parents[2] / "docs" / "spec" / "bindings-surface.json").read_text()
)
SIDE = SURFACE["python"]
OPERATORS = SIDE["operators"]
DESCRIPTIONS = SURFACE["descriptions"]


def class_members(cls):
    """A class's public methods and constants, and the operators that spell shared methods."""
    found = set()
    for name in dir(cls):
        value = inspect.getattr_static(cls, name)
        if name.startswith("_"):
            if name in OPERATORS.values() and callable(value):
                found.add(name)
            continue
        if any(name in vars(base) for base in cls.__mro__ if base.__module__ == "builtins"):
            continue
        method = isinstance(value, (staticmethod, classmethod)) or (callable(value) and not inspect.isclass(value))
        if method or isinstance(value, cls):
            found.add(name)
    return found


def live_surface():
    """Every name each module exports, and every member of each exported class."""
    names, members = set(), {}
    for neutral, module_name in SIDE["modules"].items():
        module = importlib.import_module(module_name)
        for name in module.__all__:
            names.add((neutral, name))
            value = getattr(module, name)
            if inspect.isclass(value):
                members[(neutral, name)] = class_members(value)
    return names, members


def renamed(path):
    entry = SIDE["undecided"].get(path, {})
    return SIDE["spellings"].get(path, entry.get("spelled"))


def spell(path):
    """A neutral module-level path as Python spells its name."""
    name = path.split(".")[1]
    return renamed(path) or (name + "_" if keyword.iskeyword(name) else name)


def spell_member(owner, member):
    """A neutral member as Python spells it on the class at `owner`."""
    spelled = renamed(f"{owner}.{member}")
    if spelled:
        return spelled
    if member in SIDE["operator_classes"].get(owner, ()):
        return OPERATORS[member]
    return member + "_" if keyword.iskeyword(member) else member


def declared():
    """The shared names Python must offer, and the shared names it lacks by a plan node."""
    expected, gaps = {}, {}
    for module, entries in SURFACE["modules"].items():
        for name, methods in entries.items():
            path = f"{module}.{name}"
            key = (module, spell(path))
            if path in SIDE["gaps"]:
                gaps[key] = path
                continue
            expected[key] = path
            for member in methods or ():
                member_path = f"{path}.{member}"
                member_key = key + (spell_member(path, member),)
                (gaps if member_path in SIDE["gaps"] else expected)[member_key] = member_path
    return expected, gaps


def own_names():
    """Names Python offers alone, for a language reason or awaiting the operator, as keys."""
    own = {}
    for path in [*SIDE["only"], *(p for p, e in SIDE["undecided"].items() if "spelled" not in e)]:
        module, *rest = path.split(".")
        own[(module, *rest)] = path
    return own


def declared_paths(shape, path=""):
    """Every key path a declared description holds, nested keys included."""
    if isinstance(shape, list):
        yield from declared_paths(shape[0], path + "[]")
    elif isinstance(shape, dict):
        for key, inner in shape.items():
            yield f"{path}.{key}"
            yield from declared_paths(inner, f"{path}.{key}")


def differences(live, shape, path, seen):
    """How a live description differs from its declared shape, each difference
    at its neutral key path; every declared path it reaches joins `seen`.
    Python spells a neutral key as written (python.naming)."""
    if shape is None:
        return []
    if isinstance(shape, list):
        if not isinstance(live, list):
            return [f"{path}: is no list"]
        return [found for item in live for found in differences(item, shape[0], path + "[]", seen)]
    if not isinstance(live, dict):
        return [f"{path}: is no mapping"]
    if set(shape) == {"*"}:
        seen.update(f"{path}.*" for _ in live)
        return [found for item in live.values() for found in differences(item, shape["*"], f"{path}.*", seen)]
    found = [f"{path}.{key}: undeclared" for key in live if key not in shape]
    for key, inner in shape.items():
        if key not in live:
            found.append(f"{path}.{key}: missing")
        else:
            seen.add(f"{path}.{key}")
            found.extend(differences(live[key], inner, f"{path}.{key}", seen))
    return found


class SurfaceTests(unittest.TestCase):
    maxDiff = None

    """bindings.surface: what the package offers is the shared surface, Python's own and its gaps."""

    def setUp(self):
        self.names, self.members = live_surface()
        self.live = self.names | {key + (m,) for key, found in self.members.items() for m in found}
        self.expected, self.gaps = declared()
        self.own = own_names()

    def checked_classes(self):
        """The shared classes the surface lists methods for, whose members are compared."""
        return {
            key for key, path in self.expected.items()
            if len(key) == 2 and SURFACE["modules"][key[0]][path.split(".")[1]] is not None
        }

    # [spec:pgorm:req:bindings.surface+2/test]
    # [spec:pgorm:req:python.naming+3/test]
    def test_every_shared_name_is_offered(self):
        missing = sorted(self.expected[key] for key in self.expected if key not in self.live)
        self.assertEqual(missing, [], "shared names the package lacks: add them, or list each as a gap with its plan node")

    # [spec:pgorm:req:bindings.surface+2/test]
    def test_offers_only_declared_names(self):
        checked = self.checked_classes()
        extra = []
        for key in sorted(self.live):
            if key in self.expected or key in self.own:
                continue
            if len(key) == 3:
                if key[:2] not in checked or key[2] in SIDE["protocols"]:
                    continue
            extra.append(".".join(key))
        self.assertEqual(extra, [], "undeclared public names: share them in bindings-surface.json, or list them as Python's own with a reason")

    # [spec:pgorm:req:bindings.surface+2/test]
    def test_landed_gaps_leave_the_list(self):
        closed = sorted(path for key, path in self.gaps.items() if key in self.live and key not in self.expected)
        self.assertEqual(closed, [], "names offered now that the list still calls gaps: remove them from python.gaps")

    # [spec:pgorm:req:bindings.surface+2/test]
    def test_every_declared_own_name_exists(self):
        stale = sorted(path for key, path in self.own.items() if key not in self.live)
        self.assertEqual(stale, [], "names python.only or python.undecided declares that the package no longer offers")

    # [spec:pgorm:req:bindings.surface+2/test]
    def test_entries_name_a_node_or_question(self):
        shared = {
            f"{module}.{name}{suffix}"
            for module, entries in SURFACE["modules"].items()
            for name, methods in entries.items()
            for suffix in ["", *(f".{member}" for member in methods or ())]
        }
        for path, node in SIDE["gaps"].items():
            with self.subTest(path=path):
                self.assertIn(path, shared, "a gap must be a shared name")
                self.assertRegex(node, r"^[a-z0-9][a-z0-9.\-]*$")
        for path, entry in SIDE["undecided"].items():
            with self.subTest(path=path):
                self.assertIn(entry["question"], SURFACE["questions"])
        for path, reason in {**SIDE["only"], **SIDE["protocols"]}.items():
            with self.subTest(path=path):
                self.assertTrue(reason.strip())


class DescriptionTests(unittest.TestCase):
    """bindings.surface: each describe() reports exactly the keys the surface declares for it."""

    maxDiff = None

    def assert_declared(self, owner, descriptions):
        shape, seen = DESCRIPTIONS[owner], set()
        found = sorted({f for d in descriptions for f in differences(d, shape, owner, seen)})
        self.assertEqual(found, [], "describe() keys differ from bindings-surface.json's descriptions")
        unreached = sorted({owner + path for path in declared_paths(shape)} - seen)
        self.assertEqual(unreached, [], "declared keys no live description reached")

    # [spec:pgorm:req:bindings.surface+2/test]
    # [spec:pgorm:req:bindings.models+5/test]
    # [spec:pgorm:req:bindings.model-unique-keys+1/test]
    def test_model_description_has_the_declared_keys(self):
        model = p.Model(
            "accounts",
            {
                "id": p.Column("i64", primary_key=True, generated="always"),
                "mood": p.Column(p.TypeName("mood", schema="app"), nullable=True, values=["calm"]),
            },
            schema="app",
            unique_keys=[{"fields": ["mood", "id"], "name": "accounts_mood_id"}],
        )
        self.assert_declared("pgorm.Model", [model.describe(), model.as_("a").describe()])

    # [spec:pgorm:req:bindings.surface+2/test]
    # [spec:pgorm:req:bindings.disjoint+1/test]
    def test_disjoint_description_has_the_declared_keys(self):
        kind = p.TypeName("item_kind", schema="app")
        base = p.Model(
            "items",
            {"id": p.Column("i32", primary_key=True), "kind": p.Column(kind, values=["plain", "boxed"])},
            schema="app",
        )
        boxed = p.Model("boxes", {"id": p.Column("i32", primary_key=True)}, schema="app")
        items = p.Disjoint("item", base, {"plain": None, "boxed": boxed}, schema_name="app")
        self.assert_declared("pgorm.Disjoint", [items.describe()])

    # [spec:pgorm:req:bindings.surface+2/test]
    # [spec:pgorm:req:bindings.disjoint+1/test]
    def test_disjoint_entity_description_has_the_declared_keys(self):
        names = p.disjoint_entities()
        if not names:
            self.skipTest("the module registers no disjoint entity; checks/entities.py runs this against an application's")
        self.assert_declared("pgorm.DisjointEntity", [p.disjoint_entity(name).describe() for name in names])

    # [spec:pgorm:req:bindings.surface+2/test]
    # [spec:pgorm:req:bindings.one-of+1/test]
    def test_one_of_describes_the_declared_keys(self):
        target = p.Model("posts", {"id": p.Column("i32", primary_key=True)}, schema="app")
        model = p.Model(
            "comments",
            {"id": p.Column("i32", primary_key=True), "post": p.Column("i32", nullable=True)},
            schema="app",
        )
        reference = p.OneOf("commentable", model, {"post": ("post", target)}, optional=True)
        self.assert_declared("pgorm.OneOf", [reference.describe()])

    # [spec:pgorm:req:bindings.surface+2/test]
    # [spec:pgorm:req:bindings.one-of+1/test]
    def test_one_of_entity_describes_the_declared_keys(self):
        names = p.one_of_entities()
        if not names:
            self.skipTest("the module registers no one-of entity; checks/entities.py runs this against an application's")
        self.assert_declared("pgorm.OneOfEntity", [p.one_of_entity(name).describe() for name in names])

    # [spec:pgorm:req:bindings.surface+2/test]
    # [spec:pgorm:req:bindings.entities+5/test]
    def test_entity_description_has_the_declared_keys(self):
        names = p.entities()
        if not names:
            self.skipTest("the module registers no entity; checks/entities.py runs this against an application's")
        self.assert_declared("pgorm.Entity", [p.entity(name).describe() for name in names])

    # [spec:pgorm:req:bindings.surface+2/test]
    # [spec:pgorm:req:bindings.entity-graphs+3/test]
    def test_graph_description_has_the_declared_keys(self):
        names = p.graphs()
        if not names:
            self.skipTest("the module registers no graph; checks/entities.py runs this against an application's")
        self.assert_declared("pgorm.EntityGraph", [p.graph(name).describe() for name in names])

    # [spec:pgorm:req:bindings.surface+2/test]
    # [spec:pgorm:req:bindings.pipeline-sources+3/test]
    def test_source_tuple_description_has_the_declared_keys(self):
        names = pl.source_tuples()
        if not names:
            self.skipTest("the module registers no source tuple; checks/entities.py runs this against an application's")
        self.assert_declared("pipeline.SourceSelection", [pl.sources(name).describe() for name in names])


if __name__ == "__main__":
    unittest.main()
