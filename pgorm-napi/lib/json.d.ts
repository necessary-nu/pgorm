/**
 * SQL/JSON's query functions, constructors, FORMAT JSON, IS JSON and
 * JSON_TABLE, and the column types their RETURNING and columns name.
 *
 * @module
 */

// [spec:pgorm:req:napi.typing+1]
// [spec:pgorm:req:bindings.sql-json]

import type { CreatedMultirange, CreatedRange, TypeName } from "./index.d.ts";
import type { Expr, Operand, OrderBy, Predicate } from "./expressions.d.ts";
import type { FromItem, Select } from "./select.d.ts";

/** The built-in types a {@link DataType} names, as pgorm-python names them. */
export type BuiltinType =
  | "char"
  | "varchar"
  | "text"
  | "smallint"
  | "integer"
  | "bigint"
  | "real"
  | "double"
  | "numeric"
  | "boolean"
  | "date"
  | "time"
  | "timestamp"
  | "timestamptz"
  | "interval"
  | "bytea"
  | "bit"
  | "varbit"
  | "money"
  | "json"
  | "jsonb"
  | "uuid"
  | "vector"
  | "cidr"
  | "inet"
  | "macaddr"
  | "ltree"
  | "int4range"
  | "int8range"
  | "numrange"
  | "daterange"
  | "tsrange"
  | "tstzrange"
  | "int4multirange"
  | "int8multirange"
  | "nummultirange"
  | "datemultirange"
  | "tsmultirange"
  | "tstzmultirange";

/**
 * A column type, pgorm-query's `ColumnType`: a built-in type, sized where it
 * takes a size — `length` for char, varchar, bit, varbit and vector,
 * `precision` (1–1000) and `scale` (0–1000) for numeric — an enum by
 * `TypeName`, or a range type a schema created.
 */
/** The field qualifiers an `interval` takes, `hourToMinute` for `HOUR TO MINUTE`. */
export type IntervalFields =
  | "year"
  | "month"
  | "day"
  | "hour"
  | "minute"
  | "second"
  | "yearToMonth"
  | "dayToHour"
  | "dayToMinute"
  | "dayToSecond"
  | "hourToMinute"
  | "hourToSecond"
  | "minuteToSecond";

export declare class DataType {
  constructor(
    kind: BuiltinType | TypeName | CreatedRange | CreatedMultirange,
    options?: {
      readonly length?: number;
      readonly precision?: number;
      readonly scale?: number;
      /** An interval's fields; its precision then belongs to a last field of seconds. */
      readonly fields?: IntervalFields;
    },
  );
  /** A domain over `base`, which a model column reads as its base and, over an enum, compares as the enum. */
  static domain(name: string | TypeName, base: DataType | BuiltinType | TypeName | CreatedRange | CreatedMultirange): DataType;
  array(): DataType;
}

/** A RETURNING or column type: a {@link DataType} or a built-in type's name. */
export type JsonType = DataType | BuiltinType;

/** PASSING variables: names, each an identifier, to values bound as operands are. */
export type Passing = { readonly [name: string]: Operand | JsonInput };

/** An operand read as JSON: any operand, or one marked {@link formatJson}. */
export type JsonOperand = Operand | JsonInput;

/** An operand marked as JSON text, `FORMAT JSON`, accepted only where SQL/JSON reads JSON. */
export declare class JsonInput {
  #private;
  private constructor();
}

/** `DEFAULT value` for ON EMPTY or ON ERROR, written as an escaped literal. */
export declare class JsonDefault {
  #private;
  private constructor();
}

/** JSON_EXISTS's ON ERROR. */
export type JsonExistsBehavior = "true" | "false" | "unknown" | "error";
/** JSON_VALUE's ON EMPTY and ON ERROR. */
export type JsonValueBehavior = "null" | "error" | JsonDefault;
/** JSON_QUERY's ON EMPTY and ON ERROR. */
export type JsonQueryBehavior = "null" | "error" | "emptyArray" | "emptyObject" | JsonDefault;
/** JSON_QUERY's one shaping slot: PostgreSQL refuses OMIT QUOTES beside a wrapper. */
export type JsonShaping = "withWrapper" | "withConditionalWrapper" | "omitQuotes";

export declare function formatJson(operand: Operand): JsonInput;
/** A literal DEFAULT; a value carrying an enum's or a created range's cast is refused. */
export declare function jsonDefault(value: Operand): JsonDefault;

/** The kind of JSON an IS JSON test accepts. */
export interface JsonTestOptions {
  readonly kind?: "value" | "scalar" | "array" | "object";
  readonly uniqueKeys?: boolean;
}

export declare function isJson(operand: Operand, options?: JsonTestOptions): Expr;
export declare function isNotJson(operand: Operand, options?: JsonTestOptions): Expr;

