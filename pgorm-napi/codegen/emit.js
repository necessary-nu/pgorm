// @ts-check

// Write a project's typed module from its built native library: one export
// per registration — an entity, a graph shape, a source tuple, a disjoint
// enum — and declarations typing each from what the library describes, its
// records' columns, their nullability and an enum's labels, a graph's, a
// source tuple's and a disjoint enum's rows. The module checks the library
// against what it was written from as it loads.
// [spec:pgorm:req:bindings.codegen]
// [spec:pgorm:req:bindings.codegen-disjoint]
// [spec:pgorm:req:napi.codegen+1]
// [spec:pgorm:req:napi.codegen-types]

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

import { CodegenError } from "./config.js";
import { MARKER } from "./scaffold.js";
import { columnType, propertyKey } from "./types.js";

const HEADER = "// Written by pgorm-napi's codegen from the application's native module; regenerate rather than edit.";

/**
 * @typedef {{ name: string, kind: string | null, nullable: boolean, values?: readonly string[] | null }} Column
 * @typedef {{ name: string, columns: readonly Column[] }} EntityDescription
 */

/**
 * The record and input interfaces of one entity.
 *
 * @param {string} name
 * @param {string} exported
 * @param {EntityDescription} description
 */
function entityTypes(name, exported, description) {
  const fields = (/** @type {boolean} */ write) =>
    description.columns.map((column) => `  readonly ${propertyKey(column.name)}: ${columnType(column, write)};`);
  return [
    `/** A record of \`${name}\`: its columns by SQL name, as its reads and writes give them. */`,
    `export interface ${exported}Record {`,
    ...fields(false),
    "}",
    "",
    `/** What each column of \`${name}\` takes, written or compared. */`,
    `export interface ${exported}Input {`,
    ...fields(true),
    "}",
    "",
    `/** The entity registered as \`${name}\`. */`,
    `export declare const ${exported}: Entity<{ record: ${exported}Record; input: ${exported}Input }>;`,
    "",
  ];
}

/**
 * The record type of the entity registered as `name`.
 *
 * @param {Map<string, string>} exports
 * @param {string} name
 * @param {string} what
 */
function recordOf(exports, name, what) {
  const exported = exports.get(name);
  if (exported === undefined) throw new CodegenError(`${what} reads ${name}, which the build description does not export`);
  return `${exported}Record`;
}

/**
 * A registered disjoint enum's row type, a union over its kinds, and its
 * export: a kinded enum's writes take its entities' ActiveModels, and an
 * unkinded one, which has no kind field, is read-only.
 *
 * @param {Map<string, string>} exports
 * @param {{ name: string, typescript: string }} entry
 * @param {{ kindField: string | null, base: string, kinds: { value: string, model: string | null }[] }} described
 */
function disjointTypes(exports, entry, described) {
  const exported = (/** @type {string} */ name) => {
    const found = exports.get(name);
    if (found === undefined) throw new CodegenError(`${entry.name} reads ${name}, which the build description does not export`);
    return found;
  };
  const base = exported(described.base);
  const kinds = described.kinds.map(({ value, model }) => [value, model === null ? null : exported(model)]);
  const types = (/** @type {string} */ name) => `{ record: ${name}Record; input: ${name}Input }`;
  const row = `${entry.typescript}Row`;
  const values = kinds.map(([value]) => JSON.stringify(value)).join(" | ");
  const actives = kinds.filter(([, model]) => model !== null).map(([, model]) => `ActiveModel<${types(/** @type {string} */ (model))}>`);
  const view = described.kindField === null
    ? `UnkindedDisjointEntity<${row}, ${values}>`
    : `DisjointEntity<${row}, ${values}, ActiveModel<${types(base)}>, ${actives.join(" | ") || "never"}>`;
  return [
    `/** A row of the disjoint enum \`${entry.name}\`: its base's record, its kind and the kind's record, or null for a kind with no table. */`,
    `export type ${row} =`,
    ...kinds.map(([value, model], index) =>
      `  | [${base}Record, ${JSON.stringify(value)}, ${model === null ? "null" : `${model}Record`}]${index === kinds.length - 1 ? ";" : ""}`
    ),
    `/** The disjoint enum registered as \`${entry.name}\`${described.kindField === null ? ", read-only" : ""}. */`,
    `export declare const ${entry.typescript}: ${view};`,
    "",
  ];
}

/**
 * Write the project's module from its built library.
 *
 * @param {string} project
 */
