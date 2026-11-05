// Example 1 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, model, TypeName } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  const flavor = new TypeName("flavor", { schema: "public" });

  const Cake = model("cake", {
    columns: {
      id: column("i32", { primaryKey: true, default: true }),
      name: column("text"),
      flavor: column(flavor, { values: ["chocolate", "vanilla", "lemon", "carrot"] }),
      price: column("decimal"),
      glutenFree: column("bool", { name: "gluten_free" }),
      bakeryId: column("i32", { name: "bakery_id" }),
    },
  });

  assert.equal(Cake.name, "cake");
  const columns = Object.values(Cake.describe().fields).map((field) => field.column);
  assert.deepEqual(columns, ["id", "name", "flavor", "price", "gluten_free", "bakery_id"]);
}
