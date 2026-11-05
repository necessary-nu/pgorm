// @ts-check

// pgorm's values in JavaScript: the classes for the kinds plain JavaScript has
// no type for, the `Value` that declares a kind explicitly, and the two
// functions the addon reads and builds them with. Every judgement about a
// value — its kind, its range, its precision — is the addon's; these classes
// hold what it accepted.
// [spec:pgorm:req:bindings.values]
// [spec:pgorm:req:napi.values+1]
// [spec:pgorm:req:bindings.value-tags+1]
// [spec:pgorm:req:napi.temporal+1]

import { ConstructionError, InternalError } from "./errors.js";

/** @type {any} */
let native;

/** How Node.js's and Deno's consoles ask an object to show itself. */
const INSPECT = Symbol.for("nodejs.util.inspect.custom");

/** Marks a construction from data the addon has already checked. */
const TRUSTED = Symbol("pgorm-napi checked value");

/**
 * Hand the addon the functions it reads and builds values with.
 *
 * @param {any} addon
 */
export function install(addon) {
  native = addon;
  addon.setCodec(describe, make);
}

const encoder = new TextEncoder();

/**
 * Whether `part` is a name PostgreSQL holds as an identifier: 1–63 UTF-8
 * bytes without NUL.
 *
 * @param {string} part
 */
export function isIdentifier(part) {
  const length = encoder.encode(part).length;
  return length > 0 && length <= 63 && !part.includes("\0");
}

/**
 * @param {unknown} part
 * @param {string} what
 * @returns {string}
 */
function namePart(part, what) {
  if (typeof part !== "string") {
    throw new TypeError(`a type's ${what} is a string`);
  }
  if (!isIdentifier(part)) {
    throw new ConstructionError("type name parts are 1–63 UTF-8 bytes without NUL");
  }
  return part;
}

/** @param {string} part */
function quoted(part) {
  return `"${part.replaceAll('"', '""')}"`;
}

/** A PostgreSQL type named by identifier, optionally schema-qualified: an enum's type. */
export class TypeName {
  /**
   * @param {string} name
   * @param {{ schema?: string | null }} [options]
   */
  constructor(name, { schema = null } = {}) {
    /** @readonly */
    this.name = namePart(name, "name");
    /** @readonly @type {string | null} */
    this.schema = schema === null || schema === undefined ? null : namePart(schema, "schema");
    Object.freeze(this);
  }

  toString() {
    return this.schema === null ? quoted(this.name) : `${quoted(this.schema)}.${quoted(this.name)}`;
  }
}

const SUBTYPES = new Set([
  "i16",
  "i32",
  "i64",
  "f32",
  "f64",
  "decimal",
  "text",
  "date",
  "time",
  "datetime",
  "datetime_utc",
  "uuid",
]);

/**
 * @param {unknown} subtype
 * @returns {string}
 */
function subtypeKind(subtype) {
  if (typeof subtype !== "string") throw new TypeError("a created range's subtype is a kind name");
  if (!SUBTYPES.has(subtype)) {
    throw new ConstructionError(
      "a created range's subtype is one of i16, i32, i64, f32, f64, decimal, text, date, time, datetime, datetime_utc or uuid",
    );
  }
  return subtype;
}

/** A range type a schema created with `CREATE TYPE ... AS RANGE`, and the kind of its subtype. */
export class CreatedRange {
  /**
   * @param {string} name
   * @param {string} subtype
   * @param {{ schema?: string | null }} [options]
   */
  constructor(name, subtype, { schema = null } = {}) {
    /** @readonly */
    this.name = namePart(name, "name");
    /** @readonly */
    this.subtype = subtypeKind(subtype);
    /** @readonly @type {string | null} */
    this.schema = schema === null || schema === undefined ? null : namePart(schema, "schema");
    Object.freeze(this);
  }
}

/** The multirange type PostgreSQL creates beside a created range type, named by its own name. */
export class CreatedMultirange {
  /**
   * @param {string} name
   * @param {string} subtype
   * @param {{ schema?: string | null }} [options]
   */
  constructor(name, subtype, { schema = null } = {}) {
    /** @readonly */
    this.name = namePart(name, "name");
    /** @readonly */
    this.subtype = subtypeKind(subtype);
    /** @readonly @type {string | null} */
    this.schema = schema === null || schema === undefined ? null : namePart(schema, "schema");
    Object.freeze(this);
  }
}

