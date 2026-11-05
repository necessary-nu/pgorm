// Example 1 of guide/09-errors.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal, ForeignKeyViolationError, NotNullViolationError, UniqueViolationError } from "@necessary/pgorm";
import { Cake, Customer } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // Lee's address is taken.
  const duplicate = await Customer.insert({ name: "Lee Two", email: "lee@example.com" })
    .execute(pool)
    .catch((error: unknown) => error);
  assert.ok(duplicate instanceof UniqueViolationError);
  assert.equal(duplicate.table, "customer");
  assert.equal(duplicate.constraint, "customer_email_key");

  // There is no bakery 99.
  const orphan = await Cake.insert({
    name: "Ghost Cake",
    flavor: "vanilla",
    price: new Decimal("1.00"),
    glutenFree: false,
    bakeryId: 99,
  }).execute(pool).catch((error: unknown) => error);
  assert.ok(orphan instanceof ForeignKeyViolationError);
  assert.equal(orphan.constraint, "fk-cake-bakery_id");

  // A cake left without a name.
  const nameless = await pool
    .execute("INSERT INTO cake (flavor, price, gluten_free, bakery_id) VALUES ('vanilla', 1.00, false, 1)")
    .catch((error: unknown) => error);
  assert.ok(nameless instanceof NotNullViolationError);
  assert.equal(nameless.column, "name");
}
