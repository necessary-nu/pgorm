import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import os from "node:os";
import { basename, dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

import type pg from "pg";

import { checkout, type Options } from "./options.ts";

const harness = join(dirname(fileURLToPath(import.meta.url)), "..");

function run(command: string, args: readonly string[]): string | null {
  const out = spawnSync(command, args, { encoding: "utf8", cwd: checkout });
  return out.status === 0 ? out.stdout.trim() : null;
}

/** Whether a Time Machine backup is running, by `tmutil status`'s `Running`; null where tmutil does not answer. */
export function timeMachineRunning(): boolean | null {
  const status = run("tmutil", ["status"]);
  const match = status === null ? null : /\bRunning = (\d+);/.exec(status);
  return match ? match[1] !== "0" : null;
}

interface ProcessEntry {
  readonly pid: number;
  readonly cpu: number;
  readonly command: string;
}

export function processesNamed(names: readonly string[]): ProcessEntry[] {
  const listing = run("ps", ["-axo", "pid=,pcpu=,comm="]) ?? "";
  return listing
    .split("\n")
    .map((line) => /^\s*(\d+)\s+([\d.]+)\s+(.*)$/.exec(line))
    .filter((match): match is RegExpExecArray => match !== null)
    .map((match) => ({ pid: Number(match[1]), cpu: Number(match[2]), command: match[3] as string }))
    .filter((entry) => entry.pid !== process.pid && names.includes(basename(entry.command)));
}

export function idleProblems(options: Options): string[] {
  const problems: string[] = [];
  const backup = timeMachineRunning();
  if (backup === true) problems.push("a Time Machine backup is running (tmutil status reports Running = 1)");
  if (backup === null) problems.push("tmutil status did not answer, so a running backup cannot be ruled out");
  const [load] = os.loadavg() as [number, number, number];
  if (load >= options.idleLoad) problems.push(`the 1-minute load average is ${load.toFixed(2)}, not under ${options.idleLoad}`);
  const builds = processesNamed(["cargo", "rustc"]);
  if (builds.length > 0) problems.push(`${builds.length} cargo/rustc processes are running (pids ${builds.map((entry) => entry.pid).join(", ")})`);
  return problems;
}

/** The moment's load and backup state, recorded at the start and end of every round. */
export function moment() {
  return { at: new Date().toISOString(), loadavg: os.loadavg(), time_machine_running: timeMachineRunning() };
}

function packageVersion(directory: string): string | null {
  const path = join(directory, "package.json");
  return existsSync(path) ? (JSON.parse(readFileSync(path, "utf8")) as { version: string }).version : null;
}

export function packageVersions(): Record<string, string | null> {
  const names = ["drizzle-orm", "pg", "pg-pool", "pg-protocol", "pg-types", "pg-cursor", "pg-query-stream", "postgres", "typescript", "@types/node", "@types/pg"];
  return {
    "@necessary/pgorm": packageVersion(join(checkout, "pgorm-napi")),
    ...Object.fromEntries(names.map((name) => [name, packageVersion(join(harness, "node_modules", name))])),
  };
}

function sha256(path: string): string | null {
  return existsSync(path) ? createHash("sha256").update(readFileSync(path)).digest("hex") : null;
}

/**
 * Which build the loaded addon is: the one Cargo wrote for a profile, by
 * matching its bytes to that profile's library under target/. The `addon`
 * profile is the release profile with fat LTO and one codegen unit, as
 * `scripts/build.mjs --release` builds it.
 */
export function addonBuild() {
  const loaded = join(checkout, "pgorm-napi", "lib", "pgorm_napi.node");
  const hash = sha256(loaded);
  const target = process.env.CARGO_TARGET_DIR ?? join(checkout, "target");
  const profiles = Object.fromEntries(
    ["addon", "debug", "release"].map((profile) => {
      const library = ["libpgorm_napi.dylib", "libpgorm_napi.so", "pgorm_napi.dll"].map((name) => join(target, profile, name)).find(existsSync);
      return [profile, library === undefined ? null : sha256(library)];
    }),
  );
  const profile = Object.entries(profiles).find(([, built]) => built !== null && built === hash)?.[0] ?? null;
  return {
    path: loaded,
    sha256: hash,
    bytes: hash === null ? null : statSync(loaded).size,
    modified: hash === null ? null : statSync(loaded).mtime.toISOString(),
    profile,
  };
}

function files(directory: string, keep: (name: string) => boolean): string[] {
  return readdirSync(directory, { recursive: true, encoding: "utf8" })
    .filter((name) => keep(name) && !name.split("/").includes("node_modules"))
    .map((name) => join(directory, name))
    .filter((path) => statSync(path).isFile())
    .sort();
}

/**
 * SHA-256 over each file's path and bytes, in path order: the harness's own
 * sources and lockfile, and the JavaScript half of pgorm-napi, which the
 * addon's hash does not cover.
 */
export function sources() {
  const digest = (paths: readonly string[]) => {
    const hash = createHash("sha256");
    for (const path of paths) hash.update(relative(checkout, path)).update("\0").update(readFileSync(path)).update("\0");
    return hash.digest("hex");
  };
  const harnessFiles = [
    ...["package.json", "package-lock.json", "tsconfig.json"].map((name) => join(harness, name)).filter(existsSync),
    ...files(join(harness, "src"), (name) => name.endsWith(".ts")),
  ];
  const lib = files(join(checkout, "pgorm-napi", "lib"), (name) => name.endsWith(".js") || name.endsWith(".d.ts"));
  return {
    harness_sha256: digest(harnessFiles),
    pgorm_napi_lib_sha256: digest([...lib, join(checkout, "pgorm-napi", "tests", "support.ts")]),
  };
}

export function machine() {
  const cpus = os.cpus();
  return {
    node: process.version,
    node_versions: { v8: process.versions.v8, uv: process.versions.uv, napi: process.versions.napi, modules: process.versions.modules },
    execArgv: process.execArgv,
    os: `${os.type()} ${os.release()} ${os.arch()}`,
    os_version: run("sw_vers", []),
    cpu: run("sysctl", ["-n", "machdep.cpu.brand_string"]) ?? cpus[0]?.model ?? null,
    logical_cpus: cpus.length,
    total_memory_bytes: os.totalmem(),
    time_zone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    git_head: run("git", ["rev-parse", "HEAD"]),
    git_status: run("git", ["status", "--short"]),
  };
}

/** Processes a measurement shares the machine with, recorded as the run starts. */
export function contenders() {
  return {
    cargo_rustc: processesNamed(["cargo", "rustc"]),
    node: processesNamed(["node"]),
  };
}

const SETTINGS = [
  "fsync",
  "full_page_writes",
  "synchronous_commit",
  "shared_buffers",
  "wal_level",
  "max_connections",
  "plan_cache_mode",
  "jit",
];

/** The server: its version, the settings driver_baseline records and those that change what a prepared statement does, and who else is on it. */
export async function server(client: pg.Client, transport: string) {
  const version = (await client.query("SELECT version()")).rows[0].version as string;
  const settings = Object.fromEntries(
    (await client.query("SELECT name, setting FROM pg_settings WHERE name = ANY($1) ORDER BY name", [SETTINGS])).rows.map(
      (row: { name: string; setting: string }) => [row.name, row.setting],
    ),
  );
  const sessions = (
    await client.query(
      "SELECT coalesce(state, 'none') AS state, count(*)::int4 AS sessions FROM pg_stat_activity " +
        "WHERE datname IS NOT NULL AND pid <> pg_backend_pid() AND application_name <> 'pgorm-drizzle-bench' GROUP BY 1 ORDER BY 1",
    )
  ).rows;
  return { version, settings, transport, other_sessions_at_start: sessions };
}
