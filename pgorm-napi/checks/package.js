// Build, pack and prove pgorm-napi's npm packages, as pgorm-python's
// checks/distribution.py does for its wheel.
//
//   node pgorm-napi/checks/package.js manifests         # versions and support.json agree
//   node pgorm-napi/checks/package.js build             # this platform's release addon
//   node pgorm-napi/checks/package.js pack [--partial]  # the main and platform packages
//   node pgorm-napi/checks/package.js install           # install and smoke-test them here
//   node pgorm-napi/checks/package.js all [--partial]   # build, pack and install
//
// `build` makes the release addon with the crate's `addon` profile and
// refuses one carrying the debug-only probe exports or a symbol table its
// loading does not need. `pack` makes the main package and one package per
// release platform in support.json from the addons built, verifies their
// contents, and packs them with `npm pack`; a release packs every release
// platform, so CI builds each on its own runner first, and `--partial` packs
// only the ones built here, its main package naming only those. `install`
// serves the tarballs from a registry of its own on loopback, installs them
// into fresh Node.js and Deno projects as an application would — `npm install
// @necessary/pgorm`, and `npm:@necessary/pgorm` in a `nodeModulesDir: "auto"`
// Deno project — runs tests/package/smoke.test.ts in each against the server
// PGORM_TEST_DSN or DATABASE_URL names, and holds the loader to refusing a
// missing, a mismatched and an unoffered platform package in both runtimes.
//
// Everything is written under target/napi-package:
//   binaries/<platform>/  an addon and its binary.json, per platform built
//   stage/                each package's files, as packed
//   tarballs/             the .tgz files and packages.json
//   projects/             the throwaway projects `install` creates
//   report.json           what `install` proved; `passed` stays false until it has
// Nothing is published: no step talks to a registry but the one served here.
// [spec:pgorm:req:napi.distribution]

import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { basename, dirname, join } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import { manifest as cargoManifest } from "../codegen/scaffold.js";
import { platform } from "../lib/native.js";
import { buildLibrary } from "../scripts/build.mjs";
import { localSymbols } from "../tests/package/symbols.ts";
import { dsn } from "../tests/support.ts";
import { generate, NOTICES } from "./notices.js";

const napi = dirname(dirname(fileURLToPath(import.meta.url)));
const checkout = dirname(napi);
const target = process.env.CARGO_TARGET_DIR ?? join(checkout, "target");
const out = join(target, "napi-package");
const binaries = join(out, "binaries");
const tarballs = join(out, "tarballs");
const projects = join(out, "projects");

/** The exports only a debug build carries (src/probes.rs). */
const PROBES = ["probePanic", "probeDropQueue"];

/** How long one install or suite may run before it counts as hung. */
const DEADLINE = 10 * 60 * 1000;

/** @param {string} message @returns {never} */
function fail(message) {
  console.error(message);
  process.exit(1);
}

/** @param {unknown} condition @param {string} message @returns {asserts condition} */
function ensure(condition, message) {
  if (!condition) fail(message);
}

/** @param {string} path @returns {any} */
function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

/** @param {string} path @param {unknown} value */
function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

/** @param {string} algorithm @param {Uint8Array} data @param {"hex" | "base64"} encoding */
function hash(algorithm, data, encoding) {
  return createHash(algorithm).update(data).digest(encoding);
}

/**
 * Run a command, its output passed through unless captured, while this
 * process's event loop stays free to serve the registry.
 *
 * @param {string} command
 * @param {string[]} args
 * @param {{ cwd?: string, env?: NodeJS.ProcessEnv, capture?: boolean, expectFailure?: boolean }} [options]
 * @returns {Promise<{ status: number | null, output: string }>}
 */
