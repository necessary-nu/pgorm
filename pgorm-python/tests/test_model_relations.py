"""Declared models following relations as pgorm's entities do, against a live
server: a relation's own condition, a path through a junction, a chain of
relations, loads of a filtered and ordered query, a relation starting from an
alias, a join narrowed in its ON, and cursors over a slot's fields and with a
secondary ordering."""

import os
import unittest

import pgorm as p

SCHEMA = "python_relations"

Account = p.Model(
    "account",
    {"id": p.Column("i64", primary_key=True), "name": p.Column("text"), "active": p.Column("bool")},
    schema=SCHEMA,
)
Post = p.Model(
    "post",
    {
        "id": p.Column("i64", primary_key=True),
        "author_id": p.Column("i64"),
        "editor_id": p.Column("i64", nullable=True),
        "title": p.Column("text"),
        "published": p.Column("bool"),
    },
    schema=SCHEMA,
)
Tag = p.Model("tag", {"id": p.Column("i32", primary_key=True), "label": p.Column("text")}, schema=SCHEMA)
PostTag = p.Model(
    "post_tag",
    {"post_id": p.Column("i64", primary_key=True), "tag_id": p.Column("i32", primary_key=True)},
    schema=SCHEMA,
)

posts = Account.has_many(Post, from_="id", to="author_id")
author = Post.belongs_to(Account, from_="author_id", to="id")
editor = Post.belongs_to(Account, from_="editor_id", to="id")
post_tags = Post.has_many(PostTag, from_="id", to="post_id")
tag_of = PostTag.belongs_to(Tag, from_="tag_id", to="id")
tag_post_tags = Tag.has_many(PostTag, from_="id", to="tag_id")
post_tag_post = PostTag.belongs_to(Post, from_="post_id", to="id")
published = Account.has_many(
    Post, from_="id", to="author_id", on_condition=lambda account, post: post.col("published") == True  # noqa: E712
)
active_author = Post.belongs_to(
    Account, from_="author_id", to="id", on_condition=lambda post, account: account.col("active") == True  # noqa: E712
)
tags = Post.related(tag_of, via=post_tags)

DDL = [
    f"CREATE SCHEMA {SCHEMA}",
    f"CREATE TABLE {SCHEMA}.account (id int8 PRIMARY KEY, name text NOT NULL, active bool NOT NULL)",
    f"""CREATE TABLE {SCHEMA}.post (
        id int8 PRIMARY KEY, author_id int8 NOT NULL REFERENCES {SCHEMA}.account,
        editor_id int8, title text NOT NULL, published bool NOT NULL
    )""",
    f"CREATE TABLE {SCHEMA}.tag (id int4 PRIMARY KEY, label text NOT NULL)",
    f"""CREATE TABLE {SCHEMA}.post_tag (
        post_id int8 REFERENCES {SCHEMA}.post, tag_id int4 REFERENCES {SCHEMA}.tag,
        PRIMARY KEY (post_id, tag_id)
    )""",
]

by_account = Account.col("id").asc()
by_post = Post.col("id").asc()


def names(rows, field="title"):
    return [row[field] for row in rows]


