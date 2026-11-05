// Example 2 of guide/15-disjoint.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, ConstructionError, disjoint, model, TypeName } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  const paymentKind = new TypeName("payment_kind", { schema: "public" });
  const Payment = model("payment", {
    columns: {
      id: column("i32", { primaryKey: true, generated: "byDefault" }),
      kind: column(paymentKind, { values: ["card", "cash"], nullable: true }),
    },
  });
  const CardPayment = model("card_payment", {
    columns: { paymentId: column("i32", { name: "payment_id", primaryKey: true }) },
  });

  assert.throws(
    () => disjoint("payment", Payment, { card: CardPayment, cash: null }, { schemaName: "public" }),
    (error: unknown) => error instanceof ConstructionError && /is nullable/.test(error.message),
  );
}
