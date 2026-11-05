// Parity: every model case `parity/models.ts` builds is the SQL and values
// `parity/models.json` holds — the file the Rust test `models::parity`
// builds each case to with pgorm's own entities, graphs, cursors and
// paginator — in both runtimes.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { type Built, canonical } from "./parity/canonical.ts";
import { cases } from "./parity/models.ts";

const golden = JSON.parse(readFileSync(new URL("./parity/models.json", import.meta.url), "utf8")) as Record<string, Built>;

// [spec:pgorm:req:bindings.models+5/test]
test("the model family's JavaScript cases are the golden file's", () => {
  assert.deepStrictEqual(Object.keys(cases).sort(), Object.keys(golden).sort());
});

for (const [name, make] of Object.entries(cases)) {
  test(`models: ${name} builds what pgorm builds`, () => {
    const expected = golden[name];
    assert.ok(expected, `${name} is in the golden file`);
    const { sql, values } = make();
    assert.deepStrictEqual({ sql, values: values.map(canonical) }, { sql: expected.sql, values: expected.values });
  });
}
