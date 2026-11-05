// Shared by the test files, which run unchanged under `node --test` and
// `deno test`: both runners accept node:test's `test`, and every runtime
// difference the suite meets is kept here.
// [spec:pgorm:req:napi.runtimes+1/test]

import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import type { Transaction } from "../lib/index.js";

const here = dirname(fileURLToPath(import.meta.url));

/** The repository checkout this package sits in. */
const checkout = join(here, "..", "..");

/** Whether the suite is running under Deno rather than Node. */
export const deno = "Deno" in globalThis;

function fromEnvFile(name: string): string | undefined {
  for (const file of [".env.local", ".env"]) {
    const path = join(checkout, file);
    if (!existsSync(path)) continue;
    for (const line of readFileSync(path, "utf8").split("\n")) {
      const match = /^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*?)\s*$/.exec(line);
      if (match?.[1] === name) return match[2]?.replace(/^(['"])(.*)\1$/, "$2");
    }
  }
  return undefined;
}

/**
 * The connection string the live tests use: `PGORM_TEST_DSN` if it is set,
 * otherwise the server `DATABASE_URL` names — from the environment, or from
 * the checkout's `.env.local` or `.env` as the Rust suite reads it — and its
 * `postgres` maintenance database, which the queries here leave untouched.
 * A pool verifies TLS unless told otherwise, so a string that does not say
 * gains `sslmode=disable`; the TLS tests use {@link tlsCa} instead.
 */
export function dsn(): string {
  const server = process.env.PGORM_TEST_DSN ?? process.env.DATABASE_URL ?? fromEnvFile("DATABASE_URL");
  if (!server) {
    throw new Error(
      "set PGORM_TEST_DSN, or DATABASE_URL to a PostgreSQL server URL, to run the live suite",
    );
  }
  const url = new URL(server);
  if (url.pathname === "" || url.pathname === "/") url.pathname = "/postgres";
  if (!url.searchParams.has("sslmode")) url.searchParams.set("sslmode", "disable");
  return url.toString();
}

/**
 * The PEM certificate authority `PGORM_TEST_CA` names, whose certificate for
 * `localhost` the server presents when it runs with TLS, or `undefined` when
 * the server does not.
 */
export function tlsCa(): string | undefined {
  const path = process.env.PGORM_TEST_CA;
  return path ? readFileSync(path, "utf8") : undefined;
}

/** What a child process did. */
export interface Outcome {
  readonly code: number | null;
  readonly signal: string | null;
  readonly stdout: string;
  readonly stderr: string;
  /** Wall-clock milliseconds from spawn to exit. */
  readonly elapsed: number;
  /** Whether it outlived its deadline and was killed. */
  readonly hung: boolean;
}

/**
 * Run `tests/fixtures/<name>.ts` in a fresh process of the runtime running
 * the suite, with the permissions a native addon needs under Deno — and, with
 * `exposeGc`, a `gc()` global — and kill it if it has not exited by `deadline`
 * milliseconds.
 */
export function runFixture(
  name: string,
  deadline: number,
  env: Record<string, string> = {},
  { exposeGc = false }: { exposeGc?: boolean } = {},
): Promise<Outcome> {
  const script = join(here, "fixtures", `${name}.ts`);
  const args = deno
    ? ["run", "--allow-ffi", "--allow-read", "--allow-env", ...(exposeGc ? ["--v8-flags=--expose-gc"] : []), script]
    : [...(exposeGc ? ["--expose-gc"] : []), script];
  const started = performance.now();
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, args, {
      env: { ...process.env, PGORM_TEST_DSN: dsn(), NO_COLOR: "1", ...env },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    let hung = false;
    child.stdout.setEncoding("utf8").on("data", (chunk: string) => (stdout += chunk));
    child.stderr.setEncoding("utf8").on("data", (chunk: string) => (stderr += chunk));
    const timer = setTimeout(() => {
      hung = true;
      child.kill("SIGKILL");
    }, deadline);
    child.on("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.on("close", (code, signal) => {
      clearTimeout(timer);
      resolve({ code, signal, stdout, stderr, elapsed: performance.now() - started, hung });
    });
  });
}

/**
 * The longest identifier PostgreSQL stores without truncating it.
 */
const MAX_IDENTIFIER_LEN = 63;

/**
 * The tail every minted name ends with, as a PostgreSQL regular expression.
 * It only narrows what the catalogue returns; {@link creatorOf} reads each
 * candidate exactly before anything is decided about it.
 */
const TAG_PATTERN = "__[0-9a-f]{8}_[0-9]+_[0-9a-f]{8}$";

/**
 * How far after its stamp a creator's start can appear: `ps` reports elapsed
 * time in whole seconds, and the stamp is truncated to one.
 */
const START_SLACK_SECS = 2;

/** The process that minted a name. */
interface Creator {
  readonly owner: number;
  readonly pid: number;
  readonly stamp: number;
}

