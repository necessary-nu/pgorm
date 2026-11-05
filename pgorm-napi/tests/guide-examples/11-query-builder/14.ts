// Example 14 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { bind, call, select, Table, tableFunction } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = new Table("cake");
  const wanted = tableFunction(call("unnest", bind([4, 1, 3]).cast("integer", { array: true })))
    .withOrdinality()
    .columns("id", "position")
    .alias("wanted");
  const query = select(cake.col("name"))
    .from(wanted)
    .join(cake, cake.col("id").eq(wanted.col("id")))
    .orderBy(wanted.col("position").asc());

  assert.equal(
    query.inspect().sql,
    [
      `SELECT "cake"."name" FROM UNNEST(CAST($1::int8[] AS integer[])) WITH ORDINALITY AS "wanted"("id", "position")`,
      `INNER JOIN "cake" ON "cake"."id" = "wanted"."id" ORDER BY "wanted"."position" ASC`,
    ].join(" "),
  );
  assert.deepEqual(await pool.query(query), [
    { name: "Vanilla Sponge" },
    { name: "Chocolate Fudge" },
    { name: "Carrot Cake" },
  ]);
}
