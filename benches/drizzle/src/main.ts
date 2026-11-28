// pgorm's JavaScript binding, Drizzle ORM and the raw node-postgres and
// postgres.js drivers, measured on driver_baseline's workloads against one
// scratch database: `node --expose-gc src/main.ts [options]`, or
// `node src/main.ts summarize <report.json>` for a report's tables.

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

import { features, version } from "@necessary/pgorm";

import { scratchDatabase } from "../../../pgorm-napi/tests/support.ts";
import { addonBuild, contenders, idleProblems, machine, moment, packageVersions, server, sources } from "./conditions.ts";
import { fixtureScript, TEXT_WIDTHS, WIDE_COLUMNS } from "./fixture.ts";
import { gcExposed, measure, measureConcurrent, type Operation, settle, type Summary } from "./measure.ts";
import { type Options, readOptions } from "./options.ts";
import { markdown } from "./report.ts";
import { type Prepared, SEQUENTIAL, type Session, type Side, WORKLOADS } from "./sides.ts";
import { drizzleSides } from "./sides/drizzle.ts";
import { pgClient, pgSides } from "./sides/pg.ts";
import { pgormSides } from "./sides/pgorm.ts";
import { postgresSides } from "./sides/postgresjs.ts";

/** The sides the attribution reads at every row count: pgorm's two paths and each raw driver's prepared path, the floor. */
const ATTRIBUTION_SIDES = ["pgorm/raw", "pgorm/orm", "pg/prepared", "postgres/prepared"];

/** `items` rotated so that each round starts a different part of the way through. */
function rotated<T>(items: readonly T[], round: number, rounds: number): T[] {
  if (items.length === 0) return [];
  const shift = (round * Math.ceil(items.length / rounds)) % items.length;
  return [...items.slice(shift), ...items.slice(0, shift)];
}

interface Absence {
  readonly side: string;
  readonly workload: string;
  readonly reason: string;
}

async function openAll(sides: readonly Side[], dsn: string, options: Options): Promise<Map<Side, Session>> {
  const sessions = new Map<Side, Session>();
  try {
    for (const side of sides) sessions.set(side, await side.open(dsn, options));
  } catch (error) {
    await closeAll(sessions);
    throw error;
  }
  return sessions;
}

async function closeAll(sessions: Map<Side, Session>): Promise<void> {
  const failures: unknown[] = [];
  for (const session of sessions.values()) await session.close().catch((error: unknown) => failures.push(error));
  if (failures.length > 0) throw new AggregateError(failures, "closing the sides' sessions failed");
}

async function timed(side: Side, workload: string, round: number, options: Options, operation: Operation, results: Summary[]) {
  settle();
  results.push(await measure(side.name, workload, round, options, operation));
}

/**
 * One round: every side's sequential workloads, grouped by workload so each
 * workload's sides run close together in time, the sides rotated; then what
 * each side's session holds prepared; then each side's pool.
 */
async function round(round: number, sides: readonly Side[], dsn: string, options: Options, report: Report): Promise<void> {
  const order = rotated(sides, round, options.rounds);
  report.order.push(order.map((side) => side.name));
  const sessions = await openAll(order, dsn, options);
  try {
    for (const workload of SEQUENTIAL.filter(wanted(options))) {
      for (const side of order) {
        const session = sessions.get(side) as Session;
        const operation =
          workload === "rows_all" || workload === "wide_all"
            ? session.counted[workload](options.rows)
            : workload === "rows_stream"
              ? session.counted.rows_stream?.(options.rows)
              : session.fixed[workload];
        if (operation === undefined) continue;
        await timed(side, workload, round, options, operation, report.measurements);
      }
    }
    for (const side of order) {
      report.prepared.push({ side: side.name, round, statements: await (sessions.get(side) as Session).prepared() });
    }
  } finally {
    await closeAll(sessions);
  }
  if (!wanted(options)("concurrent_pool")) return;
  for (const side of order) {
    if (side.openPool === undefined) continue;
    const pool = await side.openPool(dsn, options);
    try {
      settle();
      report.measurements.push(await measureConcurrent(side.name, "concurrent_pool", round, options, pool.operation));
    } finally {
      await pool.close();
    }
  }
}

/**
 * The attribution: row counts that separate what a call costs from what a
 * row and a value cost, single columns of each type, and text of growing
 * width, on pgorm and the raw drivers' prepared paths; and, for pgorm, the
 * native call alone and the row objects alone.
 */
