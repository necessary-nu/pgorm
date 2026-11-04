// The module offers the surface both bindings share, spelled as JavaScript
// spells it, with JavaScript's own names and its gaps declared beside it in
// docs/spec/bindings-surface.json, which pgorm-python's suite reads too; and
// each describe() reports the keys the file declares for it.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import * as pgorm from "../lib/index.js";

type Entry = { readonly spelled?: string; readonly question: string };
type Side = {
  readonly modules: Readonly<Record<string, string>>;
  readonly spellings: Readonly<Record<string, string>>;
  readonly protocols: Readonly<Record<string, string>>;
  readonly only: Readonly<Record<string, string>>;
  readonly gaps: Readonly<Record<string, string>>;
  readonly undecided: Readonly<Record<string, Entry>>;
};
/** A declared description: a value, a list of items of one shape, or an object of exactly these keys. */
type Shape = null | readonly [Shape] | { readonly [key: string]: Shape };
type Surface = {
  readonly modules: Readonly<Record<string, Readonly<Record<string, readonly string[] | null>>>>;
  readonly descriptions: Readonly<Record<string, Shape>>;
  readonly questions: Readonly<Record<string, string>>;
  readonly javascript: Side;
};

const surface: Surface = JSON.parse(
  readFileSync(new URL("../../docs/spec/bindings-surface.json", import.meta.url), "utf8"),
);
const side = surface.javascript;

type Constructor = abstract new (...args: never) => unknown;

