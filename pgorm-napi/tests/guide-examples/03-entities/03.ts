// Example 3 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { fromModel } from "@necessary/pgorm";
import { Customer } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  assert.equal(
    fromModel(Customer).table.inspect().sql,
    `CREATE TABLE "customer" ( "id" serial NOT NULL, "name" varchar NOT NULL, "email" varchar NOT NULL, "notes" varchar, PRIMARY KEY ("id"), UNIQUE ("email") )`,
  );
}
