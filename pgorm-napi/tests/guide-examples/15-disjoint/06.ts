// Example 6 of guide/15-disjoint.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { addDisjointKind, column, createDisjoint, Decimal, disjoint, model, TypeName } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const paymentKind = new TypeName("payment_kind", { schema: "public" });
  const payment = (kinds: string[]) =>
    model("payment", {
      columns: {
        id: column("i32", { primaryKey: true, generated: "byDefault" }),
        kind: column(paymentKind, { values: kinds }),
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

  const before = disjoint("payment", payment(["card", "cash"]), { card: CardPayment, cash: null }, {
    schemaName: "public",
  });
  for (const statement of createDisjoint(before)) await pool.execute(statement);

  const Payments = disjoint(
    "payment",
    payment(["card", "cash", "voucher"]),
    { card: CardPayment, cash: null, voucher: VoucherPayment },
    { schemaName: "public" },
  );

  const { steps } = addDisjointKind(Payments, "voucher");
  assert.equal(steps.length, 2);
  assert.equal(
    steps[0]!.statements[0]!.inspect().sql,
    `ALTER TYPE "public"."payment_kind" ADD VALUE 'voucher' AFTER 'cash'`,
  );
  assert.ok(steps.every((step) => step.gap === null));

  for (const step of steps) {
    await pool.transaction(async (tx) => {
      for (const statement of step.statements) await tx.execute(statement);
    });
    pool.clearStatementCaches();
  }

  const [, kind, voucher] = await Payments
    .insert({ orderId: 2, amount: new Decimal("3.80") }, "voucher", { code: "XMAS-2026" })
    .returningModel(pool);
  assert.equal(kind, "voucher");
  assert.deepEqual(voucher, { paymentId: 1, code: "XMAS-2026" });
}