/** An exact decimal, PostgreSQL's `numeric` within pgorm's 96-bit coefficient and 28 fractional digits. */
export class Decimal {
  /** @type {string} */
  #text;

  /**
   * @param {string | bigint} value
   * @param {symbol} [trusted]
   */
  constructor(value, trusted) {
    if (trusted === TRUSTED && typeof value === "string") {
      this.#text = value;
      return;
    }
    if (typeof value === "bigint") value = value.toString();
    if (typeof value !== "string") {
      throw new ConstructionError(
        "a Decimal is made from a decimal string or a bigint, never a number, which may already be rounded",
      );
    }
    this.#text = native.decimalText(value);
  }

  toString() {
    return this.#text;
  }

  toJSON() {
    return this.#text;
  }

  [INSPECT]() {
    return `Decimal(${this.#text})`;
  }

  /** @param {string} hint */
  [Symbol.toPrimitive](hint) {
    if (hint === "number") {
      throw new TypeError(
        "a Decimal does not become a number implicitly, which could round it: convert its toString() explicitly",
      );
    }
    return this.#text;
  }
}

/** A UUID, held as its lower-case hyphenated text. */
export class Uuid {
  /** @type {string} */
  #text;

  /**
   * @param {string} value
   * @param {symbol} [trusted]
   */
  constructor(value, trusted) {
    if (trusted === TRUSTED && typeof value === "string") {
      this.#text = value;
      return;
    }
    if (typeof value !== "string") throw new ConstructionError("a Uuid is made from a string");
    this.#text = native.uuidText(value);
  }

  /** @param {unknown} other */
  equals(other) {
    return other instanceof Uuid && other.#text === this.#text;
  }

  toString() {
    return this.#text;
  }

  toJSON() {
    return this.#text;
  }

  [INSPECT]() {
    return `Uuid(${this.#text})`;
  }
}

const MICROSECONDS_PER_HOUR = 3_600_000_000n;
const MICROSECONDS_PER_MINUTE = 60_000_000n;
const MICROSECONDS_PER_SECOND = 1_000_000n;

/**
 * @param {unknown} value
 * @param {string} what
 * @returns {number}
 */
function int32(value, what) {
  if (typeof value !== "number" || !Number.isInteger(value) || value < -(2 ** 31) || value >= 2 ** 31) {
    throw new ConstructionError(`an interval's ${what} is a 32-bit integer`);
  }
  return value === 0 ? 0 : value;
}

/**
 * @param {unknown} value
 * @returns {bigint}
 */
function int64(value) {
  if (typeof value === "number" && Number.isSafeInteger(value)) value = BigInt(value);
  if (typeof value !== "bigint" || value < -(2n ** 63n) || value >= 2n ** 63n) {
    throw new ConstructionError("an interval's microseconds are a 64-bit integer");
  }
  return value;
}

/**
 * PostgreSQL's `interval`: months, days and microseconds, each with its own
 * sign, because a month has no fixed number of days nor a day of hours.
 */
export class Interval {
  /** @param {{ months?: number, days?: number, microseconds?: bigint | number }} [fields] */
  constructor({ months = 0, days = 0, microseconds = 0n } = {}) {
    /** @readonly */
    this.months = int32(months, "months");
    /** @readonly */
    this.days = int32(days, "days");
    /** @readonly */
    this.microseconds = int64(microseconds);
    Object.freeze(this);
  }

  /**
   * An interval from a `Temporal.Duration` or an ISO 8601 duration string:
   * years and months become months, weeks and days days, and the time
   * microseconds. A nanosecond digit is refused, not truncated.
   *
   * @param {Interval | Temporal.Duration | string} value
   * @returns {Interval}
   */
  static from(value) {
    if (value instanceof Interval) return value;
    if (typeof value === "string") {
      try {
        value = Temporal.Duration.from(value);
      } catch {
        throw new ConstructionError(`${JSON.stringify(value)} is not an ISO 8601 duration`);
      }
    }
    if (!(value instanceof Temporal.Duration)) {
      throw new ConstructionError("an Interval is made from an Interval, a Temporal.Duration or an ISO 8601 duration");
    }
    if (value.nanoseconds !== 0) {
      throw new ConstructionError(
        "the duration has sub-microsecond digits, which PostgreSQL does not keep: round it to microseconds first",
      );
    }
    const time = ((BigInt(value.hours) * 60n + BigInt(value.minutes)) * 60n + BigInt(value.seconds)) *
        MICROSECONDS_PER_SECOND +
      BigInt(value.milliseconds) * 1000n +
      BigInt(value.microseconds);
    return new Interval({
      months: value.years * 12 + value.months,
      days: value.weeks * 7 + value.days,
      microseconds: time,
    });
  }