export async function emit(project) {
  const metadata = join(project, MARKER);
  if (!existsSync(metadata)) throw new CodegenError(`${project} is no scaffolded project`);
  const description = JSON.parse(readFileSync(metadata, "utf8"));
  const lib = join(project, "lib");
  if (!existsSync(join(lib, "pgorm_napi.node"))) {
    throw new CodegenError(`${project} has no native library: build it before emitting`);
  }
  const registrations = await import(pathToFileURL(join(lib, "registrations.js")).href);
  /** @type {{ entities: { name: string, rust: string, typescript: string }[], graphs: { name: string, typescript: string }[], sources: { name: string, typescript: string }[], disjoints?: { name: string, typescript: string }[] }} */
  const { entities, graphs, sources, disjoints = [] } = description;
  const named = {
    entities: entities.map((entry) => entry.name),
    graphs: graphs.map((entry) => entry.name),
    sources: sources.map((entry) => entry.name),
    disjoints: disjoints.map((entry) => entry.name),
  };
  const described = registrations.describeRegistrations(named);
  const fingerprint = registrations.registrationsFingerprint(named);
  const exports = new Map(entities.map((entry) => [entry.name, entry.typescript]));
  /** @type {string[]} */
  const declarations = [];
  entities.forEach((entry, index) => declarations.push(...entityTypes(entry.name, entry.typescript, described.entities[index])));
  graphs.forEach((entry, index) => {
    /** @type {{ entity: string, slot: string }[]} */
    const slots = described.graphs[index].sources;
    const records = slots.map((source) => {
      const record = recordOf(exports, source.entity, entry.name);
      return source.slot === "Opt" ? `${record} | null` : record;
    });
    const row = records.length === 1 ? records[0] : `[${records.join(", ")}]`;
    declarations.push(
      `/** A row of the graph \`${entry.name}\`: ${records.length === 1 ? "its root's record" : "each source's record, an absent optional slot null"}. */`,
      `export type ${entry.typescript}Row = ${row};`,
      `/** The graph shape registered as \`${entry.name}\`. */`,
      `export declare const ${entry.typescript}: EntityGraph<${entry.typescript}Row>;`,
      "",
    );
  });
  sources.forEach((entry, index) => {
    /** @type {string[]} */
    const listed = described.sources[index].entities;
    const records = listed.map((name) => `${recordOf(exports, name, entry.name)} | null`);
    declarations.push(
      `/** A row the source tuple \`${entry.name}\` selects: each source's record, or null where the row lacks it. */`,
      `export type ${entry.typescript}Row = [${records.join(", ")}];`,
      `/** The source tuple registered as \`${entry.name}\`, for a pipeline's selectSources. */`,
      `export declare const ${entry.typescript}: SourceSelection<${entry.typescript}Row>;`,
      "",
    );
  });
  disjoints.forEach((entry, index) => declarations.push(...disjointTypes(exports, entry, described.disjoints[index])));
  const body = declarations.join("\n");
  const used = (/** @type {string[]} */ names) => names.filter((name) => new RegExp(`\\b${name}\\b`).test(body));
  const types = used([
    "ActiveModel", "Decimal", "DisjointEntity", "Entity", "EntityGraph", "Interval", "JsonValue", "Multirange", "PlainValue",
    "Range", "UnkindedDisjointEntity", "Uuid",
  ]);
  const imports = [`import type { ${types.join(", ")} } from "./index.js";`];
  if (sources.length > 0) imports.push('import type { SourceSelection } from "./pipeline.d.ts";');
  const module = description.module;
  const code = [
    `// @ts-self-types="./${module}.d.ts"`,
    HEADER,
    "",
    'import { disjointEntity, entity, graph, pipeline } from "./index.js";',
    'import { checkRegistrations } from "./registrations.js";',
    "",
    `checkRegistrations(${JSON.stringify(named)}, ${JSON.stringify(fingerprint)}, ${JSON.stringify(module)});`,
    "",
    ...entities.map((entry) => `export const ${entry.typescript} = entity(${JSON.stringify(entry.name)});`),
    ...graphs.map((entry) => `export const ${entry.typescript} = graph(${JSON.stringify(entry.name)});`),
    ...sources.map((entry) => `export const ${entry.typescript} = pipeline.sources(${JSON.stringify(entry.name)});`),
    ...disjoints.map((entry) => `export const ${entry.typescript} = disjointEntity(${JSON.stringify(entry.name)});`),
    "",
  ];
  writeFileSync(join(lib, `${module}.d.ts`), `${[HEADER, "", ...imports, "", body.trimEnd()].join("\n")}\n`);
  writeFileSync(join(lib, `${module}.js`), code.join("\n"));
  return join(lib, `${module}.js`);
}
