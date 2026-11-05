// Example 15 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { bind, jsonTable, JsonTableColumn, select } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const incoming = {
    customer: "kim@example.com",
    lines: [{ cake: 2, qty: 3 }, { cake: 4, qty: 1 }],
  };

  const line = jsonTable(bind(incoming), "$.lines[*]", [
    JsonTableColumn.value("cake_id", "integer", { path: "$.cake" }),
    JsonTableColumn.value("quantity", "integer", { path: "$.qty" }),
  ], { alias: "line" });
  const query = select().from(line);

  assert.equal(
    query.inspect().sql,
    [
      `SELECT * FROM JSON_TABLE($1::jsonb, '$.lines[*]' COLUMNS`,
      `("cake_id" integer PATH '$.cake', "quantity" integer PATH '$.qty')) AS "line"`,
    ].join(" "),
  );
  assert.deepEqual(await pool.query(query), [
    { cake_id: 2, quantity: 3 },
    { cake_id: 4, quantity: 1 },
  ]);
}
