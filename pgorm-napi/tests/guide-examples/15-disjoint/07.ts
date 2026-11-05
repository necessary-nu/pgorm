// Example 7 of guide/15-disjoint.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  addDisjointKind, column, createDisjoint, disjoint, migrationFile, migrationFiles, model, TypeName,
} from "@necessary/pgorm";

export default async function example(): Promise<void> {
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
  const opening = disjoint("payment", payment(["card", "cash"]), { card: CardPayment, cash: null }, {
    schemaName: "public",
  });
  const withVouchers = disjoint(
    "payment",
    payment(["card", "cash", "voucher"]),
    { card: CardPayment, cash: null, voucher: VoucherPayment },
    { schemaName: "public" },
  );

  // The directory this program keeps its migrations in.
  const migrations = "guide/migrations";

  const opened = migrationFile("m20260201_000001_create_payment", createDisjoint(opening));
  const added = migrationFiles(addDisjointKind(withVouchers, "voucher"), "m20260301_000001_add_voucher");
  assert.deepEqual(added.map((file) => file.name), ["m20260301_000001_add_voucher_1", "m20260301_000001_add_voucher_2"]);
  assert.equal(
    added[0]!.text,
    `-- pgorm: commits-after\nALTER TYPE "public"."payment_kind" ADD VALUE 'voucher' AFTER 'cash';\n`,
  );
  assert.deepEqual(added.map((file) => file.commitsAfter), [true, false]);

  for (const file of [opened, ...added]) {
    // Frozen when each change was made, so there is nothing to write.
    assert.equal(file.freeze(migrations), false);
  }
  assert.equal(readFileSync(`${migrations}/m20260301_000001_add_voucher_1.sql`, "utf8"), added[0]!.text);
}
