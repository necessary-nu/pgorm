// Example 4 of guide/15-disjoint.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import {
  column, createDisjoint, DecodeError, Decimal, disjoint, ForeignKeyViolationError, model, TypeName,
} from "@necessary/pgorm";
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

  const [base, kind, card] = await Payments
    .insert({ orderId: 1, amount: new Decimal("12.20") }, "card", { lastFour: "4242" })
    .returningModel(pool);
  assert.equal(kind, "card");
  assert.deepEqual(card, { paymentId: base.id, lastFour: "4242" });

  // A new card number: the kind's row changes, found by the base's key.
  const [, , updated] = await Payments.update({ id: base.id }, "card", { lastFour: "4343" }).returningModel(pool);
  assert.deepEqual(updated, { paymentId: base.id, lastFour: "4343" });

  // An update as another kind finds no row of that kind.
  await assert.rejects(
    Payments.update({ id: base.id, amount: new Decimal("1") }, "cash").returningModel(pool),
    DecodeError,
  );

  const [cash] = await Payments.insert({ orderId: 2, amount: new Decimal("3.80") }, "cash").returningModel(pool);

  // The customer paid with a voucher after all.
  const [moved, movedKind, voucher] = await Payments
    .changeKind({ id: cash.id }, "voucher", { code: "XMAS-2026" })
    .returningModel(pool);
  assert.equal(moved.kind, "voucher");
  assert.equal(movedKind, "voucher");
  assert.deepEqual(voucher, { paymentId: cash.id, code: "XMAS-2026" });

  // A card payment cannot lose its card row behind pgorm's back.
  await assert.rejects(
    pool.execute(`DELETE FROM "card_payment" WHERE "payment_id" = $1`, [base.id]),
    ForeignKeyViolationError,
  );

  // Deleting the payment deletes its card row with it.
  await Payment.delete().where(Payment.key({ id: base.id })).execute(pool);
  assert.equal(await CardPayment.find().count(pool), 0);
}
