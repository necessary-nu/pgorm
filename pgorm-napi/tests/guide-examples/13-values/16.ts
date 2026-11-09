// Example 16 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, DataType, model, TypeName } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  const flavor = new TypeName("flavor", { schema: "public" });
  const Special = model("special", {
    columns: {
      id: column("i32", { primaryKey: true, autoIncrement: true }),
      flavor: column(flavor, {
        values: ["lemon", "vanilla"],
        columnType: DataType.domain(new TypeName("special_flavor", { schema: "public" }), flavor),
      }),
    },
  });

  assert.equal(
    Special.find().where(Special.col("flavor").eq("lemon")).inspect().sql,
    `SELECT "special"."id", CAST("special"."flavor" AS text) AS "flavor" FROM "special" WHERE CAST("special"."flavor" AS public.flavor) = CAST($1::text AS public.flavor)`,
  );
}