const isClass = (value: unknown): value is Constructor =>
  typeof value === "function" && /^class[\s{]/.test(Function.prototype.toString.call(value));

/** A class's static methods and constants and its prototype chain's methods. */
function classMembers(cls: Constructor): Set<string> {
  const found = new Set<string>();
  for (const name of Object.getOwnPropertyNames(cls)) {
    if (["length", "name", "prototype"].includes(name)) continue;
    const value = Object.getOwnPropertyDescriptor(cls, name)?.value;
    if (typeof value === "function" || value instanceof cls) found.add(name);
  }
  for (let proto = cls.prototype; proto && proto !== Object.prototype && proto !== Error.prototype;) {
    for (const name of Object.getOwnPropertyNames(proto)) {
      if (name !== "constructor" && typeof Object.getOwnPropertyDescriptor(proto, name)?.value === "function") {
        found.add(name);
      }
    }
    proto = Object.getPrototypeOf(proto);
  }
  return found;
}

/** Every export of the module and of its namespaces, as `module.name` and `module.Class.member`. */
function liveSurface(): { names: Set<string>; members: Map<string, Set<string>> } {
  const names = new Set<string>();
  const members = new Map<string, Set<string>>();
  const namespaces = new Set(Object.values(side.modules).filter((name) => name !== ""));
  for (const space of ["", ...namespaces]) {
    const exports: Record<string, unknown> = pgorm;
    const module = (space === "" ? exports : exports[space]) as Record<string, unknown>;
    for (const [name, value] of Object.entries(module)) {
      if (space === "" && namespaces.has(name)) continue;
      names.add(`${space}.${name}`);
      if (isClass(value)) members.set(`${space}.${name}`, classMembers(value));
    }
  }
  return { names, members };
}

const camel = (name: string) => name.replace(/_([a-z0-9])/g, (_, letter: string) => letter.toUpperCase());

function spell(path: string): string {
  const name = path.split(".")[1] ?? "";
  const spelled = side.spellings[path] ?? side.undecided[path]?.spelled;
  return spelled ?? (/^[A-Z]/.test(name) ? name : camel(name));
}

function spellMember(owner: string, member: string): string {
  const path = `${owner}.${member}`;
  const spelled = side.spellings[path] ?? side.undecided[path]?.spelled;
  if (spelled) return spelled;
  return /^[A-Z]/.test(member) ? member[0]!.toLowerCase() + member.slice(1) : camel(member);
}

/** A neutral path's module as the JavaScript surface keys it. */
const home = (path: string) => side.modules[path.split(".")[0]!] ?? "";

function declared(): { expected: Map<string, string>; gaps: Map<string, string>; classes: Set<string> } {
  const expected = new Map<string, string>();
  const gaps = new Map<string, string>();
  const classes = new Set<string>();
  for (const [module, entries] of Object.entries(surface.modules)) {
    for (const [name, methods] of Object.entries(entries)) {
      const path = `${module}.${name}`;
      const key = `${home(path)}.${spell(path)}`;
      if (path in side.gaps) {
        gaps.set(key, path);
        continue;
      }
      expected.set(key, path);
      if (methods !== null) classes.add(key);
      for (const member of methods ?? []) {
        const memberPath = `${path}.${member}`;
        (memberPath in side.gaps ? gaps : expected).set(`${key}.${spellMember(path, member)}`, memberPath);
      }
    }
  }
  return { expected, gaps, classes };
}

/** JavaScript's own names, for a language reason or awaiting the operator. */
function ownNames(): Map<string, string> {
  const own = new Map<string, string>();
  const paths = [...Object.keys(side.only), ...Object.entries(side.undecided).filter(([, e]) => !e.spelled).map(([p]) => p)];
  for (const path of paths) own.set(`${home(path)}.${path.split(".").slice(1).join(".")}`, path);
  return own;
}

const live = liveSurface();
const all = new Set([...live.names, ...[...live.members].flatMap(([key, found]) => [...found].map((m) => `${key}.${m}`))]);
const { expected, gaps, classes } = declared();
const own = ownNames();

// [spec:pgorm:req:bindings.surface+2/test]
// [spec:pgorm:req:napi.naming+4/test]
test("the module offers every shared name, or the surface lists it as a gap with its plan node", () => {
  const missing = [...expected].filter(([key]) => !all.has(key)).map(([, path]) => path).sort();
  assert.deepStrictEqual(missing, []);
});

// [spec:pgorm:req:bindings.surface+2/test]
test("the module offers nothing the surface does not declare as shared or as JavaScript's own", () => {
  const extra = [...all].filter((key) => {
    if (expected.has(key) || own.has(key)) return false;
    const parts = key.split(".");
    if (parts.length < 3) return true;
    const owner = parts.slice(0, -1).join(".");
    return classes.has(owner) && !(parts.at(-1)! in side.protocols);
  });
  assert.deepStrictEqual(extra.sort(), []);
});

// [spec:pgorm:req:bindings.surface+2/test]
test("a gap that lands leaves the gap list", () => {
  const closed = [...gaps].filter(([key]) => all.has(key) && !expected.has(key)).map(([, path]) => path);
  assert.deepStrictEqual(closed.sort(), []);
});

// [spec:pgorm:req:bindings.surface+2/test]
test("every name the surface declares as JavaScript's own is still offered", () => {
  const stale = [...own].filter(([key]) => !all.has(key)).map(([, path]) => path);
  assert.deepStrictEqual(stale.sort(), []);
});

// [spec:pgorm:req:bindings.surface+2/test]
test("every gap names a plan node and every undecided name a question", () => {
  const shared = new Set(Object.entries(surface.modules).flatMap(([module, entries]) =>
    Object.entries(entries).flatMap(([name, methods]) => [
      `${module}.${name}`,
      ...(methods ?? []).map((member) => `${module}.${name}.${member}`),
    ])
  ));
  for (const [path, node] of Object.entries(side.gaps)) {
    assert.ok(shared.has(path), `${path} is a gap in no shared name`);
    assert.match(node, /^[a-z0-9][a-z0-9.-]*$/);
  }
  for (const entry of Object.values(side.undecided)) assert.ok(entry.question in surface.questions);
  for (const reason of [...Object.values(side.only), ...Object.values(side.protocols)]) assert.ok(reason.trim());
});

const isList = (shape: Shape): shape is readonly [Shape] => Array.isArray(shape);

/** Every key path a declared description holds, nested keys included. */
function declaredPaths(shape: Shape, path = ""): string[] {
  if (shape === null) return [];
  if (isList(shape)) return declaredPaths(shape[0], `${path}[]`);
  return Object.entries(shape).flatMap(([key, inner]) => [`${path}.${key}`, ...declaredPaths(inner, `${path}.${key}`)]);
}

/**
 * How a live description differs from its declared shape, each difference at
 * its neutral key path; every declared path it reaches joins `seen`.
 * JavaScript spells a neutral key in camelCase (napi.naming).
 */
function differences(live: unknown, shape: Shape, path: string, seen: Set<string>): string[] {
  if (shape === null) return [];
  if (isList(shape)) {
    if (!Array.isArray(live)) return [`${path}: is no array`];
    return live.flatMap((item) => differences(item, shape[0], `${path}[]`, seen));
  }
  if (typeof live !== "object" || live === null || Array.isArray(live)) return [`${path}: is no object`];
  const object = live as Record<string, unknown>;
  const keys = Object.keys(shape);
  if (keys.length === 1 && keys[0] === "*") {
    return Object.values(object).flatMap((item) => {
      seen.add(`${path}.*`);
      return differences(item, shape["*"]!, `${path}.*`, seen);
    });
  }
  const spelled = new Map(keys.map((key) => [camel(key), key]));
  const found = Object.keys(object).filter((key) => !spelled.has(key)).map((key) => `${path}.${key}: undeclared`);
  for (const [key, neutral] of spelled) {
    if (!(key in object)) {
      found.push(`${path}.${neutral}: missing`);
      continue;
    }
    seen.add(`${path}.${neutral}`);
    found.push(...differences(object[key], shape[neutral]!, `${path}.${neutral}`, seen));
  }
  return found;
}

/** Each description has exactly the keys declared for `owner`, and together they reach every one. */
function assertDeclared(owner: string, descriptions: readonly unknown[]) {
  const shape = surface.descriptions[owner]!;
  const seen = new Set<string>();
  const found = [...new Set(descriptions.flatMap((described) => differences(described, shape, owner, seen)))];
  assert.deepStrictEqual(found.sort(), [], "describe() keys differ from bindings-surface.json's descriptions");
  const unreached = declaredPaths(shape).map((path) => owner + path).filter((path) => !seen.has(path));
  assert.deepStrictEqual(unreached.sort(), [], "declared keys no live description reached");
}

// [spec:pgorm:req:bindings.surface+2/test]
// [spec:pgorm:req:bindings.models+5/test]
// [spec:pgorm:req:bindings.model-unique-keys/test]
test("a model's describe() reports exactly the keys the surface declares", () => {
  const declared = pgorm.model("accounts", {
    schema: "app",
    columns: {
      id: pgorm.column("i64", { primaryKey: true, generated: "always" }),
      mood: pgorm.column(new pgorm.TypeName("mood", { schema: "app" }), { nullable: true, values: ["calm"] }),
    },
    uniqueKeys: [{ fields: ["mood", "id"], name: "accounts_mood_id" }],
  });
  assertDeclared("pgorm.Model", [declared.describe(), declared.as("a").describe()]);
});

// [spec:pgorm:req:bindings.surface+2/test]
// [spec:pgorm:req:bindings.disjoint+1/test]
test("a disjoint model's describe() reports exactly the keys the surface declares", () => {
  const kind = new pgorm.TypeName("item_kind", { schema: "app" });
  const base = pgorm.model("items", {
    schema: "app",
    columns: {
      id: pgorm.column("i32", { primaryKey: true }),
      kind: pgorm.column(kind, { values: ["plain", "boxed"] }),
    },
  });
  const boxed = pgorm.model("boxes", { schema: "app", columns: { id: pgorm.column("i32", { primaryKey: true }) } });
  const items = pgorm.disjoint("item", base, { plain: null, boxed }, { schemaName: "app" });
  assertDeclared("pgorm.Disjoint", [items.describe()]);
});

// [spec:pgorm:req:bindings.surface+2/test]
// [spec:pgorm:req:bindings.disjoint+1/test]
test("a registered disjoint enum's describe() reports exactly the keys the surface declares", {
  skip: pgorm.disjointEntities().length === 0 &&
    "the module registers no disjoint entity; checks/entities.js runs this against an application's",
}, () => {
  assertDeclared("pgorm.DisjointEntity", pgorm.disjointEntities().map((name) => pgorm.disjointEntity(name).describe()));
});

// [spec:pgorm:req:bindings.surface+2/test]
// [spec:pgorm:req:bindings.entities+4/test]
test("a registered entity's describe() reports exactly the keys the surface declares", {
  skip: pgorm.entities().length === 0 && "the module registers no entity; checks/entities.js runs this against an application's",
}, () => {
  assertDeclared("pgorm.Entity", pgorm.entities().map((name) => pgorm.entity(name).describe()));
});

// [spec:pgorm:req:bindings.surface+2/test]
// [spec:pgorm:req:bindings.entity-graphs+3/test]
test("a registered graph's describe() reports exactly the keys the surface declares", {
  skip: pgorm.graphs().length === 0 && "the module registers no graph; checks/entities.js runs this against an application's",
}, () => {
  assertDeclared("pgorm.EntityGraph", pgorm.graphs().map((name) => pgorm.graph(name).describe()));
});

// [spec:pgorm:req:bindings.surface+2/test]
// [spec:pgorm:req:bindings.pipeline-sources+3/test]
test("a source tuple's describe() reports exactly the keys the surface declares", {
  skip: pgorm.pipeline.sourceTuples().length === 0 &&
    "the module registers no source tuple; checks/entities.js runs this against an application's",
}, () => {
  assertDeclared("pipeline.SourceSelection", pgorm.pipeline.sourceTuples().map((name) => pgorm.pipeline.sources(name).describe()));
});