function fnv1a(text: string): number {
  let hash = 0x811c9dc5;
  for (const byte of new TextEncoder().encode(text)) {
    hash = Math.imul((hash ^ byte) >>> 0, 0x01000193) >>> 0;
  }
  return hash;
}

function hostName(): string {
  try {
    return readFileSync("/proc/sys/kernel/hostname", "utf8").trim();
  } catch {
    const out = spawnSync("hostname", [], { encoding: "utf8" });
    return out.status === 0 ? out.stdout.trim() : "";
  }
}

function unixNow(): number {
  return Math.floor(Date.now() / 1000);
}

let run: Creator | undefined;

/**
 * This process, as the names it mints record it: the owner hashes the host
 * name and the user as the Rust suites' `pgorm-test-support` crate does, and
 * is 0, which no sweep acts on, when the host name cannot be learned.
 */
function thisRun(): Creator {
  if (run === undefined) {
    const host = hostName();
    const user = process.env.USER ?? process.env.LOGNAME ?? process.env.USERNAME ?? "";
    run = { owner: host === "" ? 0 : fnv1a(`${host}\n${user}`), pid: process.pid, stamp: unixNow() };
  }
  return run;
}

let minted = 0;

/**
 * A database name for `label`, unique to this process and to this call within
 * it, in the form every live suite on a shared server mints:
 * `{label}_{seq}__{owner}_{pid}_{stamp}`, the label cut to fit 63 bytes. The
 * Rust suites' `pgorm-test-support` crate says what each part is for.
 */
// [spec:pgorm:req:testing.isolation]    the per-run name a scratch database takes
export function scopedName(label: string): string {
  const { owner, pid, stamp } = thisRun();
  const hex = (value: number) => value.toString(16).padStart(8, "0");
  const tail = `_${minted++}__${hex(owner)}_${pid}_${hex(stamp)}`;
  let cut = label;
  while (new TextEncoder().encode(cut + tail).length > MAX_IDENTIFIER_LEN) cut = cut.slice(0, -1);
  return cut + tail;
}

function creatorOf(name: string): Creator | undefined {
  const match = /_(\d+)__([0-9a-f]{8})_(\d+)_([0-9a-f]{8})$/.exec(name);
  if (!match) return undefined;
  return { owner: parseInt(match[2]!, 16), pid: Number(match[3]), stamp: parseInt(match[4]!, 16) };
}

/**
 * When the process holding `pid` started, in Unix seconds; `"gone"` when no
 * process holds it; `undefined` when `ps` cannot say.
 */
function holder(pid: number): number | "gone" | undefined {
  const out = spawnSync("ps", ["-o", "etime=", "-p", String(pid)], { encoding: "utf8" });
  if (out.error || out.status === null) return undefined;
  const elapsed = out.stdout.trim();
  if (out.status !== 0) return elapsed === "" && out.stderr === "" ? "gone" : undefined;
  const match = /^(?:(\d+)-)?(?:(\d+):)?(\d+):(\d+)$/.exec(elapsed);
  if (!match) return undefined;
  const [, days = "0", hours = "0", minutes, seconds] = match;
  return unixNow() - (((Number(days) * 24 + Number(hours)) * 60 + Number(minutes)) * 60 + Number(seconds));
}

let psAnswers: boolean | undefined;

/**
 * Whether the process that minted `creator` may still be running. Every
 * question this host cannot answer counts as yes: a creator recorded by
 * another host or user, or `ps` not answering for this very process, which
 * would make its silence about another pid meaningless.
 */
function mayBeRunning(creator: Creator): boolean {
  const self = thisRun();
  if (creator.owner === 0 || creator.owner !== self.owner) return true;
  if (creator.pid === self.pid && creator.stamp === self.stamp) return true;
  psAnswers ??= typeof holder(process.pid) === "number";
  if (!psAnswers) return true;
  const started = holder(creator.pid);
  if (started === "gone") return false;
  return started === undefined || started <= creator.stamp + START_SLACK_SECS;
}

let swept = false;

function quoted(name: string): string {
  return `"${name.replaceAll('"', '""')}"`;
}

/**
 * Drop every database and role a gone creator left on the server, once per
 * process, databases first so a role's privileges on a leftover database's
 * objects go with it. A database is dropped without `FORCE`, so a session
 * that connected after the catalogue was read makes the drop fail rather than
 * be evicted; each failure is left for a later sweep, and a sweep that fails
 * outright fails no test.
 */
