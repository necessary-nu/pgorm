// The SELECT family's parity cases: each builds a statement or expression
// with the JavaScript API, and `select.json` holds the SQL and values it must
// build, which the Rust test `statements::parity` builds the same statement to
// with pgorm-query's own builders.
// [spec:pgorm:req:bindings.select+2/test]
// [spec:pgorm:req:bindings.expressions+2/test]
// [spec:pgorm:req:bindings.quantifiers+2/test]
// [spec:pgorm:req:bindings.cte-options/test]

import {
  all,
  any,
  array,
  bind,
  type Builder,
  call,
  caseOf,
  caseWhen,
  col,
  Condition,
  cube,
  DataType,
  Decimal,
  exists,
  groupingSet,
  groupingSets,
  literal,
  rollup,
  scalar,
  select,
  Table,
  tableFunction,
  tuple,
  TypeName,
  Uuid,
  Value,
  With,
} from "../../lib/index.js";

const account = new Table("account", { schema: "app", alias: "a" });
const event = new Table("event", { alias: "e" });
const t = new Table("t");

export const cases: Record<string, () => Builder> = {
  "select-join-group": () => {
    const count = call("count", event.col("id"));
    return select(account.col("name"), count.as("events"))
      .from(account)
      .join(event, account.col("id").eq(event.col("account_id")), { kind: "left" })
      .where(
        Condition.all(
          account.col("active").eq(true),
          Condition.any(event.col("kind").eq("click"), event.col("id").isNull()),
        ),
      )
      .groupBy(account.col("name"))
      .having(count.gt(2))
      .orderBy(count.desc({ nulls: "last" }), account.col("name").asc({ nulls: "first" }))
      .limit(5)
      .offset(10n);
  },
  "select-star-distinct": () => select().from(t).distinct().select(t.star(), col("id").as("key")),
  "joins-every-kind": () =>
    select()
      .from(t)
      .join(new Table("u"), col("a", { table: "t" }).eq(col("a", { table: "u" })))
      .join(new Table("v", { schema: "s" }), col("b", { table: "v", schema: "s" }).gt(1), { kind: "right" })
      .join(new Table("w"), Condition.any(), { kind: "full" })
      .crossJoin(new Table("x")),
  "operators": () =>
    select(
      col("a").add(1).mul(2),
      col("b").concat("x"),
      col("c").mod(3),
      col("d").div(bind(new Value(2, "i32"))).sub(1.5),
      col("e").ne(col("f")).and(col("g").lte(4)).or(col("h").gte(5).not()),
      col("i").isDistinctFrom(1),
      col("j").isNotDistinctFrom("k"),
    ),
  "membership": () =>
    select().from(t).where(
      Condition.all(
        col("id").isIn([]),
        col("id").isNotIn([]),
        col("id").isIn([1, 2]),
        col("kind").isNotIn(["a"]),
        tuple(col("a"), col("b")).isIn([tuple(1, "x")]),
        col("id").isIn(select(col("id")).from(new Table("u"))),
        col("id").isNotIn(select(col("id")).from(new Table("v"))),
      ),
    ),
  "between": () =>
    select().from(t).where(
      Condition.all(
        col("a").between(1, 10),
        col("b").notBetween(1, 10),
        col("c").between(10, 1, { symmetric: true }),
        col("d").notBetween(10, 1, { symmetric: true }),
      ),
    ),
  "patterns": () =>
    select().from(t).where(
      Condition.any(
        col("a").like("A%"),
        col("b").notLike("50\\%%", { escape: "\\" }),
        col("c").ilike("x_y"),
        col("d").notIlike("%z", { escape: "!" }),
        col("e").startsWith("50%_"),
        col("f").endsWith("O'Brien"),
        col("g").containsText("\\"),
      ),
    ),
  "regex-and-json-path": () =>
    select().from(t).where(
      Condition.any(
        col("a").regex("^x"),
        col("b").notRegex(col("c")),
        col("d").iregex("y$"),
        col("e").notIregex("z"),
        col("doc").pathExists('$.tags[*] ? (@ == "blue")'),
      ),
    ),
  "matches-and-quantifiers": () => {
    const ids = (name: string) => select(col("id")).from(new Table(name));
    return select().from(t).where(
      Condition.all(
        col("body").matches("fat & rat"),
        col("doc").matches(bind("$.a == 1").cast("jsonpath")),
        col("id").eq(any([1, 2])),
        col("id").ne(all(array(ids("u")))),
        col("score").gt(all(ids("v"))),
        col("id").lte(any(ids("w"))),
        col("a").gte(any(col("b"))),
      ),
    );
  },
  "quantifiers-beyond-comparisons": () => {
    const patterns = select(col("p")).from(new Table("u"));
    return select().from(t).where(
      Condition.all(
        col("a").like(any(["x%", "y%"])),
        col("a").notIlike(all(patterns)),
        col("a").iregex(any(col("b"))),
        col("span").containedBy(any(col("spans"))),
        col("span").adjacentTo(all(patterns)),
        col("doc").pathExists(any(col("paths"))),
        col("body").matches(all(col("queries"))),
      ),
    );
  },
  "casts-collations-subscripts": () =>
    select(
      col("a").cast("integer"),
      col("b").cast(new TypeName("Mood", { schema: "app" }), { array: true }),
      bind("1 day").cast("interval"),
      col("c").collate("C"),
      col("d").collate("de-x-icu", { schema: "pg_catalog" }),
      col("e").at(1),
      col("f").at(1).at(bind(new Value(2, "i32"))),
      col("g").slice(2, 3),
      col("h").slice(null, 3),
      col("i").slice(2, null),
      col("j").slice(null, null),
    ),
  "case": () =>
    select(
      caseWhen(col("x").gt(0), "positive").when(Condition.all(col("x").lt(0), col("y").isNotNull()), "negative"),
      caseWhen(col("x").isNull(), 0).else(col("x")).as("filled"),
      caseOf(col("kind")).when("a", 1).when("b", 2),
      caseOf(col("kind")).when("a", 1).else(-1),
    ),
  "values": () =>
    select(
      bind(new Value(7, "i16")).add(3),
      bind(new Value("calm", new TypeName("mood", { schema: "app" }))),
      bind(Value.array(new TypeName("mood"), ["calm", "glad"])),
      bind(new Decimal("19.9900")),
      bind(new Uuid("0190a7a6-8c8e-7000-8000-000000000001")),
      bind(Temporal.PlainDate.from("2026-10-09")),
      bind(Temporal.Instant.from("2026-10-09T12:00:00Z")),
      bind(true),
      bind([1, 2, 3]),
    ),
  "literals": () =>
    select(
      literal(7).add(3),
      literal("O'Brien"),
      literal(new Value("calm", new TypeName("mood", { schema: "app" }))),
      literal(Value.array(new TypeName("mood"), ["calm", "glad"])),
      literal(new Decimal("19.9900")),
      literal(new Uuid("0190a7a6-8c8e-7000-8000-000000000001")),
      literal(Temporal.PlainDate.from("2026-10-09")),
      literal(true),
      literal([1, 2, 3]),
      literal(Value.null("i32")),
    ).from(t).where(col("name").eq(literal("x")).and(col("id").gt(2))),
  "functions": () =>
    select(
      call("lower", col("a")),
      call("upper", col("a")),
      call("abs", col("b")),
      call("char_length", col("a")),
      call("count_distinct", col("a")),
      call("sum", col("b")),
      call("avg", col("b")),
      call("min", col("b")),
      call("max", col("b")),
      call("round", col("b")),
      call("round", col("b"), bind(new Value(2, "i32"))),
      call("coalesce", col("a"), "none"),
      call("random"),
      call("gen_random_uuid"),
      call("uuidv4"),
      call("uuidv7"),
      call("uuidv7", bind("-1 hour").cast("interval")),
      call("uuid_extract_timestamp", col("u")),
      call("uuid_extract_version", col("u")),
    ),
  "subqueries": () => {
    const latest = select(col("id"), col("at")).from(new Table("event")).where(col("account", { table: "event" }).eq(account.col("id")));
    return select(account.col("id"), scalar(select(call("max", col("at"))).from(new Table("event"))).as("last"))
      .from(account)
      .join(latest.as("l"), Condition.all(), { kind: "left", lateral: true })
      .where(exists(select(bind(1)).from(new Table("flag")).where(col("account").eq(account.col("id")))));
  },
  "array-subquery": () => {
    const events = select(col("id")).from(new Table("event")).where(col("account").eq(account.col("id")));
    return select(account.col("id"), array(events).as("events")).from(account).where(array(events).contains([4n]));
  },
  "from-subquery": () => {
    const inner = select(col("id"), col("kind")).from(t).where(col("kind").ne("x"));
    const sub = inner.as("sub");
    return select(sub.col("id"), sub.star()).from(sub).crossJoin(new Table("u"));
  },
  "set-operations": () =>
    select(col("id"))
      .from(t)
      .union(select(col("id")).from(new Table("u")))
      .unionAll(select(col("id")).from(new Table("v")))
      .intersect(select(col("id")).from(new Table("w")))
      .intersectAll(select(col("id")).from(new Table("x")))
      .except(select(col("id")).from(new Table("y")))
      .exceptAll(select(col("id")).from(new Table("z"))),
  "locks": () =>
    select()
      .from(account)
      .join(event, account.col("id").eq(event.col("account_id")))
      .lock("update", { of: [account], wait: "skipLocked" }),
  "lock-share": () => select().from(t).lock("keyShare", { wait: "nowait" }),
  "lock-no-key": () => select().from(t).lock("noKeyUpdate").lock("share"),
  "with": () => {
    const clause = new With("recent", select(col("id")).from(t).where(col("at").gt(1)), { columns: ["id"], materialized: true })
      .cte("older", select(col("id")).from(new Table("recent")), { materialized: false });
    return select().from(new Table("older")).with(clause);
  },
  "with-recursive": () => {
    const body = select(bind(new Value(1, "i32")).as("n"))
      .unionAll(select(col("n").add(1)).from(new Table("r")).where(col("n").lt(5)));
    return select(col("n")).from(new Table("r")).with(
      With.recursive("r", body, {
        columns: ["n"],
        search: { order: "depth", by: col("n"), set: "ord" },
        cycle: { by: col("n"), set: "looped", using: "path" },
      }),
    );
  },
  "limits-reset": () => select().from(t).limit(3).offset(4).limit(null).offset(null).limit(9007199254740991),
  "conditions": () =>
    select().from(t).where(Condition.all()).where(Condition.any()).having(Condition.all(col("a").eq(1)).not())
      .where(Condition.any(col("b").eq(2)).add(col("c").eq(3)).add(Condition.all(col("d").eq(4), col("e").eq(5)))),
  "tie-limit": () =>
    select(col("id")).from(t).orderBy(col("score").desc()).limitWithTies(5).limit(null).limit(3).limitWithTies(2)
      .offset(1),
  "tie-limit-replaced": () => select(col("id")).from(t).orderBy(col("score").desc()).limitWithTies(2).limit(4),
  "grouping-elements": () =>
    select(col("a"), col("b"), call("count", col("id")))
      .from(t)
      .groupBy(
        col("a"),
        rollup(col("a"), col("b")),
        cube(col("b"), tuple(col("a"), col("b"))),
        groupingSets(groupingSet(col("a"), col("b")), col("b"), groupingSet(), rollup(col("a"))),
      )
      .groupBy(groupingSet(), rollup())
      .groupByDistinct(),
  "group-by-distinct-ungrouped": () => select(col("a")).from(t).groupByDistinct(),
  "similar-to": () =>
    select().from(t).where(
      Condition.all(
        col("a").similarTo("(x|y)%"),
        col("b").notSimilarTo("!%%", { escape: "!" }),
        col("c").similarTo("a\\_b", { escape: "\\" }),
        col("d").similarTo("%z").not(),
      ),
    ),
  "table-function": () => select().from(tableFunction(call("generate_series", 1, 3), { alias: "n" })),
  "table-function-ordinality": () => {
    const tags = tableFunction([call("unnest", account.col("tags"))], {
      alias: "t",
      ordinality: true,
      columns: ["tag", "position"],
    });
    return select(account.col("id"), tags.col("tag"), tags.col("position")).from(account).crossJoin(tags);
  },
  "rows-from": () => {
    const zipped = tableFunction([call("generate_series", 10, 1, -3), call("unnest", bind(["a", "b", "c"]))], {
      alias: "r",
      ordinality: true,
      columns: ["x", "y", "n"],
    });
    const single = tableFunction(call("generate_series", 1, 2), { alias: "s", rowsFrom: true });
    return select(zipped.star(), single.col("s")).from(zipped).crossJoin(single);
  },
  "record-functions": () => {
    const one = tableFunction({
      call: call("json_to_record", col("doc")),
      columnDefinitions: [["a", "integer"], ["tags", new DataType("text").array()]],
    }, { alias: "o" });
    const zipped = tableFunction(
      [{ call: call("jsonb_to_recordset", col("docs")), columnDefinitions: [["b", "text"]] }, call("generate_series", 1, 2)],
      { alias: "z", ordinality: true },
    );
    return select(one.star(), zipped.star()).from(new Table("t")).crossJoin(one).crossJoin(zipped);
  },
  "tablesample": () => {
    const sampled = account.tablesample("system", 12.5, { repeatable: 42 });
    const events = new Table("event").tablesample("bernoulli", 50);
    return select(sampled.col("id"), events.col("kind"))
      .from(sampled)
      .join(events, sampled.col("id").eq(events.col("account_id")), { kind: "left" })
      .lock("share", { of: [sampled] });
  },
  "expression-inspect": () => col("x").add(bind(new Value(7, "i16"))),
  "condition-inspect": () => Condition.any(col("a").eq(1), col("b").isNull()),
};
