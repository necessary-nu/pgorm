// Example 17 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute(
    "CREATE TABLE till (id serial PRIMARY KEY, address inet NOT NULL, hardware macaddr NOT NULL, receipt_logo bytea NOT NULL)",
  );
  const Till = model("till", {
    columns: {
      id: column("i32", { primaryKey: true, default: true }),
      address: column("ipnetwork"),
      hardware: column("mac_address"),
      receiptLogo: column("bytes", { name: "receipt_logo" }),
    },
  });

  const front = await Till.insert({
    address: "10.0.4.17/24",
    hardware: new Uint8Array([0x02, 0x00, 0x5e, 0x10, 0x00, 0x01]),
    receiptLogo: new Uint8Array([0x89, 0x50, 0x4e, 0x47]),
  }).returning().one(pool);

  assert.equal(front.address, "10.0.4.17/24");
  assert.deepEqual([...front.hardware], [0x02, 0x00, 0x5e, 0x10, 0x00, 0x01]);
  assert.deepEqual([...front.receiptLogo], [0x89, 0x50, 0x4e, 0x47]);
}
