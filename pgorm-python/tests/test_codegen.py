"""Build-input validation and metadata typing independent of a database."""

import ast
import copy
from pathlib import Path
import tempfile
import unittest

from pgorm.codegen.config import CodegenError, rust_path, rust_string, validate
from pgorm.codegen.emit import attributes, disjoint_code, entity_code, import_block
from pgorm.codegen.source import literal, statement, string
from pgorm.codegen.types import field_type


class CodegenTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        (self.base / "Cargo.toml").write_text('[package]\nname = "my-entities"\n')
        self.config = {
            "schema_version": 1,
            "entity_crate": ".",
            "entities": [
                {"name": "app.Entity", "rust": "thing::Entity", "python": "Thing"}
            ],
        }

    # [spec:pgorm:req:bindings.codegen/test]
    def test_registration_collisions_are_rejected(self):
        for update in (
            {"name": "different"},
            {"rust": "other::Entity"},
            {"name": "other", "rust": "other::Entity", "python": "ThingModel"},
        ):
            data = copy.deepcopy(self.config)
            data["entities"].append({**data["entities"][0], **update})
            with self.subTest(update=update), self.assertRaises(CodegenError):
                validate(data, self.base)
        for name in ("int", "str", "p", "ModelView", "bad.name", "_private", "class"):
            data = copy.deepcopy(self.config)
            data["entities"][0]["python"] = name
            with self.subTest(name=name), self.assertRaises(CodegenError):
                validate(data, self.base)
        data = copy.deepcopy(self.config)
        data["graphs"] = [{"name": "graph", "rust": "graph", "python": "ThingActive"}]
        with self.assertRaises(CodegenError):
            validate(data, self.base)
        data = copy.deepcopy(self.config)
        data["entities"][0]["python"] = "Key"
        data["entities"].append(
            {"name": "second", "rust": "second::Entity", "python": "Key"}
        )
        with self.assertRaises(CodegenError):
            validate(data, self.base)

    # [spec:pgorm:req:bindings.codegen-coverage/test]    one-of references and source tuples are described, a tuple listing only described entities
    def test_one_ofs_and_source_tuples_are_described(self):
        data = copy.deepcopy(self.config)
        data["entities"].append({"name": "app.Other", "rust": "other::Entity", "python": "Other"})
        data["one_ofs"] = [{"name": "app.Target", "rust": "thing::Target", "python": "Target"}]
        data["sources"] = [
            {"name": "app.Pair", "rust": ["thing::Entity", "other::Entity"], "python": "Pair"}
        ]
        validated = validate(data, self.base)
        self.assertEqual(validated["one_ofs"], data["one_ofs"])
        self.assertEqual(validated["sources"], data["sources"])
        for rust in ([], ["missing::Entity"], "thing::Entity", ["thing::Entity"] * 7):
            refused = copy.deepcopy(data)
            refused["sources"][0]["rust"] = rust
            with self.subTest(rust=rust), self.assertRaises(CodegenError):
                validate(refused, self.base)
        refused = copy.deepcopy(data)
        refused["one_ofs"][0]["python"] = "Thing"
        with self.assertRaises(CodegenError):
            validate(refused, self.base)

    # [spec:pgorm:req:bindings.codegen-disjoint/test]    a description's disjoint enums are named by Rust path, registration and export, each unique and none colliding with another export, or refused before anything is written
    def test_disjoint_enums_are_described_uniquely(self):
        data = copy.deepcopy(self.config)
        data["disjoints"] = [{"name": "app.Things", "rust": "thing::Things", "python": "Things"}]
        self.assertEqual(validate(data, self.base)["disjoints"], data["disjoints"])
        for update in (
            {"name": "app.Things", "rust": "thing::Other", "python": "Other"},
            {"name": "app.Other", "rust": "thing::Things", "python": "Other"},
            {"name": "app.Other", "rust": "thing::Other", "python": "ThingsRow"},
            {"name": "app.Other", "rust": "thing::Other", "python": "ThingModel"},
            {"name": "app.Other", "rust": "thing::Other", "python": "WritableDisjointView"},
            {"name": "app.Other", "rust": "thing::<T>", "python": "Other"},
            {"name": "app.Other", "rust": "thing::Other", "python": "Other", "fields": {}},
        ):
            broken = copy.deepcopy(data)
            broken["disjoints"].append(update)
            with self.subTest(update=update), self.assertRaises(CodegenError):
                validate(broken, self.base)

    # [spec:pgorm:req:bindings.codegen-disjoint/test]    a disjoint enum's rows are typed as a union over its kinds, a kinded enum's view writes through its entities' ActiveModels, an unkinded one's reads alone, and an enum whose base or kind entity is not exported is refused
    def test_disjoint_rows_type_as_union_over_kinds(self):
        entities = {
            name: {"python": name.split(".")[1]} for name in ("app.Box", "app.Crate", "app.Bag")
        }
        described = {
            "kind_field": "kind",
            "base": "app.Box",
            "kinds": [{"value": "crate", "model": "app.Crate"}, {"value": "loose", "model": None}],
        }
        code, stubs = disjoint_code({"name": "app.Boxes", "python": "Boxes"}, described, entities)
        self.assertIn(
            'BoxesRow: TypeAlias = (\n    tuple[BoxModel, Literal["crate"], CrateModel]\n'
            '    | tuple[BoxModel, Literal["loose"], None]\n)',
            stubs,
        )
        self.assertIn(
            'Boxes: WritableDisjointView[BoxesRow, Literal["crate", "loose"], BoxActive, CrateActive]',
            stubs,
        )
        ast.parse("\n\n\n".join(code))
        _, unkinded = disjoint_code(
            {"name": "app.Boxes", "python": "Boxes"}, {**described, "kind_field": None}, entities
        )
        self.assertIn("Boxes: DisjointView[BoxesRow]", unkinded)
        for missing in ({**described, "base": "app.Missing"},
                        {**described, "kinds": [{"value": "x", "model": "app.Missing"}]}):
            with self.subTest(missing=missing), self.assertRaises(CodegenError):
                disjoint_code({"name": "app.Boxes", "python": "Boxes"}, missing, entities)

    # [spec:pgorm:req:bindings.codegen-layout/test]    the module is laid out as black lays it out: double quotes, a call's arguments on a line of their own or one to a line within 88 columns, a long conditional split before its if and its else, a long union a member to a line, a long stub annotation an argument to a line, and only the imports the code uses, grouped and sorted
    def test_generated_python_takes_blacks_layout(self):
        entity = {
            "columns": [
                {"name": "id", "json_key": "id", "rust_type": "i32", "kind": "i32"},
                {
                    "name": "display name",
                    "json_key": "displayName",
                    "rust_type": "String",
                    "kind": "text",
                },
                {
                    "name": "mood",
                    "json_key": "mood",
                    "rust_type": "Mood",
                    "kind": 'enum "python_entities"."Mood"',
                },
            ]
        }
        entry = {"name": "app.Account", "python": "Account", "fields": {"display name": "display_name"}}
        code, stubs = entity_code(entry, entity)
        self.assertEqual(
            code[1],
            """class AccountActive(ActiveView[AccountModel]):
    _entity_name = "app.Account"
    _model_class = AccountModel

    def set_id(self, value: int | p.Value) -> AccountActive:
        return self.set(
            "id", value if isinstance(value, p.Value) else p.Value(value, "i32")
        )

    def set_display_name(self, value: str | p.Value) -> AccountActive:
        return self.set(
            "display name",
            value if isinstance(value, p.Value) else p.Value(value, "text"),
        )

    def set_mood(self, value: str | p.Value) -> AccountActive:
        return self.set(
            "mood",
            value
            if isinstance(value, p.Value)
            else p.Value(value, p.TypeName("Mood", schema="python_entities")),
        )""",
        )
        self.assertEqual(code[2], 'AccountModel._active_class = AccountActive\nAccount = EntityView(p.entity("app.Account"), AccountModel, AccountActive)')
        entities = {name: {"python": name.split(".")[1]} for name in ("app.Delivery", "app.Parcel", "app.Pallet")}
        described = {
            "kind_field": "kind",
            "base": "app.Delivery",
            "kinds": [
                {"value": "parcel", "model": "app.Parcel"},
                {"value": "letter", "model": None},
                {"value": "pallet", "model": "app.Pallet"},
            ],
        }
        code, stubs = disjoint_code({"name": "app.Deliveries", "python": "Deliveries"}, described, entities)
        self.assertEqual(
            stubs,
            [
                """DeliveriesRow: TypeAlias = (
    tuple[DeliveryModel, Literal["parcel"], ParcelModel]
    | tuple[DeliveryModel, Literal["letter"], None]
    | tuple[DeliveryModel, Literal["pallet"], PalletModel]
)""",
                """Deliveries: WritableDisjointView[
    DeliveriesRow,
    Literal["parcel", "letter", "pallet"],
    DeliveryActive,
    ParcelActive | PalletActive,
]""",
            ],
        )
        body = "\n\n\n".join(code)
        self.assertTrue(all(len(line) <= 88 for line in body.splitlines()), body)
        self.assertEqual(
            import_block(body),
            "from __future__ import annotations\n\n"
            "from typing import Any, Literal, TypeAlias, cast\n\n"
            "import pgorm as p\n"
            "from pgorm._registered.disjoints import WritableDisjointView, wrap_disjoint_row",
        )
        self.assertEqual(
            literal({"entities": ["app.Account"], "features": ["macros"] * 12}, 0, 17),
            """{
    "entities": ["app.Account"],
    "features": [
        "macros",
        "macros",
        "macros",
        "macros",
        "macros",
        "macros",
        "macros",
        "macros",
        "macros",
        "macros",
        "macros",
        "macros",
    ],
}""",
        )

    # [spec:pgorm:req:bindings.codegen/test]
    def test_build_input_does_not_accept_code(self):
        for path in (
            "crate::Entity",
            "r#self::Entity",
            "super::Entity",
            "a::<T>",
            "a();boom()",
            "type::Entity",
        ):
            with self.subTest(path=path), self.assertRaises(CodegenError):
                rust_path(path)
        self.assertEqual(rust_path("r#type::Entity"), "r#type::Entity")
        self.assertEqual(rust_string('a"\\\n\t\x01東京'), '"a\\"\\\\\\n\\t\\u{1}東京"')
        for key, value in (
            ("schema_version", True),
            ("module", "../x"),
            ("entities", {}),
            ("unknown", 1),
        ):
            with self.subTest(key=key), self.assertRaises(CodegenError):
                validate({**self.config, key: value}, self.base)

    # [spec:pgorm:req:python.codegen+3/test]
    def test_reflection_preserves_standard_value_tags(self):
        cases = [
            ("i8", "int", "i8"),
            ("i32", "int", "i32"),
            ("Option<String>", "str | None", "text"),
            ("Vec<u8>", "bytes", "bytes"),
            ("DateTimeWithTimeZone", "datetime", "datetime_utc"),
            ("jiff::Timestamp", "datetime", "datetime_utc"),
            ("Option<DateTime>", "datetime | None", "datetime"),
            ("rust_decimal::Decimal", "Decimal", "decimal"),
            ("Interval", "p.Interval", "interval"),
            ("Option<pgorm::pgorm_query::Interval>", "p.Interval | None", "interval"),
        ]
        for rust, annotation, kind in cases:
            mapped = field_type({"rust_type": rust, "kind": "i64"})
            with self.subTest(rust=rust):
                self.assertEqual(
                    (mapped.annotation, mapped.kind), (annotation, string(kind))
                )
        for rust in (None, "ApplicationAlias", "Vec<ApplicationAlias>"):
            mapped = field_type({"rust_type": rust, "kind": "i32"})
            self.assertEqual(mapped.annotation, "Any")
        array = field_type({"rust_type": "Option<Vec<Option<i32>>>", "kind": "i32[]"})
        self.assertEqual(array.annotation, "list[int | None] | None")
        self.assertIn('Value.array("i32"', statement(array.binding()))
        intervals = field_type(
            {"rust_type": "Option < Vec < Interval > >", "kind": "interval[]"}
        )
        self.assertEqual(intervals.annotation, "list[p.Interval] | None")
        self.assertIn('Value.array("interval"', statement(intervals.binding()))
        enum = field_type({"rust_type": "Mood", "kind": 'enum "a""b"."Mo.od"'})
        self.assertEqual(enum.annotation, "str")
        self.assertIn("""p.TypeName("Mo.od", schema='a"b')""", statement(enum.binding()))
        bare = field_type({"rust_type": "Vec<Mood>", "kind": 'enum "Mood"[]'})
        self.assertEqual(bare.annotation, "list[str]")
        self.assertIn('p.TypeName("Mood", schema=None)', statement(bare.binding()))
        for kind in ('enum Mood', 'enum "a"."b"."c"', 'enum "a"b'):
            with self.subTest(kind=kind), self.assertRaises(CodegenError):
                field_type({"rust_type": "Mood", "kind": kind})

    # [spec:pgorm:req:bindings.codegen/test]
    def test_field_aliases_resolve_real_column_names(self):
        columns = [
            {
                "name": "display name",
                "json_key": "displayName",
                "rust_type": "String",
                "kind": "text",
            }
        ]
        entry = {"name": "app.Entity", "python": "Thing", "fields": {}}
        entity = {"columns": columns}
        self.assertEqual(attributes(entity, entry)[0][1], "displayName")
        code, stubs = entity_code(
            {**entry, "fields": {"display name": "display_name"}}, entity
        )
        self.assertIn('self["display name"]', "\n".join(code))
        ast.parse("\n\n\n".join(code))
        ast.parse("\n".join(stubs))
        for fields in (
            {"missing": "x"},
            {"display name": "native"},
            {"display name": "property"},
        ):
            with self.subTest(fields=fields), self.assertRaises(CodegenError):
                attributes(entity, {**entry, "fields": fields})
        with self.assertRaises(CodegenError):
            attributes(
                {"columns": columns + [{**columns[0], "name": "displayName"}]}, entry
            )
