import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const checkout = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");

export interface Options {
  readonly warmup: number;
  readonly samples: number;
  readonly rounds: number;
  readonly workers: number;
  readonly poolSize: number;
  readonly rows: number;
  readonly attributionRows: readonly number[];
  /** Rows per fetch for the drivers whose streams fetch in batches: pg-query-stream and postgres.js's cursor. */
  readonly cursorBatch: number;
  /** Side names to run, or every side when empty. */
  readonly sides: readonly string[];
  /** Workload names to run, or every workload when empty. */
  readonly workloads: readonly string[];
  readonly attribution: boolean;
  readonly output: string;
  readonly rawSamples: boolean;
  /** Refuse to start unless the machine is idle by {@link idleLoad}, Time Machine and cargo. */
  readonly requireIdle: boolean;
  readonly idleLoad: number;
  readonly allowUnverifiedAddon: boolean;
  readonly label: string;
}

const NUMBERS = ["--warmup", "--samples", "--rounds", "--workers", "--pool-size", "--rows", "--cursor-batch", "--idle-load"];
const BOOLEANS = ["--attribution", "--raw-samples", "--require-idle", "--allow-unverified-addon"];
const LISTS = ["--sides", "--workloads", "--attribution-rows"];
const STRINGS = ["--output", "--label"];

function stamp(): string {
  return new Date().toISOString().replaceAll(":", "-").replace(/\.\d+Z$/, "Z");
}

export function readOptions(argv: readonly string[]): Options {
  const values = new Map<string, string>();
  for (let at = 0; at < argv.length; at += 2) {
    const name = argv[at] as string;
    const value = argv[at + 1];
    if (![...NUMBERS, ...BOOLEANS, ...LISTS, ...STRINGS].includes(name)) throw new Error(`unknown argument ${name}`);
    if (value === undefined) throw new Error(`${name} needs a value`);
    values.set(name, value);
  }
  const number = (name: string, fallback: number) => {
    const text = values.get(name);
    if (text === undefined) return fallback;
    const parsed = Number(text);
    if (!Number.isFinite(parsed) || parsed <= 0) throw new Error(`${name} is a positive number`);
    return parsed;
  };
  const whole = (name: string, fallback: number) => {
    const parsed = number(name, fallback);
    if (!Number.isInteger(parsed)) throw new Error(`${name} is a positive whole number`);
    return parsed;
  };
  const flag = (name: string, fallback: boolean) => {
    const text = values.get(name);
    if (text === undefined) return fallback;
    if (text !== "true" && text !== "false") throw new Error(`${name} is true or false`);
    return text === "true";
  };
  const list = (name: string) => values.get(name)?.split(",").map((item) => item.trim()).filter((item) => item !== "") ?? [];
  const rows = whole("--rows", 128);
  const attributionRows = list("--attribution-rows").map(Number);
  for (const count of [rows, ...attributionRows]) {
    if (!Number.isInteger(count) || count < 1 || count > 1024) throw new Error("row counts are whole numbers from 1 to 1024");
  }
  return {
    warmup: whole("--warmup", 100),
    samples: whole("--samples", 1000),
    rounds: whole("--rounds", 3),
    workers: whole("--workers", 4),
    poolSize: whole("--pool-size", 4),
    rows,
    attributionRows: attributionRows.length === 0 ? [1, 128, 1024] : attributionRows,
    cursorBatch: whole("--cursor-batch", 100),
    sides: list("--sides"),
    workloads: list("--workloads"),
    attribution: flag("--attribution", true),
    output: values.get("--output") ?? join(checkout, "target", "drizzle-bench", `${stamp()}.json`),
    rawSamples: flag("--raw-samples", false),
    requireIdle: flag("--require-idle", false),
    idleLoad: number("--idle-load", 4),
    allowUnverifiedAddon: flag("--allow-unverified-addon", false),
    label: values.get("--label") ?? "development",
  };
}