  /**
   * The `Temporal.Duration` of the same fields, the time balanced into hours
   * and smaller units. A `Duration`'s fields share one sign, so an interval
   * whose fields do not throws a `RangeError`.
   *
   * @returns {Temporal.Duration}
   */
  toDuration() {
    const signs = [Math.sign(this.months), Math.sign(this.days), Number(this.microseconds > 0n) - Number(this.microseconds < 0n)]
      .filter((sign) => sign !== 0);
    if (signs.some((sign) => sign !== signs[0])) {
      throw new RangeError("an interval whose months, days and time differ in sign has no Temporal.Duration");
    }
    const sign = this.microseconds < 0n ? -1n : 1n;
    let rest = this.microseconds * sign;
    const part = (/** @type {bigint} */ unit) => {
      const whole = rest / unit;
      rest %= unit;
      return Number(whole * sign);
    };
    return Temporal.Duration.from({
      months: this.months,
      days: this.days,
      hours: part(MICROSECONDS_PER_HOUR),
      minutes: part(MICROSECONDS_PER_MINUTE),
      seconds: part(MICROSECONDS_PER_SECOND),
      milliseconds: part(1000n),
      microseconds: part(1n),
    });
  }

  /** The interval in PostgreSQL's `iso_8601` style, each field signed: `P1M-2DT3H`. */
  toString() {
    const years = Math.trunc(this.months / 12);
    const months = this.months % 12;
    const negative = this.microseconds < 0n;
    let rest = negative ? -this.microseconds : this.microseconds;
    const hours = rest / MICROSECONDS_PER_HOUR;
    rest %= MICROSECONDS_PER_HOUR;
    const minutes = rest / MICROSECONDS_PER_MINUTE;
    rest %= MICROSECONDS_PER_MINUTE;
    const seconds = rest / MICROSECONDS_PER_SECOND;
    const fraction = rest % MICROSECONDS_PER_SECOND;
    if (this.months === 0 && this.days === 0 && this.microseconds === 0n) return "PT0S";
    let text = "P";
    if (years !== 0) text += `${years}Y`;
    if (months !== 0) text += `${months}M`;
    if (this.days !== 0) text += `${this.days}D`;
    if (this.microseconds !== 0n) {
      const sign = negative ? "-" : "";
      text += "T";
      if (hours !== 0n) text += `${sign}${hours}H`;
      if (minutes !== 0n) text += `${sign}${minutes}M`;
      if (seconds !== 0n || fraction !== 0n) {
        text += `${sign}${seconds}`;
        if (fraction !== 0n) text += `.${fraction.toString().padStart(6, "0").replace(/0+$/, "")}`;
        text += "S";
      }
    }
    return text;
  }

  toJSON() {
    return this.toString();
  }
}

const EMPTY = Symbol("pgorm-napi empty range");

/**
 * A range: the empty range, or the values between two bounds, `null` on a
 * side being no bound. A side with no bound includes nothing, so its bracket
 * is always `(` or `)`.
 */
export class Range {
  /**
   * @param {unknown} [lower]
   * @param {unknown} [upper]
   * @param {"[)" | "[]" | "(]" | "()"} [bounds]
   * @param {symbol} [empty]
   */
  constructor(lower = null, upper = null, bounds = "[)", empty) {
    if (typeof bounds !== "string" || !/^[[(][\])]$/.test(bounds)) {
      throw new ConstructionError('a range\'s bounds are "[)", "[]", "(]" or "()"');
    }
    const isEmpty = empty === EMPTY;
    /** @readonly @type {unknown} */
    this.lower = isEmpty || lower === undefined ? null : lower;
    /** @readonly @type {unknown} */
    this.upper = isEmpty || upper === undefined ? null : upper;
    /** @readonly */
    this.lowerInclusive = this.lower !== null && bounds[0] === "[";
    /** @readonly */
    this.upperInclusive = this.upper !== null && bounds[1] === "]";
    /** @readonly */
    this.isEmpty = isEmpty;
    Object.freeze(this);
  }

  /** The range that contains no value, which differs from `new Range()`, every value. */
  static empty() {
    return new Range(null, null, "()", EMPTY);
  }

