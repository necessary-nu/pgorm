// @ts-check

// Generated DDL frozen as a migration written as a `.sql` file, pgorm's
// `MigrationFile`: its text and the writing of its file are the addon's, so
// they are pgorm's exactly, and a file once written is never written again.
// [spec:pgorm:req:bindings.migration-file+1]

import { fileURLToPath } from "node:url";

import { TRUSTED } from "./builder.js";
import { native } from "./operations.js";
import { DDL } from "./schema.js";

/** The handle of the change pgorm wrote, which a kind change's object keeps. */
export const CHANGE = Symbol("pgorm-napi disjoint change");

/** Statements frozen as the text of a migration written as a `.sql` file. */
export class MigrationFile {
  /** @type {unknown} */
  #handle;

  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    if (token !== TRUSTED) throw new TypeError("a MigrationFile is made by migrationFile or migrationFiles");
    this.#handle = handle;
    const [name, text, commitsAfter] = /** @type {[string, string, boolean]} */ (native.migrationFileParts(handle));
    /** The file's stem, and the migration's name. */
    this.name = name;
    /** The file's text, which the migration runs whole. */
    this.text = text;
    /** Whether the text opens with `-- pgorm: commits-after`. */
    this.commitsAfter = commitsAfter;
    Object.freeze(this);
  }

  /**
   * Write `<name>.sql` into `directory` if no file of that name is there,
   * answering whether it wrote one: a file already there is never
   * rewritten.
   *
   * @param {string | URL} directory
   * @returns {boolean}
   */
  freeze(directory) {
    const path = directory instanceof URL ? fileURLToPath(directory) : directory;
    if (typeof path !== "string") throw new TypeError("a migration file freezes into a directory's path");
    return /** @type {boolean} */ (native.migrationFileFreeze(this.#handle, path));
  }
}

/**
 * `statements` frozen as the migration `name`, each ending `;` on a line of
 * its own, opening with `-- pgorm: commits-after` where `commitsAfter`.
 *
 * @param {string} name
 * @param {readonly DDL[]} statements
 * @param {{ commitsAfter?: boolean }} [options]
 */
export function migrationFile(name, statements, { commitsAfter = false } = {}) {
  if (typeof name !== "string") throw new TypeError("a migration's name is a string");
  if (!Array.isArray(statements) || !statements.every((statement) => statement instanceof DDL)) {
    throw new TypeError("a migration file freezes DDL statements");
  }
  const sql = statements.map((statement) => statement.inspect().sql);
  return new MigrationFile(native.migrationFile(name, sql, commitsAfter === true), TRUSTED);
}

/**
 * A kind change frozen as migration files, one per step in order, as pgorm's
 * `DisjointChange::migrationFiles` freezes it: named `name` for a change of
 * one step and `<name>_<n>` from 1 otherwise, every step but the last opening
 * with `-- pgorm: commits-after`, and a step leaving rows to the program
 * ending with a comment saying which.
 *
 * @param {import("./disjoint.d.ts").DisjointChange} change
 * @param {string} name
 */
export function migrationFiles(change, name) {
  const handle = change !== null && typeof change === "object"
    ? /** @type {{ [CHANGE]?: unknown }} */ (change)[CHANGE]
    : undefined;
  if (handle === undefined) throw new TypeError("migrationFiles takes a change a disjoint kind-change function returned");
  if (typeof name !== "string") throw new TypeError("a migration's name is a string");
  return /** @type {unknown[]} */ (native.disjointMigrationFiles(handle, name)).map((file) =>
    new MigrationFile(file, TRUSTED)
  );
}
