// @ts-check

// Views, recursive views and materialized views over a Select, the refresh
// and drops a materialized view takes, and CREATE TABLE AS. Each body is a
// Select, written with its values as pgorm-query's escaped literals: a
// definition is stored as SQL text and binds nothing.
// [spec:pgorm:req:bindings.schema-views+3]

import { arg, TRUSTED } from "./builder.js";
import { native } from "./operations.js";
import { DDL, relations, statement } from "./schema.js";

/**
 * @typedef {import("./schema.js").TableRef} TableRef
 * @typedef {import("./select.js").Select} Select
 */

/** @type {(handle: unknown) => CreateView} */
let viewOf;

/** @type {(handle: unknown) => CreateRecursiveView} */
let recursiveOf;

/** @type {(handle: unknown) => CreateMaterializedView} */
let materializedOf;

/** @type {(handle: unknown) => CreateTableAs} */
let tableAsOf;

/**
 * The options a plain view and a recursive one share; each method gives a new
 * statement of the receiver's kind.
 *
 * @template {new (...args: any[]) => DDL} B
 * @param {B} Base
 * @param {() => (handle: unknown) => any} of
 */
function viewOptions(Base, of) {
  return class extends Base {
    orReplace() {
      return of()(native.ddlViewOrReplace(arg(this)));
    }

    /** @param {readonly string[]} names */
    columns(names) {
      return of()(native.ddlViewColumns(arg(this), names));
    }

    securityBarrier() {
      return of()(native.ddlViewSecurityBarrier(arg(this)));
    }

    securityInvoker() {
      return of()(native.ddlViewSecurityInvoker(arg(this)));
    }
  };
}

/** `CREATE VIEW`; every method returns a new statement. */
export class CreateView extends viewOptions(DDL, () => viewOf) {
  static {
    viewOf = (handle) => new CreateView(handle, TRUSTED);
  }

  /** @param {"local" | "cascaded"} option */
  checkOption(option) {
    return viewOf(native.ddlViewCheckOption(arg(this), option));
  }
}

/** `CREATE RECURSIVE VIEW`, which takes no check option; every method returns a new statement. */
export class CreateRecursiveView extends viewOptions(DDL, () => recursiveOf) {
  static {
    recursiveOf = (handle) => new CreateRecursiveView(handle, TRUSTED);
  }
}

/**
 * The clauses a materialized view and `CREATE TABLE AS` share.
 *
 * @template {new (...args: any[]) => DDL} B
 * @param {B} Base
 * @param {() => (handle: unknown) => any} of
 */
function relationOptions(Base, of) {
  return class extends Base {
    ifNotExists() {
      return of()(native.ddlRelationIfNotExists(arg(this)));
    }

    /** @param {readonly string[]} names */
    columns(names) {
      return of()(native.ddlViewColumns(arg(this), names));
    }

    withNoData() {
      return of()(native.ddlRelationWithNoData(arg(this)));
    }
  };
}

/** `CREATE MATERIALIZED VIEW`; every method returns a new statement. */
export class CreateMaterializedView extends relationOptions(DDL, () => materializedOf) {
  static {
    materializedOf = (handle) => new CreateMaterializedView(handle, TRUSTED);
  }
}

/** `CREATE TABLE AS`; every method returns a new statement. */
export class CreateTableAs extends relationOptions(DDL, () => tableAsOf) {
  static {
    tableAsOf = (handle) => new CreateTableAs(handle, TRUSTED);
  }
}

/**
 * @param {TableRef} name
 * @param {Select} query
 */
export function createView(name, query) {
  return viewOf(native.ddlCreateView(arg(name), arg(query)));
}

/**
 * @param {TableRef} name
 * @param {string | readonly string[]} columns
 * @param {Select} query
 */
export function createRecursiveView(name, columns, query) {
  return recursiveOf(native.ddlCreateRecursiveView(arg(name), columns, arg(query)));
}

/**
 * @param {TableRef | readonly TableRef[]} names
 * @param {object} [options]
 */
export function dropView(names, options) {
  return statement(native.ddlDropView(relations(names), options));
}

/**
 * @param {TableRef} name
 * @param {Select} query
 */
export function createMaterializedView(name, query) {
  return materializedOf(native.ddlCreateMaterializedView(arg(name), arg(query)));
}

/**
 * @param {TableRef} name
 * @param {{ concurrently?: boolean, withNoData?: boolean }} [options]
 */
export function refreshMaterializedView(name, options) {
  return statement(native.ddlRefreshMaterializedView(arg(name), options));
}

/**
 * @param {TableRef | readonly TableRef[]} names
 * @param {object} [options]
 */
export function dropMaterializedView(names, options) {
  return statement(native.ddlDropMaterializedView(relations(names), options));
}

/**
 * @param {TableRef} name
 * @param {Select} query
 */
export function createTableAs(name, query) {
  return tableAsOf(native.ddlCreateTableAs(arg(name), arg(query)));
}
