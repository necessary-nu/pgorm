// Example 7 of guide/15-disjoint.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, ConstructionError, createDisjoint, disjoint, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute("CREATE TABLE staff (id serial PRIMARY KEY, name text NOT NULL)");
  await pool.execute("CREATE TABLE driver (staff_id integer PRIMARY KEY REFERENCES staff, licence text NOT NULL)");
  await pool.execute("INSERT INTO staff (name) VALUES ('Eli'), ('Fay')");
  await pool.execute("INSERT INTO driver VALUES (1, 'B-1234')");

  const Staff = model("staff", {
    columns: { id: column("i32", { primaryKey: true, default: true }), name: column("text") },
  });
  const Driver = model("driver", {
    columns: { staffId: column("i32", { name: "staff_id", primaryKey: true }), licence: column("text") },
  });
  const Roles = disjoint("staff", Staff, { driver: Driver, counter: null }, { unkinded: true });

  const staff = await Roles.find().orderBy(Staff.col("id").asc()).all(pool);
  assert.deepEqual(staff, [
    [{ id: 1, name: "Eli" }, "driver", { staffId: 1, licence: "B-1234" }],
    [{ id: 2, name: "Fay" }, "counter", null],
  ]);
  assert.equal(await Roles.find().kind("counter").count(pool), 1);

  assert.throws(() => createDisjoint(Roles as never), ConstructionError);
}