class Construction(unittest.TestCase):
    # [spec:pgorm:req:bindings.relations+2/test]    on_condition joins with the fields' equality, ANDed or ORed
    def test_on_condition_is_part_of_every_join(self):
        self.assertEqual(
            Account.find().join(published).inspect().sql,
            f'SELECT "account"."id", "account"."name", "account"."active" FROM "{SCHEMA}"."account" '
            f'INNER JOIN "{SCHEMA}"."post" ON "account"."id" = "post"."author_id" AND "post"."published" = $1',
        )
        either = Account.has_many(
            Post, from_="id", to="author_id", condition_type="any",
            on_condition=lambda account, post: post.col("title") == "b1",
        )
        self.assertIn(
            'ON "account"."id" = "post"."author_id" OR "post"."title" = $1',
            Account.find().join(either).inspect().sql,
        )
        # The ends answer to the names the join gives them.
        aliased = Post.graph().join_one(active_author, alias="writer").inspect().sql
        self.assertIn('"post"."author_id" = "writer"."id" AND "writer"."active" = $1', aliased)
        self.assertEqual((published.on_condition is not None, published.condition_type), (True, "all"))
        with self.assertRaises(TypeError):
            Account.has_many(Post, from_="id", to="author_id", on_condition="published")
        with self.assertRaises(TypeError):
            Account.has_many(Post, from_="id", to="author_id", condition_type="either")  # type: ignore[arg-type]
        broken = Account.has_many(Post, from_="id", to="author_id", on_condition=lambda a, b: "yes")
        with self.assertRaisesRegex(TypeError, "on_condition gives an expression"):
            Account.find().join(broken)

    # [spec:pgorm:req:bindings.related/test]    find_related's SQL through the junction, and what a path refuses
    def test_related_path_through_a_junction(self):
        self.assertEqual(
            tags.find({"id": 10}).inspect().sql,
            f'SELECT "tag"."id", "tag"."label" FROM "{SCHEMA}"."tag" '
            f'INNER JOIN "{SCHEMA}"."post_tag" ON "post_tag"."tag_id" = "tag"."id" '
            f'INNER JOIN "{SCHEMA}"."post" ON "post"."id" = "post_tag"."post_id" WHERE "post"."id" = $1',
        )
        self.assertIs(tags.from_, Post)
        self.assertIs(tags.to, Tag)
        self.assertEqual((tags.via, tags.relation, tags.cardinality), (post_tags, tag_of, "many"))
        with self.assertRaisesRegex(p.ConstructionError, "via relation starts at post"):
            Tag.related(tag_of, via=post_tags)
        with self.assertRaisesRegex(p.ConstructionError, "where the via relation reaches post_tag"):
            Post.related(author, via=post_tags)
        with self.assertRaisesRegex(p.ConstructionError, "reaches one row"):
            Post.related(PostTag.has_many(Tag, from_="tag_id", to="id"), via=post_tags)
        maybe = Post.graph().related_maybe(tags).inspect().sql
        self.assertIn(
            f'LEFT JOIN "{SCHEMA}"."post_tag" ON "post"."id" = "post_tag"."post_id" '
            f'LEFT JOIN "{SCHEMA}"."tag" ON "post_tag"."tag_id" = "tag"."id"',
            maybe,
        )
        one = Post.graph().related_one(tags).inspect().sql
        self.assertIn(
            f'INNER JOIN "{SCHEMA}"."post_tag" ON "post"."id" = "post_tag"."post_id" '
            f'INNER JOIN "{SCHEMA}"."tag" ON "post_tag"."tag_id" = "tag"."id"',
            one,
        )
        self.assertEqual(
            Post.graph().related_one(author).inspect().sql, Post.graph().join_one(author).inspect().sql
        )

    # [spec:pgorm:req:bindings.linked/test]    each hop under r{i}, walked from the far end, keyed by the start's key
    def test_linked_chain_aliases_every_hop(self):
        chain = Account.linked(posts, post_tags, tag_of)
        self.assertEqual(
            chain.find({"id": 1}).inspect().sql,
            f'SELECT "tag"."id", "tag"."label" FROM "{SCHEMA}"."tag" '
            f'INNER JOIN "{SCHEMA}"."post_tag" AS "r0" ON "r0"."tag_id" = "tag"."id" '
            f'INNER JOIN "{SCHEMA}"."post" AS "r1" ON "r1"."id" = "r0"."post_id" '
            f'INNER JOIN "{SCHEMA}"."account" AS "r2" ON "r2"."id" = "r1"."author_id" WHERE "r2"."id" = $1',
        )
        self.assertEqual([chain.hop(index).qualifier for index in range(3)], ["r0", "r1", "r2"])
        self.assertEqual(chain.last_hop().table.name, "account")
        self.assertEqual((chain.from_, chain.to, chain.relations), (Account, Tag, (posts, post_tags, tag_of)))
        with self.assertRaisesRegex(p.ConstructionError, "no hop 3"):
            chain.hop(3)
        with self.assertRaisesRegex(p.ConstructionError, "relation 1 starts at post_tag, where the one before it reaches post"):
            Account.linked(posts, tag_of)
        with self.assertRaisesRegex(p.ConstructionError, "first relation starts at post"):
            Account.linked(post_tags)
        with self.assertRaises(p.ConstructionError):
            Account.linked()

    # [spec:pgorm:req:bindings.relations+2/test]    from_alias starts a join from the table answering to the alias
    def test_from_alias_names_where_a_join_starts(self):
        shared = (
            Post.find()
            .join(post_tags, alias="mine")
            .join(tag_of.from_alias("mine"))
            .join(tag_post_tags, alias="theirs")
            .join(post_tag_post.from_alias("theirs"), alias="other")
        )
        self.assertIn(
            f'INNER JOIN "{SCHEMA}"."post" AS "other" ON "theirs"."post_id" = "other"."id"',
            shared.inspect().sql,
        )
        with self.assertRaisesRegex(p.ConstructionError, "reads no table called 'nowhere'"):
            Post.find().join(tag_of.from_alias("nowhere"))
        with self.assertRaisesRegex(p.ConstructionError, "'post' is post, where the relation starts at post_tag"):
            Post.find().join(tag_of.from_alias("post"))
        with self.assertRaisesRegex(p.ConstructionError, "takes no from_"):
            Post.graph().via(post_tags, alias="mine").join_one(tag_of.from_alias("mine"), from_=0)

    # [spec:pgorm:req:bindings.graphs+2/test]    a filtered join's condition lands in its ON, given both ends
    def test_join_maybe_filtered_narrows_the_on(self):
        graph = Account.graph().join_maybe_filtered(
            posts, lambda account, post: post.col("published") == True  # noqa: E712
        )
        self.assertIn(
            f'LEFT JOIN "{SCHEMA}"."post" ON "account"."id" = "post"."author_id" AND "post"."published" = $1',
            graph.inspect().sql,
        )
        with self.assertRaises(TypeError):
            Account.graph().join_maybe_filtered(posts, Post.col("published") == True)  # type: ignore[arg-type]  # noqa: E712

    # [spec:pgorm:req:bindings.cursors+2/test]    cursor_on orders by a slot; its key and tiebreaks
    def test_cursor_on_orders_by_a_slot(self):
        cursor = Post.graph().join_one(author).cursor_on(1, "name")
        self.assertTrue(
            cursor.first(2).inspect().sql.endswith(
                'ORDER BY "account"."name" ASC, "account"."id" ASC, "post"."id" ASC LIMIT $1'
            )
        )
        with self.assertRaisesRegex(p.ConstructionError, "cursor orders by the root's"):
            Post.graph().join_one(author).cursor_on(0, "title")
        with self.assertRaisesRegex(p.ConstructionError, "no source 2"):
            Post.graph().join_one(author).cursor_on(2, "name")

    # [spec:pgorm:req:bindings.cursor-secondary-order/test]    the column after the key, in the SQL and the boundary
    def test_secondary_ordering_follows_the_key(self):
        secondary = Post.find().cursor("author_id").add_secondary_order_by(Post.col("id"))
        self.assertEqual(
            secondary.after_with(1, 10).first(2).inspect().sql,
            f'SELECT "post"."id", "post"."author_id", "post"."editor_id", "post"."title", "post"."published" '
            f'FROM "{SCHEMA}"."post" WHERE ("post"."author_id" = $1 AND "post"."id" > $2) OR "post"."author_id" > $3 '
            'ORDER BY "post"."author_id" ASC, "post"."id" ASC LIMIT $4',
        )
        with self.assertRaisesRegex(p.ConstructionError, "does not match 1 key column"):
            secondary.after(1, 10)
        with self.assertRaises(TypeError):
            secondary.add_secondary_order_by("id")  # type: ignore[arg-type]


