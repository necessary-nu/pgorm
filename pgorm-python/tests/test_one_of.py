"""One-of references declared over runtime models: the declaration and its
refusals, the CHECK pgorm's shape writes, and a reference read, set and
loaded on a live server."""

import os
import unittest
from unittest.mock import patch

import pgorm as p
import pgorm.schema as ps

SCHEMA = "python_one_of"


def post():
    return p.Model(
        "post",
        {"id": p.Column("i32", primary_key=True), "title": p.Column("text")},
        schema=SCHEMA,
    )


def photo():
    return p.Model(
        "photo",
        {"id": p.Column("i64", primary_key=True), "url": p.Column("text")},
        schema=SCHEMA,
    )


def comment():
    return p.Model(
        "comment",
        {
            "id": p.Column("i32", primary_key=True),
            "body": p.Column("text"),
            "post": p.Column("i32", name="post_id", nullable=True),
            "photo": p.Column("i64", name="photo_id", nullable=True),
        },
        schema=SCHEMA,
    )


def commentable(**options):
    return p.OneOf(
        "commentable",
        comment(),
        {"post": ("post", post()), "photo": ("photo", photo())},
        **options,
    )


class OneOfDeclaration(unittest.TestCase):
    # [spec:pgorm:req:bindings.one-of+1/test]    a reference declared over models sends nothing, describes itself under the declared keys, and writes the CHECK pgorm's shape writes: at most one target where it is optional, exactly one otherwise
    def test_declaration_describes_itself_and_its_check(self):
        for operation in ("one_of.declare", "one_of.key", "one_of.set", "one_of.load", "one_of.schema"):
            p.require_capability(operation)
        with patch("socket.socket", side_effect=AssertionError("a declaration connected")):
            optional = commentable(optional=True)
            required = commentable()
        self.assertEqual(
            optional.describe(),
            {
                "name": "commentable",
                "model": "comment",
                "targets": [
                    {"value": "post", "field": "post", "target": "post"},
                    {"value": "photo", "field": "photo", "target": "photo"},
                ],
                "optional": True,
            },
        )
        self.assertEqual(
            ps.create_one_of(optional).inspect().sql,
            f'ALTER TABLE "{SCHEMA}"."comment" ADD CONSTRAINT "ck-comment-commentable" '
            'CHECK (num_nonnulls("post_id", "photo_id") <= 1)',
        )
        self.assertIn("= 1)", ps.create_one_of(required).inspect().sql)
        self.assertIsInstance(ps.create_one_of(optional), ps.AlterTable)

    # [spec:pgorm:req:bindings.one-of+1/test]    what the derive refuses is a ConstructionError raised as the reference is declared: a field the model lacks, one not nullable, one field for two targets, one of another kind than its target's key, a target keyed by more than one field, one target model twice, no target and a name past 59 bytes; an argument of the wrong type is a TypeError
    def test_declaration_refuses_what_the_derive_refuses(self):
        pair = p.Model(
            "pair",
            {"a": p.Column("i32", primary_key=True), "b": p.Column("i32", primary_key=True)},
            schema=SCHEMA,
        )
        refused = {
            "missing field": ({"post": ("missing", post())}, None),
            "not nullable": ({"post": ("id", post())}, "is not nullable"),
            "one field twice": ({"post": ("post", post()), "photo": ("post", photo())}, "holds the keys"),
            "another kind": ({"photo": ("post", photo())}, "the key of"),
            "composite key": ({"pair": ("post", pair)}, "keyed by 2 columns"),
            "one target twice": ({"post": ("post", post()), "again": ("photo", post())}, "is the target of"),
            "no target": ({}, "names no target"),
        }
        for case, (targets, message) in refused.items():
            with self.subTest(case=case), self.assertRaises(p.ConstructionError) as raised:
                p.OneOf("commentable", comment(), targets)
            if message is not None:
                self.assertIn(message, str(raised.exception))
        with self.assertRaises(p.ConstructionError):
            p.OneOf("n" * 60, comment(), {"post": ("post", post())})
        for wrong in (
            lambda: p.OneOf(1, comment(), {"post": ("post", post())}),
            lambda: p.OneOf("c", "comment", {"post": ("post", post())}),
            lambda: p.OneOf("c", comment(), [("post", ("post", post()))]),
            lambda: p.OneOf("c", comment(), {"post": ("post",)}),
            lambda: p.OneOf("c", comment(), {1: ("post", post())}),
            lambda: p.OneOf("c", comment(), {"post": ("post", post())}, optional=1),
            lambda: ps.create_one_of(comment()),
        ):
            with self.assertRaises(TypeError):
                wrong()

    # [spec:pgorm:req:bindings.one-of+1/test]    set gives the fields a write sets, its target's field to the key and every other to None, all None for no target only where the reference is optional; an unknown target, a target without its key and no target where one is required are ConstructionErrors
    def test_set_gives_the_fields_a_write_sets(self):
        optional, required = commentable(optional=True), commentable()
        self.assertEqual(optional.set("photo", 2), {"post": None, "photo": 2})
        self.assertEqual(optional.set(None), {"post": None, "photo": None})
        for refused in (
            lambda: required.set(None),
            lambda: optional.set("video", 1),
            lambda: optional.set("post"),
        ):
            with self.assertRaises(p.ConstructionError):
                refused()
        with self.assertRaises(TypeError):
            optional.set(1, 2)


