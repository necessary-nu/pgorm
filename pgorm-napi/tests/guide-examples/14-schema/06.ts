// Example 6 of guide/14-schema.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { alterType, createType } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const boxSize = createType("box_size").values(["small", "large"]);
  assert.equal(boxSize.inspect().sql, `CREATE TYPE "box_size" AS ENUM ('small', 'large')`);
  await pool.execute(boxSize);

  const medium = alterType("box_size").addValue("medium", { before: "large" });
  assert.equal(medium.inspect().sql, `ALTER TYPE "box_size" ADD VALUE 'medium' BEFORE 'large'`);
  await pool.execute(medium);

  const address = createType("address").attribute("street", "text").attribute("city", "text");
  assert.equal(address.inspect().sql, `CREATE TYPE "address" AS ("street" text, "city" text)`);
  await pool.execute(address);

  const labels = await pool.one("SELECT enum_range(NULL::box_size)::text AS labels");
  assert.equal(labels.labels, "{small,medium,large}");
}
