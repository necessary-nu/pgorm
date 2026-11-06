// Example 5 of guide/09-errors.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, DecodeError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // The parameter is a smallint, and 100000 is not one.
  const tooWide = await pool.execute("SELECT $1::smallint", [100_000]).catch((error: unknown) => error);
  assert.ok(tooWide instanceof ConstructionError);
  assert.match(tooWide.message, /^parameter \$1 cannot be bound/);
  assert.equal(tooWide.parameter, 1);
  assert.equal(tooWide.reason, "value `100000` is out of range for Postgres type `int2`");

  // Two placeholders, and one value.
  const short = await pool.execute("SELECT $1::text, $2::text", ["lemon"]).catch((error: unknown) => error);
  assert.ok(short instanceof ConstructionError);
  assert.equal(short.message, "the statement takes 2 parameters, and 1 was given");
  assert.equal(short.parameter, null);

  // No Temporal type is a time of day at a fixed offset.
  const unreadable = await pool.one("SELECT '12:00+01'::timetz AS opens").catch((error: unknown) => error);
  assert.ok(unreadable instanceof DecodeError);
  assert.match(unreadable.message, /column "opens" of type `timetz` cannot be decoded/);
}
