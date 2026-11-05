// Example 4 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, caseWhen, col, Condition, likeExpr, select, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const query = select(
    col("name"),
    call("lower", col("name")).as("key"),
    caseWhen(col("price").gte(5), "premium").when(col("price").gte(3), "standard").else("budget").as("tier"),
  )
    .from(new Table("cake"))
    .where(Condition.any(
      col("name").like(likeExpr("%Choc%")),
      Condition.all(col("gluten_free").eq(true), col("price").between(2, 4)),
    ))
    .orderBy(col("name").asc());

  assert.equal(
    query.inspect().sql,
    [
      `SELECT "name", LOWER("name") AS "key",`,
      `(CASE WHEN ("price" >= $1) THEN $2 WHEN ("price" >= $3) THEN $4 ELSE $5 END) AS "tier"`,
      `FROM "cake"`,
      `WHERE "name" LIKE $6 OR ("gluten_free" = $7 AND "price" BETWEEN $8 AND $9)`,
      `ORDER BY "name" ASC`,
    ].join(" "),
  );

  const tiers = await pool.query(query);
  assert.deepEqual(
    tiers.map((row) => [row.name, row.tier]),
    [
      ["Chocolate Fudge", "standard"],
      ["Chocolate Torte", "premium"],
      ["Lemon Drizzle", "standard"],
      ["Vanilla Sponge", "budget"],
    ],
  );
}
