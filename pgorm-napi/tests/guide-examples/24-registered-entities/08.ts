// Example 8 of guide/24-registered-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { createDisjoint, createSchema, disjointEntity, entity } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const Shipments = disjointEntity("app.Shipments");
  await pool.execute(createSchema("napi_shipments"));
  for (const statement of createDisjoint(Shipments)) await pool.execute(statement);

  const [base, kind, packet] = await Shipments.insert(
    entity("app.Shipment").active().set("label", "box"),
    "packet",
    entity("app.Packet").active().set("weight_g", 500),
  ).returningModel(pool);
  assert.equal(kind, "packet");
  assert.equal(base.label, "box");
  assert.equal(packet?.weight_g, 500);
}