async function attribution(round: number, sides: readonly Side[], dsn: string, options: Options, report: Report): Promise<void> {
  const order = rotated(sides.filter((side) => ATTRIBUTION_SIDES.includes(side.name)), round, options.rounds);
  if (order.length === 0) return;
  const sessions = await openAll(order, dsn, options);
  const each = async (workload: string, pick: (session: Session) => Operation | undefined) => {
    for (const side of order) {
      const operation = pick(sessions.get(side) as Session);
      if (operation !== undefined) await timed(side, workload, round, options, operation, report.attribution);
    }
  };
  try {
    for (const count of options.attributionRows) {
      await each(`rows_all@${count}`, (session) => session.counted.rows_all(count));
      await each(`wide_all@${count}`, (session) => session.counted.wide_all(count));
      await each(`rows_stream@${count}`, (session) => session.counted.rows_stream?.(count));
      const extras = order.flatMap((side) => {
        const made = (sessions.get(side) as Session).attribution?.(count);
        return made === undefined ? [] : Object.entries(made).map(([name, operation]) => ({ side, name, operation }));
      });
      for (const { side, name, operation } of extras) await timed(side, `${name}@${count}`, round, options, operation, report.attribution);
    }
    await pools(round, order, dsn, options, report);
    const widest = Math.max(...options.attributionRows);
    for (const column of WIDE_COLUMNS) await each(`column_${column}@${widest}`, (session) => session.counted.column?.(column, widest));
    for (const width of TEXT_WIDTHS) await each(`text_${width}B@256`, (session) => session.counted.texts?.(width));
  } finally {
    await closeAll(sessions);
  }
}

/**
 * Each side's pool-level query with one operation in flight, which separates
 * what acquiring and releasing cost from what concurrency costs, and the
 * pool configurations a side offers beside its default.
 */
async function pools(round: number, order: readonly Side[], dsn: string, options: Options, report: Report): Promise<void> {
  const one = { ...options, workers: 1 };
  for (const side of order) {
    if (side.openPool === undefined) continue;
    for (const variant of [undefined, ...(side.poolVariants ?? [])]) {
      const suffix = variant === undefined ? "" : `[${variant}]`;
      const runs: [string, Options][] = [[`pool_query${suffix}@1worker`, one]];
      if (variant !== undefined) runs.push([`concurrent_pool${suffix}`, options]);
      for (const [workload, shape] of runs) {
        const pool = await side.openPool(dsn, shape, variant);
        try {
          settle();
          report.attribution.push(await measureConcurrent(side.name, workload, round, shape, pool.operation));
        } finally {
          await pool.close();
        }
      }
    }
  }
}

function wanted(options: Options) {
  return (workload: string) => options.workloads.length === 0 || options.workloads.includes(workload);
}

interface Report {
  schema_version: 1;
  label: string;
  started: string;
  finished?: string;
  options: Options;
  gc_between_workloads: boolean;
  idle_check: { required: boolean; bound: number; problems_at_start: string[] };
  machine: ReturnType<typeof machine>;
  sources_at_start: ReturnType<typeof sources>;
  sources_at_end?: ReturnType<typeof sources>;
  contenders_at_start: ReturnType<typeof contenders>;
  packages: Record<string, string | null>;
  pgorm: { version: string; features: readonly string[]; addon: ReturnType<typeof addonBuild> };
  server?: Awaited<ReturnType<typeof server>>;
  sides: { name: string; library: string; absent: Record<string, string>; sql: Record<string, string> }[];
  absent: Absence[];
  rounds: { round: number; start: ReturnType<typeof moment>; end?: ReturnType<typeof moment> }[];
  order: string[][];
  prepared: { side: string; round: number; statements: Prepared[] }[];
  measurements: Summary[];
  attribution: Summary[];
  notes: string[];
}

const NOTES = [
  "Every side's code is TypeScript run by the same Node.js process by type stripping; pgorm's native addon is part of what pgorm/* measures, as each driver is part of what its sides measure.",
  "Sequential workloads hold one connection per side; concurrent_pool runs options.workers async loops over a pool of options.pool_size, each operation the side's own pool-level query, which acquires, queries and releases.",
  "Values are each side's defaults: pg and postgres.js read int8 and numeric as strings and timestamptz as a Date; pgorm reads bigint, Decimal and Temporal.Instant; Drizzle reads bigint (its bigint columns are declared mode: 'bigint'), numeric as a string and timestamptz as a Date. The checks normalise; the conversions are part of what each side costs.",
  "Sequential operations_per_second is samples over the sum of their latencies; each result's check runs after the clock stops and is not counted. Concurrent operations_per_second is wall-clock, as driver_baseline reports it.",
  "pgorm prepares and caches every statement on its connection itself; it has no unprepared mode. Whether each other side prepares is recorded from pg_prepared_statements on its own session (`prepared`).",
  "This run instruments no allocations or copies; latency differences are not evidence about either.",
];

