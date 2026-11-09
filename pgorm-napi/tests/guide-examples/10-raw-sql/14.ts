// Example 14 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { prql } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const { sql, values } = prql(
    "from cake\nfilter bakery_id == 1\nremove (from cake | filter gluten_free)\nselect {name}",
  );
  assert.equal(
    sql,
    "WITH table_1 AS (SELECT * FROM cake WHERE bakery_id = 1 " +
      "EXCEPT ALL SELECT * FROM cake WHERE gluten_free) SELECT name FROM table_1",
  );
  assert.deepEqual(await pool.query(sql, values), [{ name: "Chocolate Fudge" }]);
}
