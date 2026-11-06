// @ts-check

// Write an application's module project: a Rust crate that registers the
// described entities, graphs and source tuples and builds one native library
// with the binding's API, beside a copy of the binding's ES module that loads
// it. Nothing is built and no database is reached.
// [spec:pgorm:req:bindings.codegen]
// [spec:pgorm:req:napi.codegen+1]

import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { CodegenError } from "./config.js";

/** The binding's own ES module, which a project copies. */
const FACADE = join(dirname(dirname(fileURLToPath(import.meta.url))), "lib");

/** The project's build description, as scaffolding resolved it. */
export const MARKER = "application.json";

/**
 * The string values of a Cargo manifest's sections: `section.key` for
 * `key = "value"`, and `section.key.version` for an inline table's version.
 *
 * @param {string} path
 */
export function manifest(path) {
  /** @type {Map<string, string>} */
  const values = new Map();
  let section = "";
  for (const line of readFileSync(path, "utf8").split("\n")) {
    const header = /^\s*\[([^\]]+)\]\s*$/.exec(line);
    if (header) {
      section = /** @type {string} */ (header[1]).trim();
      continue;
    }
    const plain = /^\s*([A-Za-z0-9_-]+)\s*=\s*"([^"]*)"/.exec(line);
    if (plain) values.set(`${section}.${plain[1]}`, /** @type {string} */ (plain[2]));
    const table = /^\s*([A-Za-z0-9_-]+)\s*=\s*\{.*\bversion\s*=\s*"([^"]*)"/.exec(line);
    if (table) values.set(`${section}.${table[1]}.version`, /** @type {string} */ (table[2]));
  }
  return values;
}

/**
 * @param {Map<string, string>} values
 * @param {string} key
 * @param {string} path
 */
function required(values, key, path) {
  const value = values.get(key);
  if (value === undefined) throw new CodegenError(`${path} has no ${key}`);
  return value;
}

/**
 * The project's Rust crate: its manifest and the registration module.
 *
 * @param {import("./config.js").Description} description
 * @param {string} source
 */
function crate(description, source) {
  const napi = manifest(join(source, "pgorm-napi", "Cargo.toml"));
  const entities = manifest(join(description.entity_crate, "Cargo.toml"));
  const version = required(napi, "package.version", "pgorm-napi's Cargo.toml");
  const neon = required(napi, "dependencies.neon.version", "pgorm-napi's Cargo.toml");
  const name = required(entities, "package.name", "the entity crate's Cargo.toml");
  const library = (entities.get("lib.name") ?? name).replaceAll("-", "_");
  const toml = [
    "[package]",
    `name = ${JSON.stringify(`${description.module}-napi`)}`,
    `version = ${JSON.stringify(version)}`,
    'edition = "2024"',
    "publish = false",
    'license = "MIT OR Apache-2.0"',
    "",
    "[workspace]",
    "",
    "[lib]",
    `name = ${JSON.stringify(`${description.module}_napi`)}`,
    'crate-type = ["cdylib"]',
    "",
    "[dependencies]",
    `pgorm-napi = { path = ${JSON.stringify(join(source, "pgorm-napi"))}, default-features = false }`,
    `pgorm = { path = ${JSON.stringify(source)} }`,
    `neon = { version = ${JSON.stringify(neon)}, default-features = false, features = ["napi-6"] }`,
    `${name} = { path = ${JSON.stringify(description.entity_crate)} }`,
    "",
  ].join("\n");
  const at = (/** @type {string} */ path) => `entities::${path}`;
  const lines = [
    `//! The native module of \`${description.module}\`, written by pgorm-napi's codegen:`,
    "//! the binding's API and the application's registrations, in one library.",
    "",
    "use neon::prelude::*;",
    "use pgorm_napi::{RegistrationError, Registry};",
    `use ${library} as entities;`,
    "",
    "fn registry() -> Result<Registry, RegistrationError> {",
    "    let mut registry = Registry::default();",
    ...description.entities.map((entry) => `    registry.entity::<${at(entry.rust)}>(${JSON.stringify(entry.name)})?;`),
    ...description.disjoints.map((entry) => `    registry.disjoint::<${at(entry.rust)}>(${JSON.stringify(entry.name)})?;`),
    // [spec:pgorm:req:bindings.codegen-coverage]
    ...description.oneOfs.map((entry) => `    registry.one_of::<${at(entry.rust)}>(${JSON.stringify(entry.name)})?;`),
    ...description.graphs.map((entry) => `    registry.graph(${JSON.stringify(entry.name)}, ${at(entry.rust)})?;`),
    ...description.sources.map((entry) =>
      `    registry.sources::<(${entry.rust.map(at).join(", ")},)>(${JSON.stringify(entry.name)})?;`
    ),
    "    Ok(registry)",
    "}",
    "",
    "#[neon::main]",
    "fn main(mut cx: ModuleContext) -> NeonResult<()> {",
    "    match registry() {",
    "        Ok(registry) => pgorm_napi::install(&mut cx, registry),",
    "        Err(error) => cx.throw_error(error.to_string()),",
    "    }",
    "}",
    "",
  ];
  return { toml, lib: lines.join("\n"), version };
}

/**
 * Copy the binding's ES module into `lib`, its native library left out.
 *
 * @param {string} lib
 */
function facade(lib) {
  mkdirSync(lib, { recursive: true });
  for (const file of readdirSync(FACADE)) {
    if (/\.(js|d\.ts)$/.test(file)) copyFileSync(join(FACADE, file), join(lib, file));
  }
}

/**
 * Write the project for `description` at `destination`, which must not
 * exist, its crate depending on the checkout at `source`.
 *
 * @param {import("./config.js").Description} description
 * @param {string} destination
 * @param {{ source: string }} options
 */
export function scaffold(description, destination, { source }) {
  const target = resolve(destination);
  const checkout = resolve(source);
  if (existsSync(target)) throw new CodegenError(`${target} exists: scaffold into a new directory`);
  if (!existsSync(join(checkout, "pgorm-napi", "Cargo.toml"))) {
    throw new CodegenError(`${checkout} is no pgorm checkout with pgorm-napi`);
  }
  const written = crate(description, checkout);
  mkdirSync(join(target, "src"), { recursive: true });
  writeFileSync(join(target, "Cargo.toml"), written.toml);
  writeFileSync(join(target, "src", "lib.rs"), written.lib);
  // The binding's own lockfile seeds the project's, so the crates it shares
  // with the binding resolve to the versions the binding is audited at.
  copyFileSync(join(checkout, "pgorm-napi", "Cargo.lock"), join(target, "Cargo.lock"));
  facade(join(target, "lib"));
  writeFileSync(join(target, "package.json"), `${JSON.stringify({ name: `${description.module}-napi`, private: true, type: "module" }, null, 2)}\n`);
  writeFileSync(join(target, "deno.json"), `${JSON.stringify({ nodeModulesDir: "none" }, null, 2)}\n`);
  writeFileSync(
    join(target, MARKER),
    `${JSON.stringify({ ...description, pgorm_source: checkout, version: written.version }, null, 2)}\n`,
  );
  return target;
}
