// @ts-check

// The SQL types SQL/JSON's RETURNING and JSON_TABLE's columns name: a built-in
// type by name, with the size it takes, an enum by `TypeName`, or a range type
// a schema created.
// [spec:pgorm:req:bindings.sql-json]

import { arg, Handle, TRUSTED } from "./builder.js";
import { native } from "./operations.js";

/** A column type: pgorm-query's `ColumnType`. */
export class DataType extends Handle {
  /**
   * @param {string | import("./values.js").TypeName | import("./values.js").CreatedRange | import("./values.js").CreatedMultirange} kind
   * @param {{ length?: number, precision?: number, scale?: number, fields?: string }} [options]
   * @param {symbol} [token]
   */
  constructor(kind, options = {}, token) {
    if (token === TRUSTED) {
      super(kind);
      return;
    }
    const { length, precision, scale, fields } = options;
    super(native.dataTypeNew(kind, length, precision, scale, fields));
  }

  /**
   * A domain over `base`: a column of it is written as the domain, read as
   * its base and, a domain over an enum, compared as the enum.
   *
   * @param {string | import("./values.js").TypeName} name
   * @param {unknown} base
   */
  // [spec:pgorm:req:bindings.model-casts]
  static domain(name, base) {
    return new DataType(/** @type {any} */ (native.dataTypeDomain(name, arg(base))), {}, TRUSTED);
  }

  /** The array of this type. */
  array() {
    return new DataType(/** @type {any} */ (native.dataTypeArray(arg(this))), {}, TRUSTED);
  }
}
