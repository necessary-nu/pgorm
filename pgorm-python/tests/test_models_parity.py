"""Parity: every model case below builds the SQL and values the golden file
holds, the file pgorm-napi's Rust test ``models::parity`` builds each case to
with pgorm's own entities, ``SelectGraph``, ``Cursor`` and ``Paginator``, so
both bindings answer to one Rust oracle."""

import json
from pathlib import Path
import unittest

import pgorm as p

GOLDEN = json.loads(
    (
        Path(__file__).resolve().parents[2] / "pgorm-napi" / "tests" / "parity" / "models.json"
    ).read_text()
)

LONG = "column_with_a_name_long_enough_to_need_bounding_in_a_graph_xx"

Account = p.Model(
    "account",
    {
        "id": p.Column("i64", primary_key=True, generated="by_default"),
        "name": p.Column("text"),
        "display_name": p.Column("text", nullable=True),
    },
    schema="app",
)
Post = p.Model(
    "post",
    {"id": p.Column("i64", primary_key=True), "author_id": p.Column("i64"), "title": p.Column("text")},
    schema="app",
)
Tag = p.Model("tag", {"id": p.Column("i32", primary_key=True), "label": p.Column("text")}, schema="app")
PostTag = p.Model(
    "post_tag",
    {"post_id": p.Column("i64", primary_key=True), "tag_id": p.Column("i32", primary_key=True)},
    schema="app",
)
Version = p.Model(
    "version",
    {
        "doc": p.Column("i32", primary_key=True),
        "rev": p.Column("i32", primary_key=True),
        "body": p.Column("text"),
    },
    schema="app",
)
Ticket = p.Model(
    "ticket",
    {
        "id": p.Column("i64", primary_key=True, generated="by_default"),
        "note": p.Column("text", nullable=True),
    },
)
Long = p.Model("long_names", {"id": p.Column("i32", primary_key=True), LONG: p.Column("text")})
Moody = p.Model(
    "moody",
    {"id": p.Column("i32", primary_key=True), "mood": p.Column(p.TypeName("mood", schema="app"))},
    schema="app",
)

author = Post.belongs_to(Account, from_="author_id", to="id")
posts = Account.has_many(Post, from_="id", to="author_id")
post_tags = Post.has_many(PostTag, from_="id", to="post_id")
tag_of = PostTag.belongs_to(Tag, from_="tag_id", to="id")

CASES = {
    "find": lambda: Account.find().inspect(),
    "find-filtered": lambda: Account.find()
    .where(Account.col("id").gte(10))
    .order_by(Account.col("name").desc())
    .limit(5)
    .offset(10)
    .inspect(),
    "select-fields": lambda: Account.select("id", "name").inspect(),
    "find-by-key": lambda: Account.find_by_key({"id": 7}).inspect(),
    "find-by-composite-key": lambda: Version.find_by_key({"doc": 1, "rev": 2}).inspect(),
    "find-joined": lambda: Post.find().join(author).where(Account.col("name").eq("Ann")).inspect(),
    "enum-comparison": lambda: Moody.find().where(Moody.col("mood").eq("calm")).inspect(),
    "insert": lambda: Account.insert({"name": "Ann", "display_name": None}).inspect(),
    "insert-many": lambda: Account.insert_many([{"name": "Ann"}, {"name": "Bob"}]).inspect(),
    "insert-defaults": lambda: Ticket.insert({}).inspect(),
    "insert-returning": lambda: Account.insert({"display_name": "A", "name": "Ann"})
    .returning("id", "name")
    .inspect(),
    "update": lambda: Account.update({"name": "x"}).where(Account.key({"id": 1})).inspect(),
    "delete": lambda: Account.delete().where(Account.col("name").eq("x")).inspect(),
    "update-changes": lambda: Account.update({"name": "x"})
    .where(Account.col("id").gte(2))
    .inspect(terminal="returning_changes"),
    "insert-upserts": lambda: Account.insert({"id": 1, "name": "Ann"})
    .on_conflict(p.Conflict.on("id").update("name"))
    .inspect(terminal="returning_upsert"),
    "relation-find": lambda: posts.find({"id": 3}).inspect(),
    "graph": lambda: Post.graph().join_one(author).inspect(),
    "graph-maybe-alias": lambda: Account.graph().join_maybe(posts, alias="p").inspect(),
    "graph-via": lambda: Post.graph().via(post_tags).join_maybe(tag_of).inspect(),
    "graph-filtered": lambda: Post.graph()
    .join_one(author)
    .where(Post.col("title").eq("x"))
    .order_by(Post.col("id").asc())
    .inspect(),
    "graph-long-name": lambda: Long.graph().inspect(),
    "graph-grouped": lambda: Account.graph()
    .join_maybe(posts)
    .order_by(Account.col("name").desc())
    .inspect(terminal="all_grouped"),
    "cursor-after-first": lambda: Account.find().cursor("id").after(5).first(10).inspect(),
    "cursor-composite-last-desc": lambda: Account.find()
    .cursor("name", "id")
    .before("m", 4)
    .last(3)
    .desc()
    .inspect(),
    "graph-cursor-with": lambda: Post.graph()
    .join_maybe(author)
    .cursor("title")
    .after_with("t", 1, 2)
    .first(2)
    .inspect(),
    "page": lambda: Account.find().order_by(Account.col("id").asc()).paginate(10).inspect(2),
    "count": lambda: Account.find()
    .where(Account.col("name").ne("x"))
    .order_by(Account.col("id").asc())
    .limit(5)
    .inspect(terminal="count"),
}


def text(value):
    if value is None:
        return "null"
    if isinstance(value, list):
        return "[" + ",".join(text(item) for item in value) + "]"
    return str(value)


def canonical(value):
    return [value.kind, text(value.value)]


class ModelParity(unittest.TestCase):
    # [spec:pgorm:req:bindings.models+5/test]
    def test_the_cases_are_the_golden_files(self):
        self.assertEqual(sorted(CASES), sorted(GOLDEN))

    # [spec:pgorm:req:bindings.model-reads/test]
    # [spec:pgorm:req:bindings.model-writes+2/test]
    # [spec:pgorm:req:bindings.relations/test]
    # [spec:pgorm:req:bindings.graphs/test]
    # [spec:pgorm:req:bindings.cursors/test]
    # [spec:pgorm:req:bindings.pagination/test]
    def test_each_case_builds_what_pgorm_builds(self):
        for name, build in CASES.items():
            with self.subTest(case=name):
                compiled = build()
                self.assertEqual(
                    {"sql": compiled.sql, "values": [canonical(value) for value in compiled.params]},
                    GOLDEN[name],
                )


if __name__ == "__main__":
    unittest.main()
