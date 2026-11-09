// Example 13 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { CopyColumn, copyFrom, CopyRowDecoder, CopyRowEncoder, copyTo } from "@necessary/pgorm";
import { Bakery } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const columns = [new CopyColumn("name", "varchar"), new CopyColumn("city", "varchar")];
  const encoder = new CopyRowEncoder(columns);
  const data = [
    CopyRowEncoder.header(),
    encoder.encode(["Noon", "Stavanger"]),
    encoder.encode(["Night Owl", "Tromsø"]),
    CopyRowEncoder.trailer(),
  ];
  assert.equal(await pool.copyIn(copyFrom("bakery", "binary").columns(["name", "city"]), data), 2);

  const decoder = new CopyRowDecoder(columns);
  const rows = [];
  for await (const chunk of await pool.copyOut(copyTo(Bakery.select("name", "city").statement, "binary"))) {
    decoder.push(chunk);
    for (let row = decoder.nextRow(); row !== null; row = decoder.nextRow()) rows.push(row);
  }
  decoder.finish();
  assert.equal(rows.length, 4);
  assert.deepEqual(rows[3], ["Night Owl", "Tromsø"]);
}
