// Example 5 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, col, DatabaseError, func, select, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = new Table("cake");
  const titled = select(func("initcap", col("name")).as("title")).from(cake).where(col("id").eq(4));
  assert.equal(titled.inspect().sql, `SELECT initcap("name") AS "title" FROM "cake" WHERE "id" = $1`);
  assert.equal((await pool.one(titled)).title, "Vanilla Sponge");

  assert.equal(select(func("Initcap", "x")).inspect().sql, `SELECT "Initcap"($1)`);
  await assert.rejects(
    pool.one(select(func("Initcap", "x").as("t"))),
    (error) => error instanceof DatabaseError && error.sqlstate === "42883",
  );

  const searched = select(col("name"))
    .from(cake)
    .where(call("to_tsvector", col("name")).matches(call("websearch_to_tsquery", "chocolate")))
    .orderBy(col("name").asc());
  assert.deepEqual((await pool.query(searched)).map((row) => row.name), ["Chocolate Fudge", "Chocolate Torte"]);
}
