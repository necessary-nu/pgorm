// @ts-check

// What a module generated for an application's registrations holds it to:
// the fingerprint of the addon's version and every registration it names, as
// the module's own entity(), graph() and pipeline sources() describe them.
// The generator computes it as it writes the module, and the module checks
// it as it loads, so a module meeting a native build its declarations no
// longer describe fails at once rather than mistyping a record.
// [spec:pgorm:req:bindings.codegen]
// [spec:pgorm:req:bindings.codegen-disjoint]

import { createHash } from "node:crypto";

import { ConstructionError, disjointEntity, entity, graph, pipeline, version } from "./index.js";

/**
 * The registrations a generated module names, by kind.
 *
 * @typedef {{ entities: readonly string[], graphs: readonly string[], sources: readonly string[], disjoints?: readonly string[] }} Named
 */

/**
 * Each named registration's description, as the module describes it.
 *
 * @param {Named} named
 */
export function describeRegistrations(named) {
  return {
    version,
    entities: named.entities.map((name) => entity(name).describe()),
    graphs: named.graphs.map((name) => graph(name).describe()),
    sources: named.sources.map((name) => pipeline.sources(name).describe()),
    disjoints: (named.disjoints ?? []).map((name) => disjointEntity(name).describe()),
  };
}

/**
 * The SHA-256 of the descriptions, in hex.
 *
 * @param {Named} named
 */
export function registrationsFingerprint(named) {
  return createHash("sha256").update(JSON.stringify(describeRegistrations(named))).digest("hex");
}

/**
 * Refuse to go on when the module's registrations are not the ones a
 * generated module was written for.
 *
 * @param {Named} named
 * @param {string} expected
 * @param {string} module
 */
export function checkRegistrations(named, expected, module) {
  const actual = registrationsFingerprint(named);
  if (actual !== expected) {
    throw new ConstructionError(
      `the native module's registrations are not the ones ${module} was generated for: regenerate it with pgorm-napi's codegen`,
    );
  }
}
