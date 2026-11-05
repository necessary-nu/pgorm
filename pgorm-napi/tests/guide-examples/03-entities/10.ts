// Example 10 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute(
    `CREATE TABLE subscriber ( id serial NOT NULL, name varchar NOT NULL, email varchar NOT NULL, weekly bool NOT NULL DEFAULT TRUE, PRIMARY KEY (id), UNIQUE (email) )`,
  );

  const Subscriber = model("subscriber", {
    columns: {
      id: column("i32", { primaryKey: true, default: true }),
      name: column("text"),
      email: column("text"),
      weekly: column("bool", { default: true }),
    },
    uniqueKeys: [{ fields: "email" }],
  });

  // What the newsletter's sign-up form sends: two of the columns.
  const form = { name: "Mo", email: "mo@example.com" };
  const mo = await Subscriber.insert(form).returning().one(pool);
  assert.equal(mo.email, "mo@example.com");
  // The form left `weekly` out, so it took the column's default.
  assert.equal(mo.weekly, true);
}