function run(command, args, { cwd = checkout, env = process.env, capture = false, expectFailure = false } = {}) {
  console.error(`$ ${command} ${args.join(" ")}`);
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd, env, stdio: ["ignore", capture || expectFailure ? "pipe" : "inherit", capture || expectFailure ? "pipe" : "inherit"] });
    let output = "";
    child.stdout?.setEncoding("utf8").on("data", (chunk) => (output += chunk));
    child.stderr?.setEncoding("utf8").on("data", (chunk) => (output += chunk));
    const timer = setTimeout(() => child.kill("SIGKILL"), DEADLINE);
    child.on("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.on("close", (status, signal) => {
      clearTimeout(timer);
      if (!expectFailure && status !== 0) {
        fail(`${command} ${args.join(" ")} ended with ${signal ?? status}${capture ? `:\n${output}` : ""}`);
      }
      resolve({ status, output });
    });
  });
}

/**
 * The checkout's manifests, held to agree: the npm version is the crate's,
 * the checkout's own package.json stays private, and support.json names each
 * platform and its package by one scheme.
 */
// [spec:pgorm:req:napi.support]
function manifests() {
  const pkg = readJson(join(napi, "package.json"));
  const support = readJson(join(napi, "support.json"));
  const crate = cargoManifest(join(napi, "Cargo.toml")).get("package.version");
  ensure(pkg.version === crate, `package.json is version ${pkg.version} and Cargo.toml ${crate}: the npm version is the crate's`);
  ensure(pkg.private === true, "pgorm-napi/package.json must stay private: the packages are made by this check, never by packing the checkout");
  ensure(pkg.name === support.npm_package, `support.json names the package ${support.npm_package}, package.json ${pkg.name}`);
  /** @type {Set<string>} */
  const seen = new Set();
  for (const entry of support.platforms) {
    const libc = { glibc: "gnu", musl: "musl" }[/** @type {string} */ (entry.libc)];
    ensure((entry.os === "linux") === (entry.libc !== undefined), `${entry.platform}: a Linux platform names its C library, and only a Linux platform does`);
    ensure(entry.libc === undefined || libc !== undefined, `${entry.platform}: libc is glibc or musl, not ${entry.libc}`);
    const name = [entry.os, entry.cpu, libc].filter(Boolean).join("-");
    ensure(entry.platform === name, `support.json's ${entry.platform} should be named ${name}`);
    ensure(entry.package === `${pkg.name}-${name}`, `${entry.platform}'s package should be ${pkg.name}-${name}`);
    ensure(typeof entry.release === "boolean", `${entry.platform} does not say whether a release builds it`);
    ensure(!seen.has(name), `support.json lists ${name} twice`);
    seen.add(name);
  }
  /** @type {any[]} */
  const releases = support.platforms.filter((/** @type {any} */ entry) => entry.release);
  ensure(releases.length > 0, "support.json names no release platform");
  for (const tested of support.tested_combinations) {
    ensure(releases.some((entry) => entry.platform === tested.platform), `${tested.platform} is recorded as tested but no release builds it`);
  }
  return { pkg, support, releases, version: /** @type {string} */ (pkg.version) };
}

/**
 * Refuse an addon that is not a stripped release build of this version: one
 * carrying a probe export, checked by name in its bytes — which works for any
 * platform's binary — and, where it can be loaded here, by its exports; or
 * one still carrying symbols its loading does not need, read from its ELF or
 * Mach-O symbol tables, which any runner can read for any platform's binary.
 *
 * @param {string} path
 * @param {string} version
 * @param {{ load: boolean }} options
 */
// [spec:pgorm:req:napi.release-builds+1]
function verifyAddon(path, version, { load }) {
  /** @type {string[] | undefined} */
  let exports;
  if (load) {
    const loaded = spawnSync(
      process.execPath,
      ["-e", `const addon = require(${JSON.stringify(path)}); console.log(JSON.stringify({ version: addon.version, exports: Object.keys(addon).sort() }));`],
      { encoding: "utf8" },
    );
    ensure(loaded.status === 0, `${path} did not load: ${loaded.stderr}`);
    const facts = JSON.parse(loaded.stdout);
    const probes = PROBES.filter((probe) => facts.exports.includes(probe));
    ensure(probes.length === 0, `${path} exports ${probes.join(" and ")}, which only a debug build carries: package a release build`);
    ensure(facts.version === version, `${path} reports version ${facts.version}, not ${version}`);
    exports = facts.exports;
  }
  const data = readFileSync(path);
  const named = PROBES.filter((probe) => data.includes(probe));
  ensure(named.length === 0, `${path} names ${named.join(" and ")}, which only a debug build carries: package a release build`);
  const symbols = localSymbols(data);
  ensure(symbols !== undefined, `${path} is neither a 64-bit ELF nor a Mach-O library, so whether it is stripped cannot be read`);
  ensure(symbols === 0, `${path} carries ${symbols} symbols beyond the ones it is loaded by: package the addon profile's stripped build`);
  return { sha256: hash("sha256", data, "hex"), size: data.length, exports };
}

