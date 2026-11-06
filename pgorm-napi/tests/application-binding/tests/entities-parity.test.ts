// Parity: each entity case `cases.ts` builds through the application's own
// module is the SQL and values `parity.json` holds — the file the application
// crate's Rust test builds each case to with its entities directly — in both
// runtimes.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { type Built, canonical } from "../../parity/canonical.ts";
import { cases } from "./cases.ts";

const golden = JSON.parse(readFileSync(new URL("./parity.json", import.meta.url), "utf8")) as Record<string, Built>;

// [spec:pgorm:req:bindings.entity-reads+4/test]
test("the entity family's cases are the golden file's", () => {
  assert.deepStrictEqual(Object.keys(cases()).sort(), Object.keys(golden).sort());
});

for (const [name, make] of Object.entries(cases())) {
  test(`entities: ${name} builds what the application's Rust builds`, () => {
    const expected = golden[name];
    assert.ok(expected, `${name} is in the golden file`);
    const { sql, values } = make();
    assert.deepStrictEqual({ sql, values: values.map(canonical) }, { sql: expected.sql, values: expected.values });
  });
}
