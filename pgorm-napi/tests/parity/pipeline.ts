// The pipeline family's parity cases, held to `pipeline.json` as `select.ts`'s
// are to theirs: the Rust test builds each with pgorm::pipeline directly,
// every value a binder binds there bound by the same stage here.
// [spec:pgorm:req:bindings.pipeline+1/test]
// [spec:pgorm:req:bindings.pipeline-expressions+3/test]
// [spec:pgorm:req:bindings.pipeline-binder/test]

import { type Builder, pipeline as pl, Table, TypeName, Value } from "../../lib/index.js";

const items = new Table("items");
const base = pl.from(items);
const id = pl.col("items", "id");
const category = pl.col("items", "category");
const amount = pl.col("items", "amount");
const total = pl.alias("total");
const rank = pl.alias("row_rank");

const left = base.filterWith((b) => id.gt(b.bind(new Value(1, "i32")))).select(id, category, amount);
const right = base.filterWith((b) => id.lt(b.bind(new Value(8, "i32")))).select(id, category, amount);

export const cases: Record<string, () => Builder> = {
  "literal": () => base.filter(id.gt(pl.literal(2))).select(id, amount).sort(id.desc()).take(3),
  "bound-operand": () => base.filter(id.gt(2).and(category.ne("O'Brien; -- 雪"))).select(id, amount),
  "literal-string": () => base.filter(category.eq(pl.literal("O'Brien; -- \\ 雪"))),
  "repeated": () =>
    base.filterWith((b) => {
      b.bind("unused");
      const value = b.bind(new Value(2, "i32"));
      return id.gt(value).and(id.lt(value.add(10)));
    }),
  "derive": () => base.derive(amount.add(pl.literal(2)).as(total)).filter(total.gt(5)),
  "derive-with": () => base.deriveWith((b) => [amount.add(b.bind(2)).as(total)]),
  "select-with": () => base.selectWith((b) => [id, b.bind("payload").as("payload")]),
  "group": () => base.group(category).aggregate(pl.sum(amount).as(total), pl.countRows().as("n")).filter(total.gt(3)),
  "group-with": () =>
    base.groupWith((b) => [category.coalesce(b.bind("missing"))])
      .aggregateWith((b) => [pl.sum(amount).add(b.bind(1)).as(total)]),
  "window": () =>
    base.window(pl.over().by(category).sortBy(id).rows(null, 0), pl.rowNumber().as(rank)).filter(
      rank.lte(pl.literal(2)),
    ),
  "window-with": () =>
    base.windowWith(pl.over().sortBy(id).range(-1, 1), (b) => pl.sum(amount).add(b.bind(2)).as(total)),
  "window-functions": () =>
    base.window(
      pl.over().by(category).sortBy(amount.desc()),
      pl.rank(amount).as("r"),
      pl.rankDense(amount).as("d"),
      pl.lag(1, amount).as("before"),
      pl.lead(2, amount).as("after"),
      pl.first(amount).as("lowest"),
      pl.last(amount).as("highest"),
    ),
  "sort-with": () => base.sortWith((b) => [id.add(b.bind(1)), amount.desc()]).takeRange(2, 4),
  "join": () =>
    base.join(pl.source(items).named("peer"), id.eq(pl.col("peer", "id")), { kind: "left" }).select(
      id,
      pl.col("peer", "amount").as("peer_amount"),
    ),
  "join-with": () =>
    base.joinWith(pl.source(right).named("peer"), (b) => id.eq(pl.col("peer", "id")).and(amount.gt(b.bind(1)))),
  "join-roles": () =>
    pl.from(left).join(right, pl.thisColumn("id").eq(pl.thatColumn("id")), { kind: "full" }),
  "append": () => left.append(right),
  "intersect": () => left.intersect(right),
  "remove": () => left.remove(right),
  "embedded": () => pl.from(left).filter(pl.alias("id").gt(5)),
  "distinct": () => base.select(category).distinct(),
  "schema-source": () =>
    pl.from(new Table("items", { schema: "app" })).join(
      new Table("items", { schema: "app", alias: "other" }),
      id.eq(pl.col("other", "id")),
    ),
  "aggregates": () =>
    base.group(category).aggregate(
      pl.min(amount).as("low"),
      pl.max(amount).as("high"),
      pl.average(amount).as("mean"),
      pl.stddev(amount).as("spread"),
      pl.count(amount).as("counted"),
      pl.countDistinct(amount).as("kinds"),
    ),
  "expressions": () =>
    base.derive(
      pl.caseWhen([[amount.gt(10), pl.literal("big")], [amount.isNull(), pl.literal("none")]], "small").as("size"),
      amount.cast("text").as("spelled"),
      amount.sub(1).mul(2).div(3).rem(4).neg().as("arith"),
      category.inArray(["a", pl.literal("b")]).not().as("other"),
      id.isNotNull().or(id.gte(0)).as("present"),
      amount.coalesce(pl.literal(null)).as("filled"),
    ),
  "compares-as": () =>
    base.filter(
      category.comparesAs(new TypeName("Mood", { schema: "app" })).inArray(["glad", pl.literal("calm")])
        .and(pl.alias("m").comparesAs("mood").lt("glad")),
    ),
};
