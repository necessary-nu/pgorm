// Example 4 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ColumnDef, createTable, migrationFile, Table } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  const create = createTable(new Table("bakery"))
    .column(new ColumnDef("id", "integer").autoIncrement().notNull())
    .column(new ColumnDef("name", "varchar").notNull())
    .column(new ColumnDef("city", "varchar").notNull())
    .primaryKey("id");

  // Frozen when the bakery table was added; freezing again writes nothing.
  const file = migrationFile("m20260101_000001_create_bakery", [create]);
  assert.equal(
    file.text,
    `CREATE TABLE "bakery" ( "id" serial NOT NULL, "name" varchar NOT NULL, "city" varchar NOT NULL, PRIMARY KEY ("id") );\n`,
  );
  assert.equal(file.freeze("guide/migrations"), false);
}