  /** @returns {"[)" | "[]" | "(]" | "()"} */
  get bounds() {
    return /** @type {any} */ ((this.lowerInclusive ? "[" : "(") + (this.upperInclusive ? "]" : ")"));
  }

  toString() {
    if (this.isEmpty) return "empty";
    const bounds = this.bounds;
    return `${bounds[0]}${this.lower ?? ""},${this.upper ?? ""}${bounds[1]}`;
  }
}

/** A multirange: a set of ranges of one subtype. */
export class Multirange {
  /** @param {Iterable<Range>} [ranges] */
  constructor(ranges = []) {
    const list = Array.from(ranges);
    for (const range of list) {
      if (!(range instanceof Range)) throw new ConstructionError("a Multirange holds Range values");
    }
    /** @readonly @type {readonly Range[]} */
    this.ranges = Object.freeze(list);
    Object.freeze(this);
  }

  get length() {
    return this.ranges.length;
  }

  [Symbol.iterator]() {
    return this.ranges[Symbol.iterator]();
  }

  toString() {
    return `{${this.ranges.join(",")}}`;
  }
}

/** @type {(value: Value) => unknown} */
let unwrap;

/**
 * A value with its kind declared: the kinds plain JavaScript cannot tell
 * apart — integer widths, a typed SQL NULL, JSON's null, an empty array, an
 * enum's type, a range's type — and any other kind written out explicitly.
 */
export class Value {
  /** @type {unknown} */
  #native;

  /**
   * @param {unknown} data
   * @param {import("./index.d.ts").Kind | symbol} [kind]
   */
  constructor(data, kind) {
    if (kind === TRUSTED) {
      this.#native = data;
      return;
    }
    this.#native = native.valueNew(data, kind);
  }

  static {
    unwrap = (value) => value.#native;
  }

  /**
   * SQL NULL of a kind.
   *
   * @param {import("./index.d.ts").Kind} kind
   */
  static null(kind) {
    return new Value(native.valueNull(kind), TRUSTED);
  }

  /**
   * A JSON document; `null` here is JSON's null, not SQL NULL.
   *
   * @param {import("./index.d.ts").JsonValue} data
   */
  static json(data) {
    return new Value(native.valueJson(data), TRUSTED);
  }

  /**
   * An array of a declared element kind, empty arrays included; `items`
   * `null` is an SQL NULL array of that element kind.
   *
   * @param {import("./index.d.ts").Kind} kind
   * @param {readonly unknown[] | null} items
   */
  static array(kind, items) {
    return new Value(native.valueArray(kind, items), TRUSTED);
  }

