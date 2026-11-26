// @ts-check

// Where the native addon comes from. A build beside this module — a checkout's
// own (scripts/build.mjs) or an application's library with its registrations
// — is loaded as it is. An installed package carries none: its addon is in
// the platform package for the running process, which npm and Deno install
// from the main package's optional dependencies by `os`, `cpu` and `libc`.
// [spec:pgorm:req:napi.loading+1]
// [spec:pgorm:req:napi.platform-loading]
//
// The addon is a Node-API module, loaded through CommonJS `require` because
// that is the one loader both runtimes give a `.node` file: Node's own, and in
// Deno the node:module compatibility layer's, which needs --allow-ffi to open
// a native library and --allow-read to resolve its path.

import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import process from "node:process";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);

/**
 * The running process's platform as the platform packages are named for it:
 * `process.platform` and `process.arch`, and on Linux the C library the
 * runtime itself links, `gnu` or `musl`.
 *
 * @returns {string}
 */
export function platform() {
  const base = `${process.platform}-${process.arch}`;
  return process.platform === "linux" ? `${base}-${linuxLibc()}` : base;
}

/** @returns {"gnu" | "musl"} */
function linuxLibc() {
  // Deno states the C library it was built for and needs no permission to
  // say so, where its diagnostic report needs --allow-sys. Node.js's report
  // names glibc's version only when the process runs on glibc.
  const deno = /** @type {{ Deno?: { build: { env?: string } } }} */ (globalThis).Deno;
  if (deno) return deno.build.env === "musl" ? "musl" : "gnu";
  const report = /** @type {typeof process.report & { excludeNetwork: boolean }} */ (process.report);
  const excluded = report.excludeNetwork;
  // The report otherwise resolves the names of open sockets' peers.
  report.excludeNetwork = true;
  try {
    const { header } = /** @type {{ header?: { glibcVersionRuntime?: string } }} */ (report.getReport());
    return header?.glibcVersionRuntime ? "gnu" : "musl";
  } finally {
    report.excludeNetwork = excluded;
  }
}

/**
 * The addon's exports, from the build beside this module if there is one and
 * otherwise from the platform package; anything else is an Error naming the
 * platform, never a later failure.
 *
 * @returns {any}
 */
export function loadAddon() {
  if (existsSync(fileURLToPath(new URL("./pgorm_napi.node", import.meta.url)))) {
    return require("./pgorm_napi.node");
  }
  // The main package's name prefixes every platform package's, and its
  // version is theirs, since they are released together.
  /** @type {{ name: string, version: string, optionalDependencies?: Record<string, string> }} */
  const own = require("../package.json");
  const running = platform();
  const wanted = `${own.name}-${running}`;
  const offered = Object.keys(own.optionalDependencies ?? {})
    .filter((name) => name.startsWith(`${own.name}-`))
    .map((name) => name.slice(own.name.length + 1));
  /** @type {string} */
  let manifestPath;
  try {
    manifestPath = require.resolve(`${wanted}/package.json`);
  } catch (error) {
    const reason = offered.includes(running)
      ? `its platform package ${wanted} is not installed; reinstall ${own.name} with optional dependencies included (npm's --omit=optional leaves it out)`
      : `there is no prebuilt package for this platform; ${offered.length ? `prebuilt packages exist for ${offered.join(", ")}` : "no platform has one"}, and DISTRIBUTION.md says how to build the addon from source`;
    throw new Error(`${own.name} ${own.version} has no native addon for ${running}: ${reason}`, { cause: error });
  }
  /** @type {{ version: string, pgormFeatures?: string[] }} */
  const installed = require(manifestPath);
  if (installed.version !== own.version) {
    throw new Error(
      `${own.name} ${own.version} found ${wanted} ${installed.version} for ${running}: the two are released together, so reinstall ${own.name} to bring ${wanted} to ${own.version}`,
    );
  }
  /** @type {any} */
  let addon;
  try {
    addon = require(wanted);
  } catch (error) {
    throw new Error(`${wanted} ${installed.version} did not load on ${running}: ${error instanceof Error ? error.message : error}`, {
      cause: error,
    });
  }
  if (addon.version !== own.version) {
    throw new Error(`${wanted}'s addon is version ${addon.version}, not ${own.version}: reinstall ${own.name}`);
  }
  if (JSON.stringify(addon.features) !== JSON.stringify(installed.pgormFeatures)) {
    throw new Error(`${wanted}'s addon features do not match its package metadata: reinstall ${own.name}`);
  }
  return addon;
}
