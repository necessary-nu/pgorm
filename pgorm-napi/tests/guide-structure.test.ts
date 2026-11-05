// The JavaScript guide is the Rust guide's dialect chapter for chapter: its
// list, guide/chapters.json, names every chapter docs/guide/chapters.json
// does, under the same id and title and in the same order, and any other
// chapter it names is declared the JavaScript guide's own. Each listed file
// opens with its title, the guide's index links every chapter in the listed
// order, and no other markdown file in guide/ is a chapter.
// [spec:pgorm:req:guide.dialects/test]    the Rust guide's chapters, one for one and in order, and only declared extras

import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { chapters, guide, rustChapters } from "../guide/examples.ts";

test("every Rust chapter is a chapter here, under its id and title, in order", () => {
  const ours = chapters().filter((chapter) => chapter.javascript_only !== true);
  assert.deepEqual(
    ours.map(({ id, title }) => ({ id, title })),
    rustChapters().map(({ id, title }) => ({ id, title })),
  );
});

test("a chapter the Rust guide lacks is declared the JavaScript guide's own", () => {
  const rust = new Set(rustChapters().map((chapter) => chapter.id));
  for (const chapter of chapters()) {
    const keys = Object.keys(chapter).filter((key) => !["id", "title", "file", "javascript_only"].includes(key));
    assert.deepEqual(keys, [], `${chapter.id} says what the chapter list does not know`);
    if (chapter.javascript_only === undefined) continue;
    assert.equal(chapter.javascript_only, true, `${chapter.id}: javascript_only is true or absent`);
    assert.ok(!rust.has(chapter.id), `${chapter.id} is a Rust chapter, so it is not the JavaScript guide's own`);
  }
  const ids = chapters().map((chapter) => chapter.id);
  assert.equal(new Set(ids).size, ids.length, "a chapter id is listed twice");
});

test("each chapter's file exists and opens with its title, and no other file is a chapter", () => {
  const listed = chapters().map((chapter) => chapter.file);
  for (const chapter of chapters()) {
    const path = join(guide, chapter.file);
    assert.ok(existsSync(path), `${chapter.file} does not exist`);
    const first = readFileSync(path, "utf8").split("\n")[0];
    assert.equal(first, `# ${chapter.title}`, `${chapter.file} opens with its title`);
  }
  const files = readdirSync(guide).filter((file) => file.endsWith(".md") && file !== "README.md");
  assert.deepEqual(files.sort(), [...listed].sort(), "every markdown file in guide/ but README.md is a listed chapter");
});

test("the guide's index lists every chapter, in the listed order", () => {
  const index = readFileSync(join(guide, "README.md"), "utf8");
  const rows = index.split("\n").filter((line) => /^\| \d+\. \[/.test(line));
  const linked = rows.map((line) => /\]\((\d\d-[a-z0-9-]+\.md)\)/.exec(line)?.[1]);
  assert.deepEqual(linked, chapters().map((chapter) => chapter.file));
});
