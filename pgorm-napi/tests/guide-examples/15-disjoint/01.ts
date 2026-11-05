// Example 1 of guide/15-disjoint.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, disjoint, model, TypeName } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  const paymentKind = new TypeName("payment_kind", { schema: "public" });

  const Payment = model("payment", {
    columns: {
      id: column("i32", { primaryKey: true, generated: "byDefault" }),
      kind: column(paymentKind, { values: ["card", "cash", "voucher"] }),
      orderId: column("i32", { name: "order_id" }),
      amount: column("decimal"),
    },
  });
  const CardPayment = model("card_payment", {
    columns: {
      paymentId: column("i32", { name: "payment_id", primaryKey: true }),
      lastFour: column("text", { name: "last_four" }),
    },
  });
  const VoucherPayment = model("voucher_payment", {
    columns: {
      paymentId: column("i32", { name: "payment_id", primaryKey: true }),
      code: column("text"),
    },
  });

  const Payments = disjoint(
    "payment",
    Payment,
    { card: CardPayment, cash: null, voucher: VoucherPayment },
    { schemaName: "public" },
  );

  const { kindField, kindType, kinds } = Payments.describe();
  assert.equal(kindField, "kind");
  assert.deepEqual(kindType, { name: "payment_kind", schema: "public" });
  assert.deepEqual(kinds.map((kind) => kind.value), ["card", "cash", "voucher"]);
}