async function main(argv: readonly string[]): Promise<number> {
  if (argv[0] === "summarize") {
    const path = argv[1];
    if (path === undefined) throw new Error("summarize needs a report's path");
    process.stdout.write(markdown(JSON.parse(readFileSync(path, "utf8")) as Report));
    return 0;
  }
  const options = readOptions(argv);
  const problems = (() => {
    try {
      return idleProblems(options);
    } catch (error) {
      return [`the idle check failed: ${error}`];
    }
  })();
  if (options.requireIdle && problems.length > 0) {
    console.error(`refusing to start: the machine is not idle:\n- ${problems.join("\n- ")}`);
    return 2;
  }
  const addon = addonBuild();
  if (addon.profile !== "addon" && !options.allowUnverifiedAddon) {
    console.error(
      `refusing to start: pgorm-napi/lib/pgorm_napi.node is not the addon profile's build (it matches ${addon.profile ?? "no profile"} under target/); build it with node pgorm-napi/scripts/build.mjs --release, or pass --allow-unverified-addon true`,
    );
    return 2;
  }
  const all = [...pgormSides(), ...drizzleSides(), ...pgSides(), ...postgresSides()];
  for (const name of options.sides) {
    if (!all.some((side) => side.name === name)) throw new Error(`no side is named ${name}: the sides are ${all.map((side) => side.name).join(", ")}`);
  }
  for (const name of options.workloads) {
    if (!(WORKLOADS as readonly string[]).includes(name)) throw new Error(`no workload is named ${name}: the workloads are ${WORKLOADS.join(", ")}`);
  }
  const sides = options.sides.length === 0 ? all : all.filter((side) => options.sides.includes(side.name));
  const report: Report = {
    schema_version: 1,
    label: options.label,
    started: new Date().toISOString(),
    options,
    gc_between_workloads: gcExposed(),
    idle_check: { required: options.requireIdle, bound: options.idleLoad, problems_at_start: problems },
    machine: machine(),
    sources_at_start: sources(),
    contenders_at_start: contenders(),
    packages: packageVersions(),
    pgorm: { version, features, addon },
    sides: sides.map((side) => ({ name: side.name, library: side.library, absent: side.absent, sql: side.sql })),
    absent: sides.flatMap((side) => Object.entries(side.absent).map(([workload, reason]) => ({ side: side.name, workload, reason }))),
    rounds: [],
    order: [],
    prepared: [],
    measurements: [],
    attribution: [],
    notes: NOTES,
  };
  const database = await scratchDatabase("drizzle_bench");
  let failure: unknown;
  try {
    const setup = await pgClient(database.dsn, "pgorm-drizzle-bench");
    try {
      await setup.query(fixtureScript());
      report.server = await server(setup, new URL(database.dsn).hostname.startsWith("/") ? "unix-socket" : "tcp");
    } finally {
      await setup.end();
    }
    for (let at = 0; at < options.rounds; at++) {
      const entry: Report["rounds"][number] = { round: at, start: moment() };
      report.rounds.push(entry);
      await round(at, sides, database.dsn, options, report);
      if (options.attribution) await attribution(at, sides, database.dsn, options, report);
      entry.end = moment();
    }
  } catch (error) {
    failure = error;
  } finally {
    try {
      await database.drop();
    } catch (error) {
      failure = failure === undefined ? error : new SuppressedError(error, failure, "the benchmark failed, and dropping its database failed too");
    }
  }
  if (failure !== undefined) throw failure;
  report.finished = new Date().toISOString();
  report.sources_at_end = sources();
  mkdirSync(dirname(options.output), { recursive: true });
  writeFileSync(options.output, `${JSON.stringify(report, null, 2)}\n`);
  writeFileSync(options.output.replace(/\.json$/, "") + ".md", markdown(report));
  console.error(`wrote ${options.output}`);
  return 0;
}

process.exitCode = await main(process.argv.slice(2));

export type { Report };