// [spec:pgorm:req:testing.isolation]    only a provably gone creator's leftovers are dropped
async function sweep(server: string): Promise<void> {
  if (swept) return;
  swept = true;
  const { Pool } = await import("../lib/index.js");
  await using pool = new Pool(server, { maxSize: 1 });
  const leftovers = async (sql: string) =>
    (await pool.query(sql, [TAG_PATTERN]))
      .map((row) => String(row.name))
      .filter((name) => {
        const creator = creatorOf(name);
        return creator !== undefined && !mayBeRunning(creator);
      });
  try {
    const databases = await leftovers(
      "SELECT d.datname::text AS name FROM pg_database d WHERE d.datname ~ $1 AND NOT d.datistemplate " +
        "AND NOT EXISTS (SELECT 1 FROM pg_stat_activity a WHERE a.datname = d.datname)",
    );
    for (const name of databases) await pool.execute(`DROP DATABASE IF EXISTS ${quoted(name)}`).catch(() => {});
    const roles = await leftovers("SELECT rolname::text AS name FROM pg_roles WHERE rolname ~ $1");
    for (const name of roles) await pool.execute(`DROP ROLE IF EXISTS ${quoted(name)}`).catch(() => {});
  } catch (error) {
    console.warn(`leftovers of crashed test runs were not swept: ${error}`);
  }
}

/**
 * A database of its own for a test file, named by {@link scopedName} so no
 * other run on the server, of either runtime or of the Rust suites, can hold
 * it, made fresh on the server {@link dsn} names and dropped by `drop`. The
 * first one a process makes sweeps away what crashed runs left. With
 * `template`, the database is a copy of that one, on which no session may be
 * open while it is copied.
 */
// [spec:pgorm:req:testing.isolation]    a run creates and drops only its own database
export async function scratchDatabase(
  prefix: string,
  { template }: { template?: string } = {},
): Promise<{ dsn: string; drop(): Promise<void> }> {
  const { Pool } = await import("../lib/index.js");
  const name = scopedName(prefix);
  const server = dsn();
  const maintenance = async (sql: string) => {
    await using pool = new Pool(server, { maxSize: 1 });
    await pool.execute(sql);
  };
  await sweep(server);
  const copied = template === undefined ? "" : ` TEMPLATE ${quoted(template)}`;
  await maintenance(`CREATE DATABASE ${quoted(name)}${copied}`);
  const url = new URL(server);
  url.pathname = `/${name}`;
  return {
    dsn: url.toString(),
    async drop() {
      // FORCE evicts only this process's own sessions: no other run knows the name.
      await maintenance(`DROP DATABASE IF EXISTS ${quoted(name)} WITH (FORCE)`);
    },
  };
}

/**
 * Assert two plain values equal, comparing what `assert.deepStrictEqual`
 * cannot see: a Temporal value has no own properties, so any two of a class
 * would pass it, and the module's `Decimal` and `Uuid` keep their text in a
 * private field. Each is compared by class and text, recursively through
 * arrays, ranges and plain objects.
 */
export function same(actual: unknown, expected: unknown, path = "value"): void {
  if (typeof expected !== "object" || expected === null) {
    assert.deepStrictEqual(actual, expected, path);
    return;
  }
  assert.equal(
    Object.getPrototypeOf(actual),
    Object.getPrototypeOf(expected),
    `${path}: ${actual?.constructor?.name} is not ${expected.constructor?.name}`,
  );
  if (expected instanceof Uint8Array || expected instanceof Float32Array) {
    assert.deepStrictEqual(actual, expected, path);
  } else if (Array.isArray(expected)) {
    assert.ok(Array.isArray(actual), path);
    assert.equal(actual.length, expected.length, `${path}.length`);
    expected.forEach((item, index) => same(actual[index], item, `${path}[${index}]`));
  } else if (Object.getOwnPropertyNames(expected).length === 0) {
    assert.equal(String(actual), String(expected), path);
  } else {
    assert.deepStrictEqual(Object.keys(actual as object), Object.keys(expected), `${path} keys`);
    for (const [key, item] of Object.entries(expected)) {
      same((actual as Record<string, unknown>)[key], item, `${path}.${key}`);
    }
  }
}

/** A transaction held open until the test ends its callback. */
export interface Held {
  readonly transaction: Transaction;
  /**
   * Let the callback resolve, or throw `error` if one is given, and give back
   * what `transaction(fn)` then settles with.
   */
  end(error?: unknown): Promise<unknown>;
}

/**
 * Open a transaction through `open`, which passes its callback to some
 * `transaction(fn)`, and hold that callback pending until `end`, so that the
 * test can act on a transaction whose callback has not settled. A transaction
 * that never opens rejects with why.
 */
export async function holdTransaction(
  open: (use: (transaction: Transaction) => Promise<void>) => Promise<unknown>,
): Promise<Held> {
  const opened = Promise.withResolvers<Transaction>();
  const release = Promise.withResolvers<void>();
  const settled = open(async (transaction) => {
    opened.resolve(transaction);
    await release.promise;
  });
  settled.catch((error: unknown) => opened.reject(error));
  const transaction = await opened.promise;
  return {
    transaction,
    end(error?: unknown) {
      if (error === undefined) release.resolve();
      else release.reject(error);
      return settled;
    },
  };
}

/**
 * A check for `assert.throws` and `assert.rejects` that the call was refused
 * with a `TypeError`, an argument of a type it does not take, whose message
 * matches `pattern`.
 */
export function mistyped(pattern: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof TypeError, `expected a TypeError, got ${error}`);
    assert.match(error.message, pattern);
    return true;
  };
}
