"""A declared model's query clauses, projections, partial models and cursor
secondary orderings, against a live server: each still decodes typed rows,
and a row that breaks a declaration is a DecodeError."""

from dataclasses import dataclass
from decimal import Decimal
import os
import unittest

import pgorm as p

SCHEMA = "python_projections"

Account = p.Model(
    "account",
    {
        "id": p.Column("i64", primary_key=True),
        "name": p.Column("text"),
        "nickname": p.Column("text", name="display_name", nullable=True),
    },
    schema=SCHEMA,
)
Post = p.Model(
    "post",
    {
        "id": p.Column("i64", primary_key=True),
        "author_id": p.Column("i64"),
        "title": p.Column("text"),
        "score": p.Column("decimal"),
    },
    schema=SCHEMA,
)
Tag = p.Model("tag", {"id": p.Column("i32", primary_key=True)}, schema=SCHEMA)

author = Post.belongs_to(Account, from_="author_id", to="id")
posts = Account.has_many(Post, from_="id", to="author_id")

DDL = [
    f"CREATE SCHEMA {SCHEMA}",
    f"""CREATE TABLE {SCHEMA}.account (
        id int8 PRIMARY KEY, name text NOT NULL, display_name text
    )""",
    f"""CREATE TABLE {SCHEMA}.post (
        id int8 PRIMARY KEY, author_id int8 NOT NULL REFERENCES {SCHEMA}.account,
        title text NOT NULL, score numeric NOT NULL
    )""",
    f"CREATE TABLE {SCHEMA}.tag (id int4 PRIMARY KEY)",
]

posts_counted = p.call("count", Post.col("id"))


@dataclass(frozen=True)
class Tally:
    name: str
    posts: int


class StatementClauses(unittest.TestCase):
    # [spec:pgorm:req:bindings.distinct-on/test]
    def test_distinct_on_takes_columns(self):
        query = p.Select(Post.col("author_id"), Post.col("title")).from_(Post.table)
        self.assertEqual(
            query.distinct_on(Post.col("author_id")).order_by(Post.col("author_id").asc()).inspect().sql,
            'SELECT DISTINCT ON ("post"."author_id") "post"."author_id", "post"."title" '
            'FROM "python_projections"."post" ORDER BY "post"."author_id" ASC',
        )
        self.assertEqual(
            query.distinct_on(Post.col("author_id")).distinct().inspect().sql,
            'SELECT DISTINCT "post"."author_id", "post"."title" FROM "python_projections"."post"',
        )
        with self.assertRaisesRegex(p.ConstructionError, "takes columns"):
            query.distinct_on(Post.col("author_id") + 1)
        with self.assertRaisesRegex(p.ConstructionError, "at least one column"):
            query.distinct_on()

    # [spec:pgorm:req:bindings.condition-option/test]
    def test_add_option_adds_a_present_condition(self):
        base = p.Condition.all(Post.col("author_id") == 1)
        self.assertEqual(base.add_option(None).inspect().sql, base.inspect().sql)
        self.assertEqual(
            base.add_option(Post.col("title") == "a").inspect().sql,
            base.add(Post.col("title") == "a").inspect().sql,
        )
        self.assertEqual(
            base.add_option(p.Condition.any()).inspect().sql,
            base.add(p.Condition.any()).inspect().sql,
        )
        with self.assertRaises(TypeError):
            base.add_option(1)


