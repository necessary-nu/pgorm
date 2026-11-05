// Example 5 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal, DecodeError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const menus = await pool.query(
    `SELECT bakery.name AS bakery, count(cake.id) AS cakes
     FROM bakery JOIN cake ON cake.bakery_id = bakery.id
     WHERE cake.price < $1
     GROUP BY bakery.name ORDER BY bakery.name`,
    [new Decimal("5.00")],
  );
  assert.deepEqual(menus, [
    { bakery: "Moonlight", cakes: 2n },
    { bakery: "Sunrise", cakes: 2n },
  ]);

  // The first of many is a LIMIT of the SQL's own: pgorm adds none.
  const cheapest = await pool.one("SELECT name, price FROM cake ORDER BY price LIMIT 1");
  assert.equal(cheapest.name, "Vanilla Sponge");

  // Five rows, where `one` wants exactly one.
  await assert.rejects(pool.one("SELECT name FROM cake"), DecodeError);

  // Two columns called `name`.
  await assert.rejects(
    pool.one("SELECT cake.name, bakery.name FROM cake JOIN bakery ON bakery.id = cake.bakery_id WHERE cake.id = 1"),
    DecodeError,
  );
}