class Reads(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=2)
        for statement in DDL:
            await self.pool.execute(p.RawSQL(statement))
        await Account.insert_many(
            [
                {"id": 1, "name": "Ann", "active": True},
                {"id": 2, "name": "Bob", "active": False},
                {"id": 3, "name": "Cy", "active": True},
            ]
        ).execute(self.pool)
        await Post.insert_many(
            [
                {"id": 10, "author_id": 1, "editor_id": 2, "title": "a1", "published": True},
                {"id": 11, "author_id": 1, "editor_id": None, "title": "a2", "published": False},
                {"id": 12, "author_id": 2, "editor_id": 999, "title": "b1", "published": True},
                {"id": 13, "author_id": 1, "editor_id": None, "title": "a3", "published": True},
            ]
        ).execute(self.pool)
        await Tag.insert_many([{"id": 1, "label": "red"}, {"id": 2, "label": "blue"}]).execute(self.pool)
        await PostTag.insert_many(
            [{"post_id": 10, "tag_id": 1}, {"post_id": 10, "tag_id": 2}, {"post_id": 12, "tag_id": 2}]
        ).execute(self.pool)
        self.accounts = await Account.find().order_by(by_account).all(self.pool)
        self.posts = await Post.find().order_by(by_post).all(self.pool)

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL(f"DROP SCHEMA {SCHEMA} CASCADE"))
        await self.pool.close()

    # [spec:pgorm:req:bindings.relations+2/test]    find and load read a relation whole, its on_condition included
    async def test_relation_condition_reads_both_ends(self):
        ann = self.accounts[0]
        self.assertEqual(names(await published.find(ann).order_by(by_post).all(self.pool)), ["a1", "a3"])
        self.assertIn('WHERE "account"."id" = $2', published.find(ann).inspect().sql)
        loaded = await published.load(self.pool, self.accounts, Post.find().order_by(by_post))
        self.assertEqual([names(rows) for rows in loaded], [["a1", "a3"], ["b1"], []])
        writers = await active_author.load(self.pool, self.posts)
        self.assertEqual([None if row is None else row["name"] for row in writers], ["Ann", "Ann", None, "Ann"])
        either = Account.has_many(
            Post, from_="id", to="author_id", condition_type="any",
            on_condition=lambda account, post: post.col("title") == "b1",
        )
        files = await either.load(self.pool, self.accounts, Post.find().order_by(by_post))
        self.assertEqual([names(rows) for rows in files], [["a1", "a2", "b1", "a3"], ["b1"], ["b1"]])

    # [spec:pgorm:req:bindings.relations+2/test]    a load takes a query of the far end, filtered and ordered
    async def test_load_reads_a_far_end_query(self):
        newest = Post.find().where(Post.col("published") == True).order_by(Post.col("id").desc())  # noqa: E712
        loaded = await posts.load(self.pool, self.accounts, newest)
        self.assertEqual([names(rows) for rows in loaded], [["a3", "a1"], ["b1"], []])
        partial = await posts.load(self.pool, self.accounts, Post.select("title").order_by(by_post))
        self.assertEqual([[dict(row) for row in rows] for rows in partial][1], [{"title": "b1"}])
        # A key the query filtered away, or nothing matched, reads as None.
        bobs = await author.load(self.pool, self.posts, Account.find().where(Account.col("name") == "Bob"))
        self.assertEqual([None if row is None else row["name"] for row in bobs], [None, None, "Bob", None])
        editors = await editor.load(self.pool, self.posts)
        self.assertEqual([None if row is None else row["name"] for row in editors], ["Bob", None, None, None])
        twice = Account.has_one(Post, from_="id", to="author_id")
        with self.assertRaisesRegex(p.DecodeError, "3 post rows have the key"):
            await twice.load(self.pool, self.accounts)
        with self.assertRaisesRegex(p.ConstructionError, "load reads post, and the query reads tag"):
            await posts.load(self.pool, self.accounts, Tag.find())
        with self.assertRaises(TypeError):
            await posts.load(self.pool, self.accounts, Post)  # type: ignore[arg-type]
        self.assertEqual(await posts.load(self.pool, [], newest), [])

    # [spec:pgorm:req:bindings.related/test]    a path finds and loads through its junction
    async def test_related_path_finds_and_loads(self):
        a1 = self.posts[0]
        self.assertEqual(
            names(await tags.find(a1).order_by(Tag.col("label").asc()).all(self.pool), "label"), ["blue", "red"]
        )
        loaded = await tags.load(self.pool, self.posts, Tag.find().order_by(Tag.col("label").desc()))
        self.assertEqual([names(rows, "label") for rows in loaded], [["red", "blue"], [], ["blue"], []])
        everything = await tags.load(self.pool, self.posts)
        self.assertEqual([len(rows) for rows in everything], [2, 0, 1, 0])
        rows = await Post.graph().related_maybe(tags).order_by(by_post, Tag.col("id").asc()).all(self.pool)
        self.assertEqual(
            [(post["title"], None if tag is None else tag["label"]) for post, tag in rows],
            [("a1", "red"), ("a1", "blue"), ("a2", None), ("b1", "blue"), ("a3", None)],
        )
        required = await Post.graph().related_one(tags).order_by(by_post, Tag.col("id").asc()).all(self.pool)
        self.assertEqual([(post["title"], tag["label"]) for post, tag in required], [("a1", "red"), ("a1", "blue"), ("b1", "blue")])

    # [spec:pgorm:req:bindings.linked/test]    a chain reads from a row, passing one table twice
    async def test_linked_chain_reads_along_its_hops(self):
        chain = Account.linked(posts, post_tags, tag_of)
        found = await chain.find(self.accounts[0]).order_by(Tag.col("id").asc()).all(self.pool)
        self.assertEqual(names(found, "label"), ["red", "blue"])
        titled = chain.find(self.accounts[0]).where(chain.hop(1).col("title") == "a1")
        self.assertEqual(len(await titled.all(self.pool)), 2)
        self.assertEqual(await chain.find({"id": None}).all(self.pool), [])
        # From a tag to the tags sharing a post with it: post_tag and tag twice each.
        neighbours = Tag.linked(tag_post_tags, post_tag_post, post_tags, tag_of)
        reached = await neighbours.find({"id": 1}).order_by(Tag.col("id").asc()).all(self.pool)
        self.assertEqual(names(reached, "label"), ["red", "blue"])
        everyone = await chain.find_linked().all(self.pool)
        self.assertEqual(sorted(names(everyone, "label")), ["blue", "blue", "red"])

    # [spec:pgorm:req:bindings.graphs+2/test]    a hop under an alias, the next relation from it, and a filtered slot
    async def test_graph_joins_from_aliases_and_filters_slots(self):
        graph = (
            Post.graph()
            .via(post_tags)
            .via(tag_of)
            .via(tag_post_tags, alias="shared")
            .join_maybe(post_tag_post.from_alias("shared"), alias="sibling")
        )
        graph = graph.where(graph.col(1, "id") != graph.col(0, "id")).order_by(by_post, graph.col(1, "id").asc())
        siblings = await graph.all_grouped(self.pool)
        self.assertEqual([(post["title"], names(others)) for post, others in siblings], [("a1", ["b1"]), ("b1", ["a1"])])
        filtered = await (
            Account.graph()
            .join_maybe_filtered(posts, lambda account, post: post.col("published") == True)  # noqa: E712
            .order_by(by_account, by_post)
            .all_grouped(self.pool)
        )
        self.assertEqual([(account["name"], names(rows)) for account, rows in filtered], [
            ("Ann", ["a1", "a3"]), ("Bob", ["b1"]), ("Cy", []),
        ])

    # [spec:pgorm:req:bindings.cursors+2/test]    a cursor pages by a slot's field, and resumes inside a key's run
    # [spec:pgorm:req:bindings.cursor-secondary-order/test]    a whole-key boundary resumes inside the key's run
    async def test_cursors_by_slot_and_secondary_ordering(self):
        cursor = Post.graph().join_one(author).cursor_on(1, "name")
        first = await cursor.first(2).all(self.pool)
        self.assertEqual([(post["title"], account["name"]) for post, account in first], [("a1", "Ann"), ("a2", "Ann")])
        past_ann = await cursor.after("Ann").all(self.pool)
        self.assertEqual([post["title"] for post, _ in past_ann], ["b1"])
        inside = await cursor.after_with("Ann", 1, 11).all(self.pool)
        self.assertEqual([post["title"] for post, _ in inside], ["a3", "b1"])
        secondary = Post.find().cursor("author_id").add_secondary_order_by(Post.col("id"))
        self.assertEqual(names(await secondary.after_with(1, 10).first(2).all(self.pool)), ["a2", "a3"])
        self.assertEqual(names(await secondary.after(1).all(self.pool)), ["b1"])
        on_graph = Account.graph().join_maybe(posts).cursor("id").add_secondary_order_by(Post.col("title"))
        rows = await on_graph.after_with(1, 10, "a1").first(3).all(self.pool)
        self.assertEqual([(account["name"], post and post["title"]) for account, post in rows], [
            ("Ann", "a2"), ("Ann", "a3"), ("Bob", "b1"),
        ])


if __name__ == "__main__":
    unittest.main()
