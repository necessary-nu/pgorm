// Example 19 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { any, column, DataType, fromModel, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const days = new DataType("bit", { length: 7 });
  const Opening = model("opening", {
    renameAll: "snake_case",
    columns: {
      bakeryId: column("i32", { primaryKey: true }),
      days: column("i64", { columnType: days, selectAs: "bigint", saveAs: days }),
    },
  });
  await pool.execute(fromModel(Opening).table);

  // Monday to Saturday.
  const sunrise = await Opening.insert({ bakeryId: 1, days: 0b111_1110n }).returning().one(pool);
  assert.equal(sunrise.days, 0b111_1110n);

  const everyDay = Opening.find().where(Opening.col("days").eq(0b111_1111n));
  assert.equal(
    everyDay.inspect().sql,
    `SELECT "opening"."bakery_id", CAST("opening"."days" AS bigint) AS "days" FROM "opening" WHERE "opening"."days" = CAST($1::int8 AS bit(7))`,
  );
  assert.deepEqual(await everyDay.all(pool), []);

  const someDays = Opening.find().where(Opening.col("days").eq(any([0b111_1110n, 0b000_0011n])));
  assert.match(someDays.inspect().sql, /WHERE "opening"\."days" = ANY\(CAST\(\$1::int8\[\] AS bit\(7\)\[\]\)\)$/);
  assert.deepEqual((await someDays.all(pool)).map((row) => row.bakeryId), [1]);
}
