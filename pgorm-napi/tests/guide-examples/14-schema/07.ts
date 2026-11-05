// Example 7 of guide/14-schema.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { createDomain, createSchema, createSequence, domainValue, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const archive = createSchema("archive", { ifNotExists: true });
  assert.equal(archive.inspect().sql, `CREATE SCHEMA IF NOT EXISTS "archive"`);
  await pool.execute(archive);

  const receiptNo = createSequence(new Table("receipt_no", { schema: "archive" }))
    .asType("bigint")
    .options({ startWith: 1000 });
  assert.equal(receiptNo.inspect().sql, `CREATE SEQUENCE "archive"."receipt_no" AS bigint START WITH 1000`);
  await pool.execute(receiptNo);
  const next = await pool.one("SELECT nextval('archive.receipt_no') AS next");
  assert.equal(next.next, 1000n);

  const percent = createDomain("percent", "smallint").check(domainValue().between(0, 100));
  assert.equal(percent.inspect().sql, `CREATE DOMAIN "percent" AS smallint CHECK (VALUE BETWEEN 0 AND 100)`);
  await pool.execute(percent);
}