/** The path is bound as text cast to `jsonpath`; it never becomes statement text. */
export declare function jsonExists(
  context: JsonOperand,
  path: string,
  options?: { readonly passing?: Passing; readonly onError?: JsonExistsBehavior },
): Expr;

/** JSON_VALUE; it refuses to return `json` or `jsonb`, as PostgreSQL 18.6 mishandles them. */
export declare function jsonValue(
  context: JsonOperand,
  path: string,
  options?: {
    readonly passing?: Passing;
    readonly returning?: JsonType;
    readonly onEmpty?: JsonValueBehavior;
    readonly onError?: JsonValueBehavior;
  },
): Expr;

export declare function jsonQuery(
  context: JsonOperand,
  path: string,
  options?: {
    readonly passing?: Passing;
    readonly returning?: JsonType;
    readonly shaping?: JsonShaping;
    readonly onEmpty?: JsonQueryBehavior;
    readonly onError?: JsonQueryBehavior;
  },
): Expr;

/**
 * JSON_OBJECT. An object's keys are bound as text; `[key, value]` pairs take
 * any operand as a key, so a repeated one can meet `uniqueKeys`.
 */
export declare function jsonObject(
  entries?: { readonly [key: string]: JsonOperand } | readonly (readonly [Operand, JsonOperand])[],
  options?: { readonly absentOnNull?: boolean; readonly uniqueKeys?: boolean; readonly returning?: JsonType },
): Expr;

export declare function jsonArray(
  elements?: readonly JsonOperand[],
  options?: { readonly nullOnNull?: boolean; readonly returning?: JsonType },
): Expr;

/** `JSON_ARRAY(SELECT ..)`. */
export declare function jsonArrayQuery(query: Select, options?: { readonly returning?: JsonType }): Expr;

/** JSON_OBJECTAGG, which also runs over a window. */
export declare function jsonObjectagg(
  key: Operand,
  value: JsonOperand,
  options?: {
    readonly absentOnNull?: boolean;
    readonly uniqueKeys?: boolean;
    readonly returning?: JsonType;
    readonly filter?: Predicate;
  },
): Expr;

/** JSON_ARRAYAGG, which also runs over a window; its ordering takes no NULLS placement. */
export declare function jsonArrayagg(
  value: JsonOperand,
  options?: {
    readonly orderBy?: readonly OrderBy[];
    readonly nullOnNull?: boolean;
    readonly returning?: JsonType;
    readonly filter?: Predicate;
  },
): Expr;

/** `JSON(input)`. */
export declare function jsonParse(input: JsonOperand, options?: { readonly uniqueKeys?: boolean }): Expr;
/** JSON_SCALAR; the operand's type is carried, so a number stays a number. */
export declare function jsonScalar(operand: Operand): Expr;
export declare function jsonSerialize(input: JsonOperand, options?: { readonly returning?: JsonType }): Expr;

/** One column of a JSON_TABLE. */
export declare class JsonTableColumn {
  private constructor();
  /** `name FOR ORDINALITY`. */
  static ordinality(name: string): JsonTableColumn;
  /** The scalar its path — `$."name"` unless `path` says — finds, read as JSON_VALUE reads it. */
  static value(
    name: string,
    kind: JsonType,
    options?: { readonly path?: string; readonly onEmpty?: JsonValueBehavior; readonly onError?: JsonValueBehavior },
  ): JsonTableColumn;
  /** The JSON its path finds, read as JSON_QUERY reads it. */
  static query(
    name: string,
    kind: JsonType,
    options?: {
      readonly path?: string;
      readonly shaping?: JsonShaping;
      readonly onEmpty?: JsonQueryBehavior;
      readonly onError?: JsonQueryBehavior;
    },
  ): JsonTableColumn;
  /** Whether its path finds anything; finding nothing is false. */
  static exists(
    name: string,
    kind: JsonType,
    options?: { readonly path?: string; readonly onError?: JsonExistsBehavior },
  ): JsonTableColumn;
  /** `NESTED PATH path COLUMNS (..)`, joined to its parent row as an outer join would be. */
  static nested(
    path: string,
    columns: readonly [JsonTableColumn, ...JsonTableColumn[]],
    options?: { readonly pathName?: string },
  ): JsonTableColumn;
}

/**
 * `JSON_TABLE(..) AS alias`, a FROM item and implicitly LATERAL. Its paths are
 * written as escaped literals; a value they need goes through `passing`.
 */
export declare function jsonTable(
  context: JsonOperand,
  path: string,
  columns: readonly [JsonTableColumn, ...JsonTableColumn[]],
  options: {
    readonly alias: string;
    readonly passing?: Passing;
    readonly pathName?: string;
    readonly onError?: "error" | "empty";
  },
): FromItem;
