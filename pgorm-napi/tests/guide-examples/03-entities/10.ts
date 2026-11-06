// Example 10 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, ConstructionError, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const Special = model("special", {
    columns: {
      id: column("i32", { primaryKey: true, default: true }),
      name: column("text"),
      note: column("text"),
    },
    hooks: {
      beforeSave(values) {
        if (!values.name) throw new ConstructionError("a special needs a name");
        return { note: "Ask at the counter", ...values };
      },
    },
  });

  await pool.execute("CREATE TABLE special (id serial PRIMARY KEY, name text NOT NULL, note text NOT NULL)");
  const saved = await Special.active({ name: "Saturday cinnamon buns" }).insert(pool);
  assert.equal(saved.note, "Ask at the counter");

  await assert.rejects(Special.active({ name: "" }).insert(pool), /a special needs a name/);
  assert.deepEqual(Special.describe().hooks, ["beforeSave"]);
}