function build() {
  const { support, version } = manifests();
  const running = platform();
  const entry = support.platforms.find((/** @type {any} */ candidate) => candidate.platform === running);
  ensure(entry, `support.json lists no platform ${running}`);
  const library = buildLibrary({ release: true });
  const directory = join(binaries, running);
  rmSync(directory, { recursive: true, force: true });
  // Checked under a name of its own, so a refused addon never reaches
  // binaries/ for the pack step to find.
  const candidate = join(out, "candidate", "pgorm_napi.node");
  rmSync(dirname(candidate), { recursive: true, force: true });
  mkdirSync(dirname(candidate), { recursive: true });
  copyFileSync(library, candidate);
  const facts = verifyAddon(candidate, version, { load: true });
  mkdirSync(directory, { recursive: true });
  const addon = join(directory, "pgorm_napi.node");
  copyFileSync(candidate, addon);
  rmSync(dirname(candidate), { recursive: true, force: true });
  writeJson(join(directory, "binary.json"), {
    platform: running,
    package: entry.package,
    release: entry.release,
    version,
    sha256: facts.sha256,
    size: facts.size,
    exports: facts.exports?.length,
    probes: [],
    node: process.version,
  });
  console.error(`release addon for ${running}: ${addon}`);
}

function verifyNotices() {
  const { manifest, notices } = generate();
  for (const [name, content] of /** @type {[string, string][]} */ ([
    ["DEPENDENCIES.json", manifest],
    ["THIRD_PARTY_NOTICES.txt", notices],
  ])) {
    const path = join(NOTICES, name);
    ensure(existsSync(path) && readFileSync(path, "utf8") === content, `${name} is stale; run node pgorm-napi/checks/notices.js --write`);
  }
}

/** pgorm's own licences, which every package carries. */
const LICENCES = ["LICENSE-APACHE", "LICENSE-MIT"];

/**
 * The packages are scoped, and npm publishes a scoped package as restricted
 * unless its manifest says otherwise.
 */
const PUBLISH = { access: "public" };

/**
 * The main package's manifest, from the checkout's: the module and its
 * declarations, and the platform packages as optional dependencies.
 *
 * @param {any} pkg
 * @param {any[]} platforms
 */
// [spec:pgorm:req:napi.packages+1]
function mainManifest(pkg, platforms) {
  return {
    name: pkg.name,
    version: pkg.version,
    description: pkg.description,
    license: pkg.license,
    repository: pkg.repository,
    type: pkg.type,
    engines: pkg.engines,
    exports: pkg.exports,
    files: ["lib/*.js", "lib/*.d.ts", "DISTRIBUTION.md", "support.json", ...LICENCES],
    // Each platform package at this package's own version, exactly: the
    // module and the addon are one release, and a range here could pair a
    // module with an addon it was not built against. A project's own
    // dependency on the main package takes a range as usual.
    optionalDependencies: Object.fromEntries(platforms.map((entry) => [entry.package, pkg.version])),
    publishConfig: PUBLISH,
  };
}

/**
 * A platform package's manifest: its addon, the notices of what is compiled
 * into it, and the `os`, `cpu` and `libc` a package manager selects it by.
 *
 * @param {any} pkg
 * @param {any} entry
 */
