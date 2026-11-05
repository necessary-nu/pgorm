// Every example of the JavaScript guide, run in this runtime: each ```ts block
// of each chapter guide/chapters.json lists, as guide/examples.ts writes it
// into tests/guide-examples/, against a scratch database of its own holding the
// bakery. An example asserts what it shows, so one that no longer does fails
// here. The modules are committed, for `deno check` to type-check, and the
// first test holds them to the chapters: after editing a chapter, run this
// file with PGORM_GUIDE_WRITE=1 to write them again. PGORM_GUIDE_CHAPTERS, a
// comma-separated list of chapter ids, runs, checks and writes only those
// chapters' examples.
// [spec:pgorm:req:guide.dialects/test]    every example runs as the binding's tests run, in Node.js and Deno

import assert from "node:assert/strict";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, relative } from "node:path";
import process from "node:process";
import { after, test } from "node:test";
import { pathToFileURL } from "node:url";

import { entities, Pool } from "../lib/index.js";
import * as bakery from "../guide/bakery.ts";
import { chapters, type Example, examples, moduleText, modulePath, needs, writtenModules } from "../guide/examples.ts";
import { scratchDatabase } from "./support.ts";

const chosen = process.env.PGORM_GUIDE_CHAPTERS?.split(",").map((id) => id.trim()).filter(Boolean);
const every = chapters().flatMap(examples);
const run = chosen ? every.filter((example) => chosen.includes(example.chapter.id)) : every;
const exports = Object.keys(bakery).filter((name) => name !== "createBakery");

test("the written example modules are the chapters' examples", () => {
  const expected = new Map(run.map((example) => [modulePath(example), moduleText(example, exports)]));
  const mine = (path: string) => !chosen || run.some((example) => path.startsWith(dirname(modulePath(example))));
  const written = () => writtenModules().filter(mine);
  if (process.env.PGORM_GUIDE_WRITE === "1") {
    for (const path of written()) if (!expected.has(path)) rmSync(path);
    for (const [path, text] of expected) {
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, text);
    }
  }
  const stale = written().filter((path) => !expected.has(path));
  assert.deepEqual(stale.map((path) => relative(process.cwd(), path)), [], "modules no chapter writes; rerun with PGORM_GUIDE_WRITE=1");
  for (const [path, text] of expected) {
    let written: string | undefined;
    try {
      written = readFileSync(path, "utf8");
    } catch {
      written = undefined;
    }
    assert.equal(written, text, `${relative(process.cwd(), path)} is not its example; rerun with PGORM_GUIDE_WRITE=1`);
  }
});

let template: Promise<{ name: string; drop(): Promise<void> }> | undefined;

/** The bakery, once per run, which each example's database copies. */
function bakeryTemplate(): Promise<{ name: string; drop(): Promise<void> }> {
  template ??= (async () => {
    const database = await scratchDatabase("guide_bakery");
    const pool = new Pool(database.dsn, { maxSize: 1 });
    try {
      await bakery.createBakery(pool);
    } finally {
      await pool.close();
    }
    return { name: new URL(database.dsn).pathname.slice(1), drop: database.drop };
  })();
  return template;
}

after(async () => {
  if (template) await (await template).drop();
});

async function databaseFor(example: Example, empty: boolean): Promise<{ dsn: string; drop(): Promise<void> }> {
  const label = `guide_${example.chapter.id}_${example.index}`;
  if (empty) return scratchDatabase(label);
  const { name } = await bakeryTemplate();
  // A session the template's pool closed can outlive the close for a moment,
  // and PostgreSQL copies no database another session is connected to.
  for (let attempt = 0; ; attempt++) {
    try {
      return await scratchDatabase(label, { template: name });
    } catch (error) {
      if (attempt >= 20 || !String(error).includes("being accessed by other users")) throw error;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
}

// The binding's own module registers nothing; an application's does.
const registering = entities().length > 0;

for (const example of run) {
  const where = `guide/${example.chapter.file}:${example.line}`;
  const skip = example.marked === "application" && !registering
    ? "it needs an application's module, which checks/entities.js runs it against"
    : false;
  test(`${example.chapter.id} example ${example.index} (${where})`, { skip }, async () => {
    const { default: body } = await import(pathToFileURL(modulePath(example)).href);
    const { database, context } = needs(example);
    if (database === "none") {
      await body();
      return;
    }
    const scratch = await databaseFor(example, database === "empty");
    const pool = new Pool(scratch.dsn);
    try {
      await body(Object.fromEntries(context.map((name) => [name, name === "pool" ? pool : scratch.dsn])));
    } finally {
      await pool.close();
      await scratch.drop();
    }
  });
}
