// Example 5 of guide/15-disjoint.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, createDisjoint, Decimal, disjoint, model, TypeName } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
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
  for (const statement of createDisjoint(Payments)) await pool.execute(statement);
  await Payments.insert({ orderId: 1, amount: new Decimal("12.20") }, "card", { lastFour: "4242" }).returningModel(pool);
  await Payments.insert({ orderId: 2, amount: new Decimal("3.80") }, "cash").returningModel(pool);
  await Payments.insert({ orderId: 1, amount: new Decimal("4.50") }, "card", { lastFour: "1111" }).returningModel(pool);

  const every = await Payments.find().orderBy(Payment.col("id").asc()).all(pool);
  assert.deepEqual(every.map(([, kind]) => kind), ["card", "cash", "card"]);

  assert.equal(await Payments.find().kind("card").count(pool), 2);

  const [base, kind, card] = await Payments.find()
    .filterKind("card", CardPayment.col("lastFour").endsWith("42"))
    .one(pool);
  assert.equal(base.orderId, 1);
  assert.equal(kind, "card");
  assert.deepEqual(card, { paymentId: base.id, lastFour: "4242" });
}
