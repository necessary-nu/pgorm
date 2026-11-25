// Parity: every case each family's module builds with the JavaScript API
// builds exactly the SQL and values its golden file holds — the same file the
// Rust test `statements::parity` builds each case to with pgorm-query's own
// builders — in both runtimes.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { built, type Built } from "./parity/canonical.ts";
import { cases as copy } from "./parity/copy.ts";
import { cases as json } from "./parity/json.ts";
import { cases as merge } from "./parity/merge.ts";
import { cases as pipeline } from "./parity/pipeline.ts";
import { cases as schema } from "./parity/schema.ts";
import { cases as select } from "./parity/select.ts";
import { cases as windows } from "./parity/windows.ts";
import { cases as writes } from "./parity/writes.ts";

const families = { select, writes, merge, json, windows, schema, pipeline, copy };

for (const [family, cases] of Object.entries(families)) {
  const golden = JSON.parse(readFileSync(new URL(`./parity/${family}.json`, import.meta.url), "utf8")) as Record<
    string,
    Built
  >;

  // [spec:pgorm:req:bindings.statements/test]
  test(`the ${family} family's JavaScript cases are the golden file's`, () => {
    assert.deepStrictEqual(Object.keys(cases).sort(), Object.keys(golden).sort());
  });

  for (const [name, make] of Object.entries(cases)) {
    // [spec:pgorm:req:bindings.delegation+1/test]
    test(`${family}: ${name} builds what pgorm-query builds`, () => {
      const expected = golden[name];
      assert.ok(expected, `${name} is in the golden file`);
      assert.deepStrictEqual(built(make()), { sql: expected.sql, values: expected.values });
    });
  }
}
