// Example 11 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, copyFrom, copyTo } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const bakeries = copyFrom("bakery", "binary").columns(["name", "city"]);
  const columns = await pool.copyInColumns(bakeries);
  assert.deepEqual(columns.map(({ name, type }) => `${name} ${type.schema}.${type.name}`), [
    "name pg_catalog.varchar",
    "city pg_catalog.varchar",
  ]);

  const sink = await pool.copyInRows(bakeries);
  await sink.send(["Daybreak", "Trondheim"]);

  // A row of the wrong width is refused, and the sink goes on taking rows.
  await assert.rejects(sink.send(["Dusk"]), ConstructionError);
  assert.equal(await sink.finish(), 1);

  const rows = [];
  for await (const row of await pool.copyOutRows(copyTo("bakery", "binary"))) rows.push(row);
  assert.deepEqual(rows[2], [3, "Daybreak", "Trondheim"]);
}
