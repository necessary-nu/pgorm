// Example 8 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, DecodeError, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const PriceList = model("cake", {
    columns: { name: column("text"), priceInCents: column("i64", { name: "price_in_cents" }) },
  });

  await assert.rejects(
    pool.verify("SELECT name, price FROM cake WHERE bakery_id = $1", PriceList),
    (error) =>
      error instanceof DecodeError &&
      error.message === "`cake` reads column `price_in_cents`, which the statement does not return; it returns: name, price",
  );
  await assert.rejects(
    pool.verify("SELECT name, price AS price_in_cents FROM cake WHERE bakery_id = $1", PriceList),
    (error) =>
      error instanceof DecodeError &&
      error.message === "`cake` decodes column `price_in_cents` as `i64`, which cannot read PostgreSQL type `numeric`",
  );

  await pool.verify("SELECT name, (price * 100)::bigint AS price_in_cents FROM cake WHERE bakery_id = $1", PriceList);
}
