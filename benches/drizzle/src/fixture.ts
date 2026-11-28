// The tables every side reads, the SQL the raw sides send, and the checks
// every result passes. Each side returns its own driver's value types; the
// checks normalise them, so what a side converts is part of what it costs
// and never part of what it is judged by.

import { createHash } from "node:crypto";

export const SCALAR = "SELECT $1::BIGINT";
export const PARAMS16 = `SELECT ${Array.from({ length: 16 }, (_, at) => `$${at + 1}::BIGINT`).join(" + ")}`;
export const LOOKUP = "SELECT id, payload, value FROM bench_row WHERE id = $1";
export const UPDATE16 = `UPDATE bench_row SET payload = $2, value = ${Array.from({ length: 14 }, (_, at) => `$${at + 3}::BIGINT`).join(" + ")} WHERE id = $1`;
export const ROWS = "SELECT id, payload, value FROM bench_row ORDER BY id LIMIT $1";
export const WIDE = "SELECT id, amount, at, uid, label, flag, doc FROM bench_wide ORDER BY id LIMIT $1";
export const TEXTS = "SELECT id, body FROM bench_text WHERE width = $1 ORDER BY id";

/** The columns of `bench_wide` the per-type attribution reads one at a time. */
export const WIDE_COLUMNS = ["id", "amount", "at", "uid", "label", "flag", "doc"] as const;
export type WideColumn = (typeof WIDE_COLUMNS)[number];

/** The text widths, in bytes, the per-byte attribution reads 256 rows of. */
export const TEXT_WIDTHS = [32, 1024, 16384] as const;
export const TEXT_ROWS = 256;

export const LOOKUP_ID = 17;
export const SCALAR_VALUE = 17;
export const PARAMS16_SUM = 17 * 16;
/** Row 17's payload. The update writes it back unchanged, so every round reads the fixture it started with. */
export const PAYLOAD = "x".repeat(32);
/** The update's first summand; the other thirteen are 1, so the sum is row 17's own value. */
export const UPDATE_FIRST = 4;
export const UPDATE_VALUES: readonly number[] = [UPDATE_FIRST, ...Array.from({ length: 13 }, () => 1)];

/** Hex text of `bytes` length for row `id`, built from MD5 digests as the server builds it, which nothing compresses. */
function hexText(id: number, bytes: number): string {
  let text = "";
  for (let part = 1; text.length < bytes; part++) text += createHash("md5").update(`${id}-${part}`).digest("hex");
  return text.slice(0, bytes);
}

export function fixtureScript(): string {
  const widths = TEXT_WIDTHS.map((width) => `(${width})`).join(", ");
  return `
CREATE TABLE bench_row (id bigint PRIMARY KEY, payload text NOT NULL, value bigint NOT NULL);
INSERT INTO bench_row SELECT i, repeat('x', 32), i FROM generate_series(1, 1024) i;
CREATE TABLE bench_wide (
  id bigint PRIMARY KEY,
  amount numeric(20, 6) NOT NULL,
  at timestamptz NOT NULL,
  uid uuid NOT NULL,
  label text NOT NULL,
  flag boolean NOT NULL,
  doc jsonb NOT NULL
);
INSERT INTO bench_wide
SELECT i,
       i * 1.5 + 0.123456,
       timestamptz '2026-01-01 00:00:00+00' + i * interval '1 second',
       md5(i::text)::uuid,
       'label-' || lpad(i::text, 8, '0'),
       i % 2 = 0,
       jsonb_build_object('i', i, 'tags', jsonb_build_array('a', 'b'), 'nested', jsonb_build_object('x', i * 2))
FROM generate_series(1, 1024) i;
CREATE TABLE bench_text (width int NOT NULL, id int NOT NULL, body text NOT NULL, PRIMARY KEY (width, id));
ALTER TABLE bench_text ALTER COLUMN body SET STORAGE EXTERNAL;
INSERT INTO bench_text
SELECT w.width, i, left((SELECT string_agg(md5(i::text || '-' || part::text), '' ORDER BY part) FROM generate_series(1, (w.width + 31) / 32) part), w.width)
FROM (VALUES ${widths}) AS w(width), generate_series(1, ${TEXT_ROWS}) i;
ANALYZE bench_row;
ANALYZE bench_wide;
ANALYZE bench_text;
`;
}

/** A value every side agrees on once normalised: text for the integers and numerics, milliseconds for instants. */
type Plain = string | number | boolean | null | Plain[] | { [key: string]: Plain };

function fail(what: string, actual: unknown, expected: unknown): never {
  throw new Error(`${what}: got ${describe(actual)}, expected ${describe(expected)}`);
}

function describe(value: unknown): string {
  if (typeof value === "bigint") return `${value}n`;
  try {
    return `${JSON.stringify(value, (_, item) => (typeof item === "bigint" ? `${item}n` : item))} (${typeof value})`;
  } catch {
    return String(value);
  }
}

/** An int8 as each side reads it: pg and postgres.js a string, pgorm and Drizzle's bigint mode a bigint. */
export function int8(value: unknown, what: string): string {
  if (typeof value === "bigint" || typeof value === "string" || typeof value === "number") return String(value);
  return fail(what, value, "an int8");
}

function numeric(value: unknown, what: string): string {
  if (typeof value !== "string" && !(typeof value === "object" && value !== null)) fail(what, value, "a numeric");
  const text = String(value);
  return text.includes(".") ? text.replace(/0+$/, "").replace(/\.$/, "") : text;
}

function instant(value: unknown, what: string): number {
  if (value instanceof Date) return value.getTime();
  if (value instanceof Temporal.Instant) return value.epochMilliseconds;
  return fail(what, value, "a Date or Temporal.Instant");
}