function platformManifest(pkg, entry) {
  return {
    name: entry.package,
    version: pkg.version,
    description: `The native addon of ${pkg.name} ${pkg.version} for ${entry.platform}`,
    license: pkg.license,
    repository: pkg.repository,
    os: [entry.os],
    cpu: [entry.cpu],
    ...(entry.libc ? { libc: [entry.libc] } : {}),
    engines: pkg.engines,
    main: "pgorm_napi.node",
    files: ["pgorm_napi.node", "DEPENDENCIES.json", "THIRD_PARTY_NOTICES.txt", ...LICENCES],
    publishConfig: PUBLISH,
  };
}

/** @param {boolean} partial */
function pack(partial) {
  const { pkg, releases, version } = manifests();
  verifyNotices();
  /** @type {Set<string>} */
  const built = new Set();
  for (const name of existsSync(binaries) ? readdirSync(binaries) : []) {
    const recorded = join(binaries, name, "binary.json");
    ensure(existsSync(recorded), `binaries/${name} has no binary.json: its addon did not come from the build step`);
    const record = readJson(recorded);
    ensure(record.platform === name, `binaries/${name} holds the addon for ${record.platform}`);
    ensure(record.version === version, `the ${name} addon is version ${record.version}, not ${version}`);
    ensure(releases.some((entry) => entry.platform === name), `${name} is not a release platform in support.json, so no package is made for it`);
    const facts = verifyAddon(join(binaries, name, "pgorm_napi.node"), version, { load: false });
    ensure(facts.sha256 === record.sha256, `the ${name} addon differs from the one its build recorded`);
    built.add(name);
  }
  ensure(built.size > 0, "no addon has been built; run the build step first");
  const missing = releases.filter((entry) => !built.has(entry.platform)).map((entry) => entry.platform);
  ensure(partial || missing.length === 0, `no addon for ${missing.join(", ")}: a release packs every release platform, and --partial only those built`);
  const platforms = releases.filter((entry) => built.has(entry.platform));

  const stage = join(out, "stage");
  rmSync(stage, { recursive: true, force: true });
  rmSync(tarballs, { recursive: true, force: true });
  mkdirSync(tarballs, { recursive: true });

  const main = join(stage, pkg.name);
  mkdirSync(join(main, "lib"), { recursive: true });
  for (const file of readdirSync(join(napi, "lib"))) {
    if (/\.(js|d\.ts)$/.test(file)) copyFileSync(join(napi, "lib", file), join(main, "lib", file));
  }
  for (const file of ["README.md", "DISTRIBUTION.md", "support.json"]) copyFileSync(join(napi, file), join(main, file));
  for (const file of LICENCES) copyFileSync(join(checkout, file), join(main, file));
  writeJson(join(main, "package.json"), mainManifest(pkg, platforms));

  const directories = [main];
  for (const entry of platforms) {
    const directory = join(stage, entry.package);
    mkdirSync(directory, { recursive: true });
    copyFileSync(join(binaries, entry.platform, "pgorm_napi.node"), join(directory, "pgorm_napi.node"));
    for (const file of ["DEPENDENCIES.json", "THIRD_PARTY_NOTICES.txt"]) copyFileSync(join(NOTICES, file), join(directory, file));
    for (const file of LICENCES) copyFileSync(join(checkout, file), join(directory, file));
    writeFileSync(
      join(directory, "README.md"),
      `# ${entry.package}\n\nThe native addon of [${pkg.name}](https://www.npmjs.com/package/${pkg.name}) ${version} for ${entry.platform}, which ${pkg.name} installs as an optional dependency and loads itself. Depend on ${pkg.name}, not on this package.\n\nTHIRD_PARTY_NOTICES.txt and DEPENDENCIES.json carry the notices of the Rust dependencies compiled into the addon.\n`,
    );
    writeJson(join(directory, "package.json"), platformManifest(pkg, entry));
    directories.push(directory);
  }

  const npmEnv = { ...process.env, npm_config_update_notifier: "false" };
  const packed = directories.map((directory) => {
    const result = spawnSync("npm", ["pack", "--json", "--ignore-scripts", "--pack-destination", tarballs], {
      cwd: directory,
      encoding: "utf8",
      env: npmEnv,
    });
    ensure(result.status === 0, `npm pack failed in ${directory}:\n${result.stderr}`);
    const [info] = JSON.parse(result.stdout);
    /** @type {string[]} */
    const files = info.files.map((/** @type {{ path: string }} */ file) => file.path).sort();
    return { name: info.name, version: info.version, tarball: info.filename, integrity: info.integrity, shasum: info.shasum, size: info.size, files };
  });

  const [mainPacked, ...platformPacked] = packed;
  ensure(mainPacked, "npm packed nothing");
  const allowed = /^(package\.json|README\.md|DISTRIBUTION\.md|support\.json|LICENSE-APACHE|LICENSE-MIT|lib\/[^/]+\.(js|d\.ts))$/;
  const stray = mainPacked.files.filter((file) => !allowed.test(file));
  ensure(stray.length === 0, `the main package carries ${stray.join(", ")}`);
  for (const file of ["lib/index.js", "lib/index.d.ts", "lib/pgorm.js", "lib/native.js"]) {
    ensure(mainPacked.files.includes(file), `the main package lacks ${file}`);
  }
  const expected = ["DEPENDENCIES.json", "LICENSE-APACHE", "LICENSE-MIT", "README.md", "THIRD_PARTY_NOTICES.txt", "package.json", "pgorm_napi.node"];
  for (const info of platformPacked) {
    ensure(JSON.stringify(info.files) === JSON.stringify(expected), `${info.name} carries ${info.files.join(", ")}, not ${expected.join(", ")}`);
  }
  writeJson(join(tarballs, "packages.json"), {
    name: pkg.name,
    version,
    partial: missing.length > 0,
    platforms: platforms.map((entry) => entry.platform),
    unbuilt: missing,
    packages: packed,
  });
  console.error(`packed ${packed.map((info) => info.tarball).join(", ")} into ${tarballs}`);
}