  /** @returns {string} */
  get kind() {
    return native.valueKind(this.#native);
  }

  /** @returns {boolean} */
  get isNull() {
    return native.valueIsNull(this.#native);
  }

  /** @returns {TypeName | null} */
  get typeName() {
    return native.valueTypeName(this.#native);
  }

  /** @returns {CreatedRange | CreatedMultirange | null} */
  get createdType() {
    return native.valueCreatedType(this.#native);
  }

  /** @returns {import("./index.d.ts").Kind | null} */
  get elementType() {
    return native.valueElementType(this.#native);
  }

  /** @returns {import("./index.d.ts").PlainValue} */
  get value() {
    return native.valueGet(this.#native);
  }

  /** @returns {Value[] | null} */
  items() {
    const items = native.valueItems(this.#native);
    return items === null ? null : items.map((/** @type {unknown} */ item) => new Value(item, TRUSTED));
  }

  /** @param {unknown} other */
  equals(other) {
    return other instanceof Value && native.valueEquals(this.#native, other.#native);
  }

  toString() {
    return `Value(${this.kind}${this.isNull ? ", null" : ""})`;
  }

  /**
   * @param {number} _depth
   * @param {object} options
   * @param {((value: unknown, options: object) => string) | undefined} inspect
   */
  [INSPECT](_depth, options, inspect) {
    const value = this.value;
    return `Value(${this.kind}, ${typeof inspect === "function" ? inspect(value, options) : String(value)})`;
  }
}

/**
 * What a JavaScript object is, for the addon: a tag and the fields it reads.
 *
 * @param {object} value
 * @returns {unknown[] | undefined}
 */
function describe(value) {
  if (value instanceof Value) return ["value", unwrap(value)];
  if (value instanceof Decimal) return ["decimal", value.toString()];
  if (value instanceof Uuid) return ["uuid", value.toString()];
  if (value instanceof Interval) return ["interval", value.months, value.days, value.microseconds];
  if (value instanceof Range) {
    return value.isEmpty
      ? ["range", null, null, false, false, true]
      : ["range", value.lower, value.upper, value.lowerInclusive, value.upperInclusive, false];
  }
  if (value instanceof Multirange) return ["multirange", value.ranges];
  if (value instanceof TypeName) return ["typename", value.name, value.schema];
  if (value instanceof CreatedRange) return ["created", value.name, value.schema, value.subtype, false];
  if (value instanceof CreatedMultirange) return ["created", value.name, value.schema, value.subtype, true];
  if (value instanceof Temporal.PlainDate) {
    const iso = value.withCalendar("iso8601");
    return ["date", iso.year, iso.month, iso.day];
  }
  if (value instanceof Temporal.PlainTime) {
    return ["time", value.hour, value.minute, value.second, value.millisecond, value.microsecond, value.nanosecond];
  }
  if (value instanceof Temporal.PlainDateTime) {
    const iso = value.withCalendar("iso8601");
    return [
      "datetime",
      iso.year,
      iso.month,
      iso.day,
      iso.hour,
      iso.minute,
      iso.second,
      iso.millisecond,
      iso.microsecond,
      iso.nanosecond,
    ];
  }
  if (value instanceof Temporal.Instant) return ["instant", value.epochNanoseconds];
  if (value instanceof Temporal.Duration) {
    const interval = Interval.from(value);
    return ["interval", interval.months, interval.days, interval.microseconds];
  }
  if (value instanceof Temporal.ZonedDateTime) {
    return [
      "refused",
      "a Temporal.ZonedDateTime is refused, because PostgreSQL keeps no time zone: pass zoned.toInstant() for a timestamptz, or zoned.toPlainDateTime() for a timestamp",
    ];
  }
  if (value instanceof Temporal.PlainYearMonth || value instanceof Temporal.PlainMonthDay) {
    return ["refused", "a PlainYearMonth or PlainMonthDay has no PostgreSQL type: make it a PlainDate"];
  }
  if (value instanceof Date) {
    return [
      "refused",
      "a Date is refused: date and time values are Temporal's, so pass Temporal.Instant.fromEpochMilliseconds(date.getTime())",
    ];
  }
  const prototype = Object.getPrototypeOf(value);
  if (prototype === Object.prototype || prototype === null) return ["json", Object.entries(value)];
  return undefined;
}

/**
 * A JSON document's integer literal past `Number.MAX_SAFE_INTEGER` becomes a
 * `bigint`, exact, rather than the nearest number.
 *
 * @param {string} _key
 * @param {unknown} value
 * @param {{ source?: string }} [context]
 */
function revive(_key, value, context) {
  if (
    typeof value === "number" && !Number.isSafeInteger(value) && typeof context?.source === "string" &&
    /^-?\d+$/.test(context.source)
  ) {
    return BigInt(context.source);
  }
  return value;
}

/**
 * Build the value the addon describes.
 *
 * @param {string} tag
 * @param {...any} fields
 */
function make(tag, ...fields) {
  const [a, b, c, d, e, f, g, h] = fields;
  switch (tag) {
    case "decimal":
      return new Decimal(a, TRUSTED);
    case "uuid":
      return new Uuid(a, TRUSTED);
    case "interval":
      return new Interval({ months: a, days: b, microseconds: c });
    case "range":
      return new Range(a, b, /** @type {any} */ ((c ? "[" : "(") + (d ? "]" : ")")));
    case "empty":
      return Range.empty();
    case "multirange":
      return new Multirange(a);
    case "date":
      return new Temporal.PlainDate(a, b, c);
    case "time":
      return new Temporal.PlainTime(a, b, c, d, e);
    case "datetime":
      return new Temporal.PlainDateTime(a, b, c, d, e, f, g, h);
    case "instant":
      return new Temporal.Instant(a);
    case "json":
      return JSON.parse(a, /** @type {any} */ (revive));
    case "value":
      return new Value(a, TRUSTED);
    case "typename":
      return new TypeName(a, { schema: b });
    case "created":
      return d ? new CreatedMultirange(a, b, { schema: c }) : new CreatedRange(a, b, { schema: c });
  }
  throw new InternalError(`pgorm-napi has no value tagged ${tag}`);
}
