// @ts-check

// An application's build description, validated: the module to generate,
// the Rust crate holding its entities, and the entities, graphs, source
// tuples and disjoint enums to register, each under a registration name and a
// TypeScript export.
// [spec:pgorm:req:bindings.codegen]
// [spec:pgorm:req:bindings.codegen-disjoint]

import { existsSync, readFileSync, statSync } from "node:fs";
import { resolve } from "node:path";

/** The application's build description is invalid or incompatible. */
export class CodegenError extends Error {}

/** Words no generated export takes: JavaScript's reserved words, and what the generated module imports. */
const RESERVED = new Set([
  "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do",
  "else", "enum", "export", "extends", "false", "finally", "for", "function", "if", "implements", "import",
  "in", "instanceof", "interface", "let", "new", "null", "package", "private", "protected", "public",
  "return", "static", "super", "switch", "this", "throw", "true", "try", "typeof", "var", "void", "while",
  "with", "yield", "undefined", "NaN", "Infinity", "entity", "graph", "pipeline", "disjointEntity",
  "checkRegistrations",
]);

const RUST_KEYWORDS = new Set([
  "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false",
  "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
  "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
  "while",
]);

/**
 * @param {unknown} value
 * @param {string} what
 * @returns {string}
 */
export function identifier(value, what) {
  if (typeof value !== "string" || !/^[A-Za-z$][A-Za-z0-9_$]*$/.test(value) || RESERVED.has(value)) {
    throw new CodegenError(`${what} is a public JavaScript identifier, not ${JSON.stringify(value)}`);
  }
  return value;
}

/**
 * A path to an item of the entity crate: named modules and an item, no
 * expression, generic or keyword.
 *
 * @param {unknown} value
 * @returns {string}
 */
export function rustPath(value) {
  if (typeof value !== "string" || !/^[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*$/.test(value)) {
    throw new CodegenError(`a Rust path names modules and an item, without expressions or generics: ${JSON.stringify(value)}`);
  }
  if (value.split("::").some((part) => RUST_KEYWORDS.has(part))) {
    throw new CodegenError(`a Rust path is relative to the entity crate and names no keyword: ${JSON.stringify(value)}`);
  }
  return value;
}

/**
 * @param {unknown} value
 * @param {string} what
 */
function registration(value, what) {
  if (typeof value !== "string" || value.length === 0 || new TextEncoder().encode(value).length > 255 || value.includes("\0")) {
    throw new CodegenError(`${what}'s registration name is 1–255 UTF-8 bytes without NUL`);
  }
  return value;
}

/**
 * @param {unknown} list
 * @param {string} what
 * @returns {Record<string, unknown>[]}
 */
function entries(list, what) {
  if (list === undefined) return [];
  if (!Array.isArray(list) || !list.every((item) => typeof item === "object" && item !== null && !Array.isArray(item))) {
    throw new CodegenError(`${what} is a list of objects`);
  }
  return list;
}

/**
 * @param {Record<string, unknown>} entry
 * @param {readonly string[]} keys
 * @param {string} what
 */
function only(entry, keys, what) {
  for (const key of Object.keys(entry)) {
    if (!keys.includes(key)) throw new CodegenError(`${JSON.stringify(key)} is no field of ${what}`);
  }
}

/**
 * A validated build description.
 *
 * @typedef {object} Description
 * @property {1} schema_version
 * @property {string} module
 * @property {string} entity_crate The entity crate's directory, absolute.
 * @property {{ name: string, rust: string, typescript: string }[]} entities
 * @property {{ name: string, rust: string, typescript: string }[]} graphs
 * @property {{ name: string, rust: string[], typescript: string }[]} sources
 * @property {{ name: string, rust: string, typescript: string }[]} disjoints
 */

/**
 * A validated description: names unique per kind, exports unique across the
 * module, every Rust path well formed, the entity crate a directory with a
 * Cargo.toml, relative to `base`.
 *
 * @param {unknown} config
 * @param {string} base
 * @returns {Description}
 */
export function validate(config, base) {
  if (typeof config !== "object" || config === null || Array.isArray(config)) {
    throw new CodegenError("the build description is an object");
  }
  const raw = /** @type {Record<string, unknown>} */ (config);
  only(raw, ["schema_version", "module", "entity_crate", "entities", "graphs", "sources", "disjoints"], "the build description");
  if (raw.schema_version !== 1) throw new CodegenError("schema_version is 1");
  const module = identifier(raw.module, "module");
  if (typeof raw.entity_crate !== "string") throw new CodegenError("entity_crate is a path");
  const crate = resolve(base, raw.entity_crate);
  if (!existsSync(resolve(crate, "Cargo.toml")) || !statSync(crate).isDirectory()) {
    throw new CodegenError(`entity_crate names no Cargo crate: ${crate}`);
  }
  const exports = new Set();
  /** @param {unknown} name @param {string} what */
  const exported = (name, what) => {
    const value = identifier(name, what);
    if (exports.has(value) || value === module) throw new CodegenError(`the export ${value} is named twice`);
    exports.add(value);
    return value;
  };
  /**
   * @param {unknown} list
   * @param {string} what
   * @param {(entry: Record<string, unknown>) => unknown} rust
   */
  const kind = (list, what, rust) => {
    const names = new Set();
    return entries(list, what).map((entry) => {
      only(entry, ["name", "rust", "typescript"], `a ${what} entry`);
      const name = registration(entry.name, `a ${what} entry`);
      if (names.has(name)) throw new CodegenError(`the ${what} registration ${name} is named twice`);
      names.add(name);
      return { name, rust: rust(entry), typescript: exported(entry.typescript, `${name}'s TypeScript export`) };
    });
  };
  const entities = kind(raw.entities, "entities", (entry) => rustPath(entry.rust));
  const rustEntities = new Set(entities.map((entry) => entry.rust));
  if (rustEntities.size !== entities.length) throw new CodegenError("each Rust entity is registered once");
  const graphs = kind(raw.graphs, "graphs", (entry) => rustPath(entry.rust));
  const sources = kind(raw.sources, "sources", (entry) => {
    if (!Array.isArray(entry.rust) || entry.rust.length < 1 || entry.rust.length > 6) {
      throw new CodegenError("a source tuple lists one to six Rust entities");
    }
    return entry.rust.map((path) => {
      const checked = rustPath(path);
      if (!rustEntities.has(checked)) throw new CodegenError(`the source entity ${checked} is not among the entities`);
      return checked;
    });
  });
  const disjoints = kind(raw.disjoints, "disjoints", (entry) => rustPath(entry.rust));
  if (new Set(disjoints.map((entry) => entry.rust)).size !== disjoints.length) {
    throw new CodegenError("each Rust disjoint enum is registered once");
  }
  return {
    schema_version: 1,
    module,
    entity_crate: crate,
    entities: /** @type {Description["entities"]} */ (entities),
    graphs: /** @type {Description["graphs"]} */ (graphs),
    sources: /** @type {Description["sources"]} */ (sources),
    disjoints: /** @type {Description["disjoints"]} */ (disjoints),
  };
}

/**
 * A build description read from a JSON file, its paths relative to the file.
 *
 * @param {string} path
 */
export function load(path) {
  let config;
  try {
    config = JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    throw new CodegenError(`${path} is no JSON build description: ${/** @type {Error} */ (error).message}`);
  }
  return validate(config, resolve(path, ".."));
}