/**
 * Serve the tarballs as an npm registry on loopback: a document per package
 * and its tarball, and 404 for any other name, so nothing an install resolves
 * comes from anywhere else.
 *
 * @param {string[]} files
 */
async function serveRegistry(files) {
  /** @type {Map<string, object>} */
  const documents = new Map();
  /** @type {Map<string, Buffer>} */
  const archives = new Map();
  const server = createServer((request, response) => {
    const path = decodeURIComponent(new URL(request.url ?? "/", "http://registry").pathname.slice(1));
    const archive = archives.get(path);
    const document = documents.get(path);
    if (request.method === "GET" && archive) {
      response.writeHead(200, { "content-type": "application/octet-stream", "content-length": archive.length });
      response.end(archive);
    } else if (request.method === "GET" && document) {
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify(document));
    } else {
      response.writeHead(404, { "content-type": "application/json" });
      response.end(JSON.stringify({ error: "not found" }));
    }
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(undefined)));
  const address = server.address();
  ensure(address && typeof address === "object", "the registry has no address");
  const url = `http://127.0.0.1:${address.port}/`;
  for (const file of files) {
    const data = readFileSync(file);
    const listed = spawnSync("tar", ["-xzOf", file, "package/package.json"], { encoding: "utf8" });
    ensure(listed.status === 0, `${file} has no package/package.json: ${listed.stderr}`);
    const manifest = JSON.parse(listed.stdout);
    const path = `${manifest.name}/-/${basename(file)}`;
    archives.set(path, data);
    documents.set(manifest.name, {
      name: manifest.name,
      "dist-tags": { latest: manifest.version },
      versions: {
        [manifest.version]: {
          ...manifest,
          dist: {
            tarball: `${url}${path}`,
            shasum: hash("sha1", data, "hex"),
            integrity: `sha512-${hash("sha512", data, "base64")}`,
          },
        },
      },
    });
  }
  return {
    url,
    close() {
      server.closeAllConnections();
      server.close();
    },
  };
}

