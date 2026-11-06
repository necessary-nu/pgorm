// Example 6 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { copyTo, select, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = new Table("cake");
  const priceList = select(cake.col("name"), cake.col("price"))
    .from(cake)
    .where(cake.col("bakery_id").eq(1))
    .orderBy(cake.col("name").asc());
  const copy = copyTo(priceList, "csv").header();
  assert.equal(
    copy.inspect().sql,
    `COPY (SELECT "cake"."name", "cake"."price" FROM "cake" WHERE "cake"."bakery_id" = 1 ORDER BY "cake"."name" ASC) TO STDOUT WITH (FORMAT csv, HEADER true)`,
  );

  const decoder = new TextDecoder();
  let csv = "";
  for await (const chunk of await pool.copyOut(copy)) csv += decoder.decode(chunk, { stream: true });
  csv += decoder.decode();
  assert.equal(csv, "name,price\nChocolate Fudge,4.50\nChocolate Torte,6.00\nLemon Drizzle,3.20\n");
}
