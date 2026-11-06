// Example 8 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { any, column, model } from "@necessary/pgorm";
import { flavor } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute(
    "CREATE TABLE menu (id serial PRIMARY KEY, name varchar NOT NULL, tags varchar[] NOT NULL, flavors flavor[] NOT NULL)",
  );
  const Menu = model("menu", {
    columns: {
      id: column("i32", { primaryKey: true, default: true }),
      name: column("text"),
      tags: column("text", { array: true }),
      flavors: column(flavor, { array: true, values: ["chocolate", "vanilla", "lemon", "carrot"] }),
    },
  });

  await Menu.insertMany([
    { name: "Breakfast", tags: ["morning", "coffee"], flavors: ["lemon", "vanilla"] },
    { name: "Party", tags: ["evening"], flavors: ["chocolate"] },
  ]).execute(pool);

  const withCoffee = await Menu.select("name").where(Menu.col("tags").contains(["coffee"])).all(pool);
  assert.deepEqual(withCoffee, [{ name: "Breakfast" }]);

  const chocolate = Menu.select("name").where(Menu.col("flavors").overlaps(["chocolate", "carrot"]));
  assert.equal(
    chocolate.inspect().sql,
    `SELECT "menu"."name" FROM "menu" WHERE "menu"."flavors" && CAST($1::text[] AS public.flavor[])`,
  );
  assert.deepEqual(await chocolate.all(pool), [{ name: "Party" }]);

  const named = Menu.select("name").where(Menu.col("name").eq(any(["Party", "Picnic"])));
  assert.equal(named.inspect().sql, `SELECT "menu"."name" FROM "menu" WHERE "menu"."name" = ANY($1)`);
  assert.deepEqual(await named.all(pool), [{ name: "Party" }]);
}