/**
 * A fresh project directory holding `files`, objects written as JSON.
 *
 * @param {string} directory
 * @param {Record<string, string | object>} files
 */
function project(directory, files) {
  mkdirSync(directory, { recursive: true });
  for (const [name, content] of Object.entries(files)) {
    writeFileSync(join(directory, name), typeof content === "string" ? content : `${JSON.stringify(content, null, 2)}\n`);
  }
  for (const file of ["smoke.test.ts", "exit.ts", "symbols.ts"]) copyFileSync(join(napi, "tests", "package", file), join(directory, file));
}

async function install() {
  const listing = readJson(join(tarballs, "packages.json"));
  const report = join(out, "report.json");
  writeJson(report, { passed: false, status: "running" });
  /** @type {string} */
  const name = listing.name;
  /** @type {string} */
  const version = listing.version;
  const running = platform();
  const wanted = `${name}-${running}`;
  ensure(listing.platforms.includes(running), `the packed packages hold no addon for ${running}, only for ${listing.platforms.join(", ")}`);
  rmSync(projects, { recursive: true, force: true });
  mkdirSync(projects, { recursive: true });
  writeFileSync(join(projects, "npmrc"), "");

  const registry = await serveRegistry(listing.packages.map((/** @type {{ tarball: string }} */ info) => join(tarballs, info.tarball)));
  try {
    const npmrc = `registry=${registry.url}\n`;
    const env = {
      ...process.env,
      NO_COLOR: "1",
      PGORM_TEST_DSN: dsn(),
      PGORM_NAPI_VERSION: version,
      PGORM_NAPI_PLATFORM_PACKAGE: wanted,
      PGORM_NAPI_PARTIAL: listing.partial ? "1" : "0",
      // npm and Deno resolve every package from the registry served here,
      // with caches of their own and no user configuration.
      npm_config_registry: registry.url,
      npm_config_userconfig: join(projects, "npmrc"),
      npm_config_cache: join(projects, "npm-cache"),
      npm_config_update_notifier: "false",
      npm_config_audit: "false",
      npm_config_fund: "false",
      NPM_CONFIG_REGISTRY: registry.url,
      DENO_DIR: join(projects, "deno-dir"),
    };
    const dependencies = { [name]: `^${version}` };
    const others = listing.platforms.filter((/** @type {string} */ other) => other !== running).map((/** @type {string} */ other) => `${name}-${other}`);

    // Node.js: `npm install @necessary/pgorm`, which picks the platform package.
    const nodeProject = join(projects, "node");
    project(nodeProject, {
      "package.json": { name: "smoke", private: true, type: "module", dependencies },
      ".npmrc": npmrc,
    });
    await run("npm", ["install", "--ignore-scripts"], { cwd: nodeProject, env });
    const modules = join(nodeProject, "node_modules");
    ensure(existsSync(join(modules, wanted, "pgorm_napi.node")), `npm did not install ${wanted}`);
    for (const other of others) ensure(!existsSync(join(modules, other)), `npm installed ${other} on ${running}`);
    ensure(!existsSync(join(modules, name, "lib", "pgorm_napi.node")), "the installed main package carries an addon");
    await run(process.execPath, ["--test", "smoke.test.ts"], { cwd: nodeProject, env });

    // Deno: `npm:@necessary/pgorm` through the import map, into a
    // node_modules Deno manages itself, whose store folders spell a scoped
    // name's `/` as `+`.
    const denoProject = join(projects, "deno");
    project(denoProject, {
      "deno.json": { nodeModulesDir: "auto", imports: { [name]: `npm:${name}@^${version}` } },
      ".npmrc": npmrc,
    });
    await run("deno", ["install"], { cwd: denoProject, env });
    const store = join(denoProject, "node_modules", ".deno");
    /** @param {string} packageName */
    const stored = (packageName) =>
      join(store, `${packageName.replace("/", "+")}@${version}`, "node_modules", packageName, "pgorm_napi.node");
    ensure(existsSync(stored(wanted)), `deno did not install ${wanted}`);
    for (const other of others) ensure(!existsSync(stored(other)), `deno installed ${other} on ${running}`);
    await run("deno", ["test", "--allow-ffi", "--allow-read", "--allow-env", "--allow-run", "smoke.test.ts"], { cwd: denoProject, env });

    // The loader's refusals, in both runtimes: npm installs each project and
    // Deno reads its node_modules as it is.
    // [spec:pgorm:req:napi.platform-loading/test]
    const refusals = [
      {
        // Installed without optional dependencies.
        case: "missing",
        omit: true,
        tamper() {},
        pattern: `${name} ${version} has no native addon for ${running}: its platform package ${wanted} is not installed`,
      },
      {
        // A platform package left from another release.
        case: "mismatched",
        omit: false,
        /** @param {string} modules */
        tamper(modules) {
          const path = join(modules, wanted, "package.json");
          writeJson(path, { ...readJson(path), version: "0.0.0-mismatched" });
        },
        pattern: `${name} ${version} found ${wanted} 0.0.0-mismatched for ${running}`,
      },
      {
        // A platform no release builds.
        case: "unoffered",
        omit: true,
        /** @param {string} modules */
        tamper(modules) {
          const path = join(modules, name, "package.json");
          const manifest = readJson(path);
          delete manifest.optionalDependencies[wanted];
          writeJson(path, manifest);
        },
        pattern: `${name} ${version} has no native addon for ${running}: there is no prebuilt package for this platform`,
      },
    ];
    for (const refusal of refusals) {
      const directory = join(projects, refusal.case);
      project(directory, {
        "package.json": { name: refusal.case, private: true, type: "module", dependencies },
        ".npmrc": npmrc,
        "deno.json": { nodeModulesDir: "manual" },
        "load.js": `import ${JSON.stringify(name)};\n`,
      });
      await run("npm", ["install", "--ignore-scripts", ...(refusal.omit ? ["--omit=optional"] : [])], { cwd: directory, env });
      refusal.tamper(join(directory, "node_modules"));
      for (const [command, args] of /** @type {[string, string[]][]} */ ([
        [process.execPath, ["load.js"]],
        ["deno", ["run", "--allow-ffi", "--allow-read", "--allow-env", "load.js"]],
      ])) {
        const outcome = await run(command, args, { cwd: directory, env, expectFailure: true });
        ensure(
          outcome.status !== 0 && outcome.output.includes(refusal.pattern),
          `${basename(command)} did not refuse the ${refusal.case} platform package as expected:\n${outcome.output}`,
        );
        console.error(`${basename(command)} refused the ${refusal.case} platform package`);
      }
    }

    const versions = await Promise.all(
      /** @type {[string, string[]][]} */ ([["npm", ["--version"]], ["deno", ["--version"]]]).map(async ([command, args]) =>
        (await run(command, args, { capture: true, env })).output.split("\n")[0]?.trim()
      ),
    );
    writeJson(report, {
      passed: true,
      platform: running,
      version,
      partial: listing.partial,
      packages: listing.packages.map((/** @type {any} */ info) => ({ name: info.name, tarball: info.tarball, integrity: info.integrity })),
      installed_from: "a registry serving only these tarballs on loopback",
      runtimes: { node: process.version, npm: versions[0], deno: versions[1] },
      smoke: ["node --test", "deno test"],
      refusals: refusals.map((refusal) => refusal.case),
    });
    console.error(`the packages installed and passed their smoke suite in Node.js and Deno: ${report}`);
  } finally {
    registry.close();
  }
}

const [command] = process.argv.slice(2);
const partial = process.argv.includes("--partial");
switch (command) {
  case "manifests":
    manifests();
    verifyNotices();
    console.error("the manifests, support.json and notices agree");
    break;
  case "build":
    build();
    break;
  case "pack":
    pack(partial);
    break;
  case "install":
    await install();
    break;
  case "all":
    build();
    pack(partial);
    await install();
    break;
  default:
    fail("usage: node pgorm-napi/checks/package.js manifests | build | pack [--partial] | install | all [--partial]");
}