function uuid(value: unknown, what: string): string {
  if (typeof value !== "string" && !(typeof value === "object" && value !== null)) fail(what, value, "a uuid");
  return String(value).toLowerCase();
}

function same(actual: Plain, expected: Plain, what: string): void {
  if (Array.isArray(expected)) {
    if (!Array.isArray(actual) || actual.length !== expected.length) fail(what, actual, expected);
    expected.forEach((item, at) => same((actual as Plain[])[at] as Plain, item, `${what}[${at}]`));
  } else if (typeof expected === "object" && expected !== null) {
    if (typeof actual !== "object" || actual === null || Array.isArray(actual)) fail(what, actual, expected);
    const keys = Object.keys(expected);
    if (Object.keys(actual).length !== keys.length) fail(what, actual, expected);
    for (const key of keys) same((actual as Record<string, Plain>)[key] as Plain, expected[key] as Plain, `${what}.${key}`);
  } else if (actual !== expected) {
    fail(what, actual, expected);
  }
}

function field(row: unknown, name: string, what: string): unknown {
  if (typeof row !== "object" || row === null || !(name in row)) fail(`${what}.${name}`, row, `a row with ${name}`);
  return (row as Record<string, unknown>)[name];
}

export function checkRow(row: unknown, id: number, what: string): void {
  const actualId = int8(field(row, "id", what), `${what}.id`);
  if (actualId !== String(id)) fail(`${what}.id`, actualId, String(id));
  const payload = field(row, "payload", what);
  if (payload !== PAYLOAD) fail(`${what}.payload`, payload, PAYLOAD);
  const value = int8(field(row, "value", what), `${what}.value`);
  if (value !== String(id)) fail(`${what}.value`, value, String(id));
}

export function checkRows(rows: unknown, count: number, what: string): void {
  if (!Array.isArray(rows) || rows.length !== count) fail(`${what} length`, Array.isArray(rows) ? rows.length : rows, count);
  for (let at = 0; at < count; at++) checkRow(rows[at], at + 1, `${what}[${at}]`);
}

interface WideRow {
  readonly id: string;
  readonly amount: string;
  readonly at: number;
  readonly uid: string;
  readonly label: string;
  readonly flag: boolean;
  readonly doc: Plain;
}

function md5Uuid(text: string): string {
  const hex = createHash("md5").update(text).digest("hex");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function wideRow(id: number): WideRow {
  const micros = BigInt(id) * 1_500_000n + 123_456n;
  const whole = micros / 1_000_000n;
  const fraction = (micros % 1_000_000n).toString().padStart(6, "0").replace(/0+$/, "");
  return {
    id: String(id),
    amount: fraction === "" ? String(whole) : `${whole}.${fraction}`,
    at: Date.UTC(2026, 0, 1) + id * 1000,
    uid: md5Uuid(String(id)),
    label: `label-${String(id).padStart(8, "0")}`,
    flag: id % 2 === 0,
    doc: { i: id, tags: ["a", "b"], nested: { x: id * 2 } },
  };
}

const WIDE_ROWS: readonly WideRow[] = Array.from({ length: 1024 }, (_, at) => wideRow(at + 1));

function normalised(column: WideColumn, value: unknown, what: string): Plain {
  switch (column) {
    case "id":
      return int8(value, what);
    case "amount":
      return numeric(value, what);
    case "at":
      return instant(value, what);
    case "uid":
      return uuid(value, what);
    case "label":
    case "flag":
      return value as Plain;
    case "doc":
      return value as Plain;
  }
}

export function checkWideColumn(rows: unknown, column: WideColumn, count: number, what: string): void {
  if (!Array.isArray(rows) || rows.length !== count) fail(`${what} length`, Array.isArray(rows) ? rows.length : rows, count);
  for (let at = 0; at < count; at++) {
    const expected = WIDE_ROWS[at] as WideRow;
    const place = `${what}[${at}].${column}`;
    same(normalised(column, field(rows[at], column, place), place), expected[column], place);
  }
}

export function checkWide(rows: unknown, count: number, what: string): void {
  if (!Array.isArray(rows) || rows.length !== count) fail(`${what} length`, Array.isArray(rows) ? rows.length : rows, count);
  for (let at = 0; at < count; at++) {
    const expected = WIDE_ROWS[at] as WideRow;
    for (const column of WIDE_COLUMNS) {
      const place = `${what}[${at}].${column}`;
      same(normalised(column, field(rows[at], column, place), place), expected[column], place);
    }
  }
}

const TEXT_BODIES: ReadonlyMap<number, readonly string[]> = new Map(
  TEXT_WIDTHS.map((width) => [width, Array.from({ length: TEXT_ROWS }, (_, at) => hexText(at + 1, width))]),
);

export function checkTexts(rows: unknown, width: number, what: string): void {
  const bodies = TEXT_BODIES.get(width);
  if (bodies === undefined) throw new Error(`no fixture text is ${width} bytes wide`);
  if (!Array.isArray(rows) || rows.length !== TEXT_ROWS) fail(`${what} length`, Array.isArray(rows) ? rows.length : rows, TEXT_ROWS);
  for (let at = 0; at < TEXT_ROWS; at++) {
    const body = field(rows[at], "body", `${what}[${at}]`);
    if (body !== bodies[at]) fail(`${what}[${at}].body`, typeof body === "string" ? `${body.length} bytes` : body, `${width} bytes`);
  }
}

export function checkInt8(value: unknown, expected: number, what: string): void {
  const text = int8(value, what);
  if (text !== String(expected)) fail(what, text, String(expected));
}

export function checkOneRow(count: unknown, what: string): void {
  if (Number(count) !== 1) fail(what, count, 1);
}
