// The JavaScript guide's chapters, as chapters.json lists them, and the
// examples in them. Every ```ts block of a chapter is an example.
// tests/guide.test.ts writes each into a module of its own under
// tests/guide-examples/, committed so that `deno check` type-checks every
// example as it checks the rest of the package, and runs that module against
// a scratch database.
//
// A block is written as a reader would write it: its imports first, then its
// code, which may name the bakery's models (bakery.ts) and `pool` and `url`
// (context.ts) without declaring them. Its module imports what the block names
// of those, and wraps the code in the function the test calls.
// [spec:pgorm:req:guide.dialects]    the JavaScript guide: the Rust guide's chapters one for one and in order, beside its declared own

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const guide = dirname(fileURLToPath(import.meta.url));

const napi = join(guide, "..");

/** Where the test writes each example's module, committed beside the suite. */
export const generated = join(napi, "tests", "guide-examples");

export interface Chapter {
  readonly id: string;
  readonly title: string;
  readonly file: string;
  /** Whether the chapter is the JavaScript guide's own, with no counterpart in the Rust guide. */
  readonly javascript_only?: boolean;
}

/** The Rust guide's chapters, the list the JavaScript guide is held to. */
export function rustChapters(): Chapter[] {
  return JSON.parse(readFileSync(join(napi, "..", "docs", "guide", "chapters.json"), "utf8")).chapters;
}

export function chapters(): Chapter[] {
  return JSON.parse(readFileSync(join(guide, "chapters.json"), "utf8")).chapters;
}

/** What an example's database holds: `none` when it names neither `pool` nor `url`. */
export type Database = "bakery" | "empty" | "none";

/**
 * What a `<!-- example: .. -->` comment before a fence asks for: `empty`, a
 * database without the bakery; `application`, an application's module that
 * registers entities (checks/entities.js runs these against its fixture) and
 * an empty database.
 */
export type Mark = "empty" | "application";

export interface Example {
  readonly chapter: Chapter;
  /** Its place among the chapter's examples, from 1. */
  readonly index: number;
  /** The line of the chapter its fence opens on, from 1. */
  readonly line: number;
  readonly code: string;
  readonly marked: Mark | null;
}

const FENCE = /^```(\S*)\s*$/;
const MARK = /^<!--\s*example:\s*(\S+)\s*-->$/;

/** Every ```ts block of `chapter`, in order: other fences are not examples. */
export function examples(chapter: Chapter): Example[] {
  const lines = readFileSync(join(guide, chapter.file), "utf8").split("\n");
  const found: Example[] = [];
  let open: { language: string; line: number; body: string[]; marked: Mark | null } | null = null;
  let marked: Mark | null = null;
  for (const [number, text] of lines.entries()) {
    const fence = FENCE.exec(text);
    if (open) {
      if (fence && fence[1] === "") {
        if (open.language === "ts") {
          found.push({
            chapter,
            index: found.length + 1,
            line: open.line,
            code: open.body.join("\n"),
            marked: open.marked,
          });
        }
        open = null;
      } else {
        open.body.push(text);
      }
      continue;
    }
    if (fence) {
      open = { language: fence[1]!, line: number + 1, body: [], marked };
      marked = null;
      continue;
    }
    const mark = MARK.exec(text.trim());
    if (mark) {
      if (mark[1] !== "empty" && mark[1] !== "application") {
        throw new Error(`${chapter.file}:${number + 1}: an example is marked empty, application or not at all`);
      }
      marked = mark[1];
    } else if (text.trim() !== "") {
      marked = null;
    }
  }
  if (open) throw new Error(`${chapter.file}:${open.line}: a code block is never closed`);
  return found;
}

const IMPORT = /^import\b[^;]*?["'][^"']+["'];?[ \t]*(?:\n|$)/;

function split(code: string): { imports: string[]; body: string } {
  const imports: string[] = [];
  let rest = code;
  for (;;) {
    const trimmed = rest.replace(/^(?:[ \t]*\n)+/, "");
    const match = IMPORT.exec(trimmed);
    if (!match) return { imports, body: trimmed.replace(/\s+$/, "") };
    imports.push(match[0].trimEnd());
    rest = trimmed.slice(match[0].length);
  }
}

function imported(imports: readonly string[]): Set<string> {
  const names = new Set<string>();
  for (const statement of imports) {
    const clause = /^import\s+(?:type\s+)?([\s\S]*?)\s+from\s/.exec(statement)?.[1] ?? "";
    for (const part of clause.replace(/[{}]/g, ",").split(",")) {
      const name = part.trim().replace(/^type\s+/, "").split(/\s+as\s+/).pop()?.replace(/^\*\s*/, "").trim();
      if (name) names.add(name);
    }
  }
  return names;
}

/** `body` without its comments and the text of its strings, where a name is not a reference. */
function code(body: string): string {
  return body
    .replace(/\/\/[^\n]*/g, "")
    // A template literal is text, an interpolation in it of no braces of its
    // own included, so a tagged template's `${..}` does not end it early.
    .replace(/"(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*'|`(?:[^`\\$]|\\.|\$(?!\{)|\$\{[^{}`]*\})*`/g, '""');
}

function names(body: string, name: string): boolean {
  return new RegExp(`(?<![\\w$.])${name.replace(/\$/g, "\\$")}(?![\\w$:])`).test(code(body));
}

function declares(body: string, name: string): boolean {
  return new RegExp(`\\b(?:const|let|var|using|class|function|type|interface)\\s+${name}\\b`).test(body);
}

/** Where the test writes `example`'s module. */
export function modulePath(example: Example): string {
  return join(generated, example.chapter.file.replace(/\.md$/, ""), `${String(example.index).padStart(2, "0")}.ts`);
}

/** What `example` needs to run: a database, and which of `pool` and `url` it names. */
export function needs(example: Example): { database: Database; context: string[] } {
  const { body } = split(example.code);
  const context = ["pool", "url"].filter((name) => names(body, name) && !declares(body, name));
  const database: Database = context.length === 0 ? "none" : example.marked === null ? "bakery" : "empty";
  return { database, context };
}

/**
 * The module `example` runs as: its imports, then the bakery's exports it
 * names and does not declare, then its code as the body of the function the
 * test calls with the context it names.
 */
export function moduleText(example: Example, bakery: readonly string[]): string {
  const { imports, body } = split(example.code);
  const own = imported(imports);
  const models = bakery.filter((name) => names(body, name) && !declares(body, name) && !own.has(name)).sort();
  const { context } = needs(example);
  const header = [
    `// Example ${example.index} of guide/${example.chapter.file}, written by tests/guide.test.ts: edit the`,
    "// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.",
    "",
  ];
  const own_imports = [...imports];
  if (models.length > 0) own_imports.push(`import { ${models.join(", ")} } from "../../../guide/bakery.ts";`);
  if (context.length > 0) own_imports.push(`import type { GuideContext } from "../../../guide/context.ts";`);
  const parameter = context.length > 0 ? `{ ${context.join(", ")} }: GuideContext` : "";
  const indented = body.split("\n").map((line) => (line === "" ? "" : `  ${line}`));
  return [
    ...header,
    ...own_imports,
    ...(own_imports.length > 0 ? [""] : []),
    `export default async function example(${parameter}): Promise<void> {`,
    ...indented,
    "}",
    "",
  ].join("\n");
}

/** The module files under tests/guide/examples/, by path. */
export function writtenModules(): string[] {
  if (!existsSync(generated)) return [];
  return readdirSync(generated, { recursive: true, encoding: "utf8" })
    .filter((file) => file.endsWith(".ts"))
    .map((file) => join(generated, file))
    .sort();
}