class Construction(unittest.TestCase):
    # [spec:pgorm:req:bindings.model-projections/test]
    def test_a_projection_refuses_what_it_cannot_read(self):
        for operation in (
            "model.clauses",
            "model.projection",
            "model.partial_model",
            "model.cursor_secondary_order",
        ):
            p.require_capability(operation)
        empty = Account.find().select_only()
        with self.assertRaisesRegex(p.ConstructionError, "at least one item"):
            empty.into_tuple()
        with self.assertRaisesRegex(p.ConstructionError, "at least one item"):
            empty.inspect()
        with self.assertRaisesRegex(p.ConstructionError, "named 'name'"):
            empty.column(Account.col("name")).column_as(Account.col("id"), "name")
        with self.assertRaises(TypeError):
            empty.column(p.col("name"))  # type: ignore[arg-type]
        with self.assertRaises(TypeError):
            empty.expr_as(p.call("count", Account.col("id")), "n", "i64")  # type: ignore[arg-type]
        with self.assertRaisesRegex(p.ConstructionError, "how it decodes"):
            empty.expr_as(p.call("count", Account.col("id")), "n", p.Column("i64", primary_key=True))
        named = empty.column(Account.col("name"))
        with self.assertRaisesRegex(p.ConstructionError, "no item 'id'"):
            named.into_values("id")
        with self.assertRaisesRegex(p.ConstructionError, "twice"):
            named.into_values("name", "name")
        with self.assertRaises(TypeError):
            named.into_model(1)  # type: ignore[arg-type]
        # A field projects under its field's name, as the model's query does.
        self.assertEqual(
            empty.column(Account.col("nickname")).inspect().sql,
            'SELECT "account"."display_name" AS "nickname" FROM "python_projections"."account"',
        )
        self.assertEqual(
            Account.select("name").column_as(Account.col("id"), "id").inspect().sql,
            'SELECT "account"."name", "account"."id" AS "id" FROM "python_projections"."account"',
        )

    # [spec:pgorm:req:bindings.partial-models/test]
    def test_a_partial_model_declares_its_fields(self):
        with self.assertRaises(TypeError):
            p.PartialModel(Account, {"id": "id"})  # type: ignore[dict-item]
        with self.assertRaisesRegex(p.ConstructionError, "at least one field"):
            p.PartialModel(Account, {"cached": p.Skip(0)})
        with self.assertRaisesRegex(p.ConstructionError, "how it decodes"):
            p.FromExpr(Account.col("name"), p.Column("text", name="x"))
        line = p.PartialModel(Account, {"id": Account.col("id")})
        with self.assertRaisesRegex(p.ConstructionError, "reads account, and the query post"):
            Post.find().into_partial_model(line)

    # [spec:pgorm:req:bindings.model-clauses/test]
    def test_clauses_refuse_what_the_builder_refuses(self):
        with self.assertRaises(TypeError):
            Account.find().lock("update")  # type: ignore[arg-type]
        with self.assertRaisesRegex(p.ConstructionError, "at least one column"):
            Account.find().distinct_on()
        with self.assertRaisesRegex(p.ConstructionError, "takes columns"):
            Account.find().distinct_on(Account.col("id") + 1).inspect()
        with self.assertRaisesRegex(p.ConstructionError, "ends at account, which the query does not read"):
            Tag.find().join_rev(author)

    # [spec:pgorm:req:bindings.cursor-secondary-order+1/test]
    def test_a_secondary_ordering_names_a_read_table(self):
        cursor = Post.find().cursor("author_id")
        with self.assertRaisesRegex(p.ConstructionError, "reads no table named account"):
            cursor.add_secondary_order_by(Account, "id")
        with self.assertRaisesRegex(p.ConstructionError, "no field 'nope'"):
            cursor.add_secondary_order_by(Post, "nope")
        ordered = cursor.add_secondary_order_by(Post, "id")
        with self.assertRaisesRegex(p.ConstructionError, "1 or 2 key column"):
            ordered.after_with(1, 2, 3)
        with self.assertRaisesRegex(p.ConstructionError, "does not match 1 key column"):
            ordered.after(1, 2)


