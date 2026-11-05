// Example 9 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, Decimal, model, Uuid } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute("CREATE TABLE voucher (code uuid PRIMARY KEY DEFAULT uuidv7(), value numeric NOT NULL)");
  const Voucher = model("voucher", {
    columns: {
      code: column("uuid", { primaryKey: true, default: true }),
      value: column("decimal"),
    },
  });

  const first = await Voucher.insert({ value: new Decimal("10") }).returning("code").one(pool);
  const second = await Voucher.insert({ value: new Decimal("10") }).returning("code").one(pool);

  assert.ok(first.code instanceof Uuid);
  // The version digit, and the order: a later v7 UUID sorts after an earlier one.
  assert.equal(first.code.toString()[14], "7");
  assert.ok(first.code.toString() < second.code.toString());
}
