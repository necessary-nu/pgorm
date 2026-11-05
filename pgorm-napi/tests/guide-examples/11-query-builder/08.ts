// Example 8 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { col, Conflict, ConstructionError, insert, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const customer = new Table("customer");
  const signUp = (name: string, email: string) =>
    insert(customer)
      .columns("name", "email")
      .values(name, email)
      .onConflict(Conflict.on("email").update("name"))
      .returning([col("id")]);

  assert.equal(
    signUp("Kimberly", "kim@example.com").inspect().sql,
    `INSERT INTO "customer" ("name", "email") VALUES ($1, $2) ON CONFLICT ("email") DO UPDATE SET "name" = "excluded"."name" RETURNING "id"`,
  );

  // Kim's address is taken, so her row is updated and keeps its id.
  assert.deepEqual(await pool.one(signUp("Kimberly", "kim@example.com")), { id: 1 });
  // A new address is a new customer. Its id is 4, not 3: the insert that
  // conflicted drew 3 from the id's sequence before it conflicted.
  assert.deepEqual(await pool.one(signUp("Mo", "mo@example.com")), { id: 4 });

  const kim = await pool.one("SELECT name FROM customer WHERE id = 1");
  assert.equal(kim.name, "Kimberly");

  // A row of one value, for two columns.
  assert.throws(() => insert(customer).columns("name", "email").values("Mo"), ConstructionError);
}
