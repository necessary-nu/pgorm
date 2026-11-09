// Example 5 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, Decimal, fromModel, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const TipJar = model("tip_jar", {
    columns: {
      id: column("i32", { primaryKey: true, autoIncrement: true }),
      amount: column("decimal", { columnType: "money" }),
    },
  });
  await pool.execute(fromModel(TipJar).table);

  const tips = await TipJar.insert({ amount: new Decimal("12.345") }).returning().one(pool);
  // Rounded to the currency's two places.
  assert.equal(String(tips.amount), "12.35");

  const query = TipJar.find().where(TipJar.col("amount").gt(new Decimal("10")));
  assert.equal(
    query.inspect().sql,
    `SELECT "tip_jar"."id", CAST("tip_jar"."amount" AS numeric) AS "amount" FROM "tip_jar" WHERE "tip_jar"."amount" > CAST($1::numeric AS money)`,
  );
  assert.equal((await query.all(pool)).length, 1);
}