class OneOfDatabase(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=2)
        for sql in (
            f"CREATE SCHEMA {SCHEMA}",
            f"CREATE TABLE {SCHEMA}.post (id integer PRIMARY KEY, title text NOT NULL)",
            f"CREATE TABLE {SCHEMA}.photo (id bigint PRIMARY KEY, url text NOT NULL)",
            f"CREATE TABLE {SCHEMA}.comment (id integer PRIMARY KEY, body text NOT NULL, "
            f"post_id integer REFERENCES {SCHEMA}.post (id), "
            f"photo_id bigint REFERENCES {SCHEMA}.photo (id))",
        ):
            await self.pool.execute(p.RawSQL(sql))
        self.reference = commentable(optional=True)
        await self.pool.execute(ps.create_one_of(self.reference))
        for at in (1, 2):
            await post().insert({"id": at, "title": f"post {at}"}).execute(self.pool)
            await photo().insert({"id": at, "url": f"{at}.png"}).execute(self.pool)

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL(f"DROP SCHEMA {SCHEMA} CASCADE"))
        await self.pool.close()

    async def comment_on(self, at, body, value, key=None):
        values = {"id": at, "body": body, **self.reference.set(value, key)}
        return await comment().insert(values).returning().one(self.pool)

    # [spec:pgorm:req:bindings.one-of+1/test]    the CHECK the shape writes holds every row on the server: a row referencing two targets is refused, one referencing none is taken where the reference is optional
    async def test_check_holds_rows_live(self):
        await self.comment_on(1, "loose", None)
        with self.assertRaises(p.CheckViolationError) as raised:
            await self.pool.execute(
                p.RawSQL(f"INSERT INTO {SCHEMA}.comment VALUES (2, 'both', 1, 1)")
            )
        self.assertEqual(raised.exception.constraint, "ck-comment-commentable")

    # [spec:pgorm:req:bindings.one-of+1/test]    each reference written through set reads back as its tagged key, None for no target; a row setting two fields is a DecodeError naming the table, its key and the fields
    async def test_set_and_key_round_trip_live(self):
        written = [
            await self.comment_on(1, "a", "photo", 2),
            await self.comment_on(2, "b", "post", 1),
            await self.comment_on(3, "c", None),
        ]
        self.assertEqual(
            [self.reference.key(record) for record in written],
            [("photo", 2), ("post", 1), None],
        )
        rewritten = await (
            comment()
            .update(self.reference.set("post", 2))
            .where(comment().col("id") == 1)
            .returning()
            .one(self.pool)
        )
        self.assertEqual(self.reference.key(rewritten), ("post", 2))
        self.assertIsNone(rewritten["photo"])
        await self.pool.execute(p.RawSQL(f"ALTER TABLE {SCHEMA}.comment DROP CONSTRAINT \"ck-comment-commentable\""))
        await self.pool.execute(p.RawSQL(f"INSERT INTO {SCHEMA}.comment VALUES (9, 'both', 1, 1)"))
        both = await comment().find_by_key({"id": 9}).one(self.pool)
        with self.assertRaises(p.DecodeError) as raised:
            self.reference.key(both)
        self.assertIn(
            "the `python_one_of.comment` row whose key is (`id` = 9) references more than one "
            "target, setting `post_id`, `photo_id`",
            str(raised.exception),
        )

    # [spec:pgorm:req:bindings.one-of+1/test]    load gives each record its target's record, tagged, in order, None for no target, reading each target's table once; a key its table does not hold is a DecodeError naming the table, the row's key, the target's key and the field
    async def test_load_reads_each_target_live(self):
        for at, value, key in ((1, "photo", 2), (2, "post", 1), (3, None, None), (4, "post", 2), (5, "photo", 1)):
            await self.comment_on(at, str(at), value, key)
        records = await comment().find().order_by(comment().col("id").asc()).all(self.pool)
        loaded = await self.reference.load(self.pool, records)
        self.assertEqual(
            [None if item is None else (item[0], dict(item[1])) for item in loaded],
            [
                ("photo", {"id": 2, "url": "2.png"}),
                ("post", {"id": 1, "title": "post 1"}),
                None,
                ("post", {"id": 2, "title": "post 2"}),
                ("photo", {"id": 1, "url": "1.png"}),
            ],
        )
        self.assertEqual(await self.reference.load(self.pool, []), [])
        await self.pool.execute(p.RawSQL(f"ALTER TABLE {SCHEMA}.comment DROP CONSTRAINT comment_post_id_fkey"))
        await self.pool.execute(p.RawSQL(f"INSERT INTO {SCHEMA}.comment VALUES (7, 'dangling', 9, NULL)"))
        dangling = await comment().find_by_key({"id": 7}).one(self.pool)
        with self.assertRaises(p.DecodeError) as raised:
            await self.reference.load(self.pool, [dangling])
        self.assertIn(
            "the `python_one_of.comment` row whose key is (`id` = 7) references the "
            "`python_one_of.post` row of key 9 through `post_id`, and `python_one_of.post` holds "
            "no such row",
            str(raised.exception),
        )
        with self.assertRaises(TypeError):
            await self.reference.load(self.pool, records[0])