class LiveProjections(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=2)
        for statement in DDL:
            await self.pool.execute(p.RawSQL(statement))
        await Account.insert_many(
            [
                {"id": 1, "name": "Ann", "nickname": "A"},
                {"id": 2, "name": "Bob", "nickname": None},
                {"id": 3, "name": "Cy", "nickname": None},
            ]
        ).execute(self.pool)
        await Post.insert_many(
            [
                {"id": 10, "author_id": 1, "title": "a1", "score": Decimal("1.5")},
                {"id": 11, "author_id": 1, "title": "a2", "score": Decimal("2.5")},
                {"id": 12, "author_id": 2, "title": "b1", "score": Decimal("4.0")},
                {"id": 13, "author_id": 1, "title": "a3", "score": Decimal("1.0")},
            ]
        ).execute(self.pool)

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL(f"DROP SCHEMA {SCHEMA} CASCADE"))
        await self.pool.close()

    # [spec:pgorm:req:bindings.model-clauses/test]
    async def test_clauses_keep_the_models_records(self):
        distinct = Account.select("id").join(posts).distinct().order_by(Account.col("id").asc())
        self.assertEqual([row["id"] for row in await distinct.all(self.pool)], [1, 2])
        latest = (
            Post.select("author_id", "title")
            .distinct_on(Post.col("author_id"))
            .order_by(Post.col("author_id").asc(), Post.col("id").desc())
        )
        self.assertEqual(
            [dict(row) for row in await latest.all(self.pool)],
            [{"author_id": 1, "title": "a3"}, {"author_id": 2, "title": "b1"}],
        )
        busy = (
            Account.find()
            .join(posts)
            .group_by(Account.col("id"))
            .having(posts_counted > 1)
        )
        self.assertEqual([row["name"] for row in await busy.all(self.pool)], ["Ann"])
        self.assertEqual(await busy.count(self.pool), 1)

    # [spec:pgorm:req:bindings.model-clauses/test]
    async def test_a_locked_query_reads_in_a_transaction(self):
        queue = (
            Post.find()
            .order_by(Post.col("id").asc())
            .limit(1)
            .lock(p.LockStrength.Update, wait=p.LockWait.SkipLocked)
        )
        self.assertTrue(queue.inspect().sql.endswith("LIMIT $1 FOR UPDATE SKIP LOCKED"))
        async with self.pool.transaction() as tx:
            taken = await queue.one(tx)
            self.assertEqual(taken["id"], 10)
            async with self.pool.transaction() as other:
                # The other worker passes over the row the first one holds.
                self.assertEqual((await queue.one(other))["id"], 11)
            shared = Post.find().where(Post.col("id") == 10).lock(
                p.LockStrength.Share, wait=p.LockWait.Nowait
            )
            with self.assertRaises(p.DatabaseError):
                async with self.pool.transaction() as other:
                    await shared.one(other)

    # [spec:pgorm:req:bindings.model-clauses/test]
    async def test_right_and_reverse_joins(self):
        tallies = (
            Post.find()
            .join(author, kind=p.Join.Right)
            .select_only()
            .column(Account.col("name"))
            .expr_as(posts_counted, "posts", p.Column("i64"))
            .group_by(Account.col("name"))
            .order_by(Account.col("name").asc())
        )
        self.assertEqual(
            await tallies.into_tuple().all(self.pool), [("Ann", 3), ("Bob", 1), ("Cy", 0)]
        )
        # Cy has no post, so the right join's row holds NULL in each post field.
        with self.assertRaisesRegex(p.DecodeError, "NULL"):
            await Post.find().join(author, kind=p.Join.Right).all(self.pool)
        writers = Account.find().join_rev(author).where(Post.col("title") == "b1")
        self.assertEqual(
            writers.inspect().sql,
            'SELECT "account"."id", "account"."name", "account"."display_name" AS "nickname" '
            'FROM "python_projections"."account" INNER JOIN "python_projections"."post" '
            'ON "post"."author_id" = "account"."id" WHERE "post"."title" = $1',
        )
        self.assertEqual([row["name"] for row in await writers.all(self.pool)], ["Bob"])

    # [spec:pgorm:req:bindings.model-projections/test]
    async def test_projections_decode_typed_rows(self):
        by_author = (
            Post.find()
            .join(author)
            .select_only()
            .column_as(Account.col("name"), "name")
            .expr_as(posts_counted, "posts", p.Column("i64"))
            .expr_as(p.call("sum", Post.col("score")), "total", p.Column("decimal", nullable=True))
            .group_by(Account.col("name"))
            .order_by(Account.col("name").asc())
        )
        self.assertEqual(
            await by_author.into_tuple().all(self.pool),
            [("Ann", 3, Decimal("5.0")), ("Bob", 1, Decimal("4.0"))],
        )
        self.assertEqual(
            await by_author.into_values("posts", "name").all(self.pool), [(3, "Ann"), (1, "Bob")]
        )
        self.assertEqual(
            await by_author.into_model(dict).all(self.pool),
            [
                {"name": "Ann", "posts": 3, "total": Decimal("5.0")},
                {"name": "Bob", "posts": 1, "total": Decimal("4.0")},
            ],
        )
        self.assertEqual(
            await Post.find()
            .join(author)
            .select_only()
            .column(Account.col("name"))
            .expr_as(posts_counted, "posts", p.Column("i64"))
            .group_by(Account.col("name"))
            .order_by(Account.col("name").asc())
            .into_model(Tally)
            .all(self.pool),
            [Tally("Ann", 3), Tally("Bob", 1)],
        )
        names = Account.select("name", "nickname").order_by(Account.col("id").asc())
        self.assertEqual(
            await names.into_tuple().all(self.pool), [("Ann", "A"), ("Bob", None), ("Cy", None)]
        )
        self.assertEqual(await names.into_tuple().count(self.pool), 3)
        page = await names.into_values("name").paginate(2).fetch_page(self.pool, 1)
        self.assertEqual(page, [("Cy",)])
        stream = await names.into_values("name").stream(self.pool)
        self.assertEqual([row async for row in stream], [("Ann",), ("Bob",), ("Cy",)])
        window = await names.into_values("name").cursor("id").after(1).first(1).all(self.pool)
        self.assertEqual(window, [("Bob",)])

    # [spec:pgorm:req:bindings.model-projections/test]
    async def test_a_row_breaking_its_declaration_is_refused(self):
        counted = Post.find().select_only().expr_as(posts_counted, "posts", p.Column("i32"))
        with self.assertRaisesRegex(p.DecodeError, "differs from the declared"):
            await counted.into_tuple().one(self.pool)
        nothing = Post.find().where(Post.col("id") == 99).select_only()
        summed = nothing.expr_as(p.call("sum", Post.col("score")), "total", p.Column("decimal"))
        with self.assertRaisesRegex(p.DecodeError, "NULL"):
            await summed.into_tuple().one(self.pool)
        summed = nothing.expr_as(
            p.call("sum", Post.col("score")), "total", p.Column("decimal", nullable=True)
        )
        self.assertEqual(await summed.into_tuple().one(self.pool), (None,))
        labelled = Account.find().select_only().expr_as(
            Account.col("name"), "label", p.Column(p.TypeName("text", schema="pg_catalog"), values=["Ann"])
        )
        with self.assertRaises(p.DecodeError):
            await labelled.into_tuple().all(self.pool)

    # [spec:pgorm:req:bindings.partial-models/test]
    async def test_a_partial_model_selects_and_decodes(self):
        line = p.PartialModel(
            Account,
            {
                "id": Account.col("id"),
                "shout": p.FromExpr(p.call("upper", Account.col("name")), p.Column("text")),
                "display": Account.col("nickname"),
                "cached": p.Skip("none"),
            },
        )
        query = Account.find().where(Account.col("id") <= 2).order_by(Account.col("id").asc())
        read = query.into_partial_model(line)
        self.assertEqual(
            read.inspect().sql,
            'SELECT "account"."id", UPPER("account"."name") AS "shout", '
            '"account"."display_name" AS "display" FROM "python_projections"."account" '
            'WHERE "account"."id" <= $1 ORDER BY "account"."id" ASC',
        )
        rows = await read.all(self.pool)
        self.assertEqual(
            [dict(row) for row in rows],
            [
                {"id": 1, "shout": "ANN", "display": "A", "cached": "none"},
                {"id": 2, "shout": "BOB", "display": None, "cached": "none"},
            ],
        )
        with self.assertRaises(TypeError):
            rows[0]["id"] = 3  # type: ignore[index]
        self.assertEqual(list(rows[0]), ["id", "shout", "display", "cached"])
        strict = p.PartialModel(
            Account,
            {"shout": p.FromExpr(p.call("upper", Account.col("nickname")), p.Column("text"))},
        )
        with self.assertRaisesRegex(p.DecodeError, "NULL"):
            await query.into_partial_model(strict).all(self.pool)

    # [spec:pgorm:req:bindings.cursor-secondary-order+1/test]
    async def test_a_secondary_ordering_breaks_key_ties(self):
        by_author = Post.find().cursor("author_id").add_secondary_order_by(Post, "id")
        first = await by_author.first(2).all(self.pool)
        self.assertEqual([row["id"] for row in first], [10, 11])
        # `after` takes the key alone and skips the rest of author 1's posts.
        self.assertEqual([row["id"] for row in await by_author.after(1).all(self.pool)], [12])
        # `after_with` takes the whole ordering and resumes inside the run.
        self.assertEqual(
            [row["id"] for row in await by_author.after_with(1, 11).first(2).all(self.pool)], [13, 12]
        )
        self.assertEqual(
            [row["id"] for row in await by_author.before_with(1, 13).last(1).all(self.pool)], [11]
        )
        joined = (
            Post.find()
            .join(author)
            .cursor("score")
            .add_secondary_order_by(Account, "name")
            .add_secondary_order_by(Post, "id")
        )
        self.assertEqual(
            joined.after_with(Decimal("1.5"), "Ann", 10).first(1).inspect().sql,
            'SELECT "post"."id", "post"."author_id", "post"."title", "post"."score" '
            'FROM "python_projections"."post" INNER JOIN "python_projections"."account" '
            'ON "post"."author_id" = "account"."id" WHERE ("post"."score" = $1 AND '
            '"account"."name" = $2 AND "post"."id" > $3) OR ("post"."score" = $4 AND '
            '"account"."name" > $5) OR "post"."score" > $6 ORDER BY "post"."score" ASC, '
            '"account"."name" ASC, "post"."id" ASC LIMIT $7',
        )
        self.assertEqual(
            [row["id"] for row in await joined.after_with(Decimal("1.5"), "Ann", 10).all(self.pool)],
            [11, 12],
        )
        graph = Post.graph().join_one(author).cursor("title").add_secondary_order_by(Account, "name")
        self.assertEqual(len((await graph.first(1).all(self.pool))[0]), 2)


if __name__ == "__main__":
    unittest.main()
