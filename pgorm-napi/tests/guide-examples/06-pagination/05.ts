// Example 5 of guide/06-pagination.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const feed = Cake.select("name").cursor("price", "id").after(new Decimal("3.20"), 2).first(2);
  assert.equal(
    feed.inspect().sql,
    `SELECT "cake"."name" FROM "cake" WHERE ("cake"."price" = $1 AND "cake"."id" > $2) OR "cake"."price" > $3 ORDER BY "cake"."price" ASC, "cake"."id" ASC LIMIT $4`,
  );
}
