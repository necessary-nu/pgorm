// Example 7 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, DecodeError, fromModel, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  interface Contact {
    readonly name: string;
    readonly phone: string;
  }

  function contact(document: any): Contact {
    if (typeof document?.name !== "string" || typeof document?.phone !== "string") {
      throw new TypeError("a contact has a name and a phone");
    }
    return { name: document.name, phone: document.phone };
  }

  const Supplier = model("supplier", {
    columns: {
      id: column("i32", { primaryKey: true }),
      contact: column("json", { decode: contact }),
    },
  });
  await pool.execute(fromModel(Supplier).table);

  const flour = await Supplier.insert({ id: 1, contact: { name: "Flour & Co", phone: "555-0100" } }).returning().one(pool);
  const phone: string = flour.contact.phone;
  assert.equal(phone, "555-0100");

  await pool.execute(`INSERT INTO supplier (id, contact) VALUES (2, '{"name": "Nobody"}')`);
  await assert.rejects(Supplier.findByKey({ id: 2 }).one(pool), DecodeError);
}
