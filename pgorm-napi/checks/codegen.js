// Generate an application's module with pgorm-napi's codegen and hold it to
// its fixture, as pgorm-python's checks/codegen.py does for its wheel.
//
//   node pgorm-napi/checks/codegen.js
//
// 1. Scaffold the project of tests/codegen-entities/application.json under
//    target/napi-codegen, and refuse to scaffold over it.
// 2. Build its native library and emit its typed module, then emit again in a
//    fresh process: the module must not change.
// 3. Type-check the fixture's consumer against the generated declarations
//    with `deno check`, its @ts-expect-error lines included.
// 4. Load a copy of the module whose recorded fingerprint differs from the
//    library's, in both runtimes: it must refuse to load.
// 5. Run the fixture's live suite against the module under node --test and
//    deno test, and write target/napi-codegen/report.json.
//
// The suite reads PGORM_TEST_DSN, or the server DATABASE_URL names, and makes
// and drops a database of its own.
// [spec:pgorm:req:napi.codegen+1]
// [spec:pgorm:req:bindings.codegen-disjoint/test]

import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import { CodegenError, load } from "../codegen/config.js";
import { scaffold } from "../codegen/scaffold.js";
import { dsn } from "../tests/support.ts";

const napi = dirname(dirname(fileURLToPath(import.meta.url)));
const checkout = dirname(napi);
const fixture = join(napi, "tests", "codegen-entities");
const target = process.env.CARGO_TARGET_DIR ?? join(checkout, "target");
const out = join(target, "napi-codegen");
const project = join(out, "project");
const cli = join(napi, "codegen", "cli.js");
const env = { ...process.env, CARGO_TARGET_DIR: target, PGORM_TEST_DSN: dsn(), NO_COLOR: "1" };

/**
 * Run a command; its status is returned when `expectFailure`, and otherwise
 * a failure ends the check.
 *
 * @param {string} command
 * @param {string[]} args
 * @param {{ cwd?: string, expectFailure?: boolean }} [options]
 */
function run(command, args, { cwd = checkout, expectFailure = false } = {}) {
  console.error(`$ ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, { cwd, env, encoding: "utf8", stdio: expectFailure ? "pipe" : "inherit" });
  if (result.error) throw result.error;
  if (expectFailure) return { status: result.status, output: `${result.stdout}${result.stderr}` };
  if (result.status !== 0) {
    console.error(`${command} exited with ${result.status}`);
    process.exit(result.status ?? 1);
  }
  return { status: 0, output: "" };
}

/** @param {boolean} condition @param {string} message */
function ensure(condition, message) {
  if (!condition) {
    console.error(message);
    process.exit(1);
  }
}

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
const description = load(join(fixture, "application.json"));
scaffold(description, project, { source: checkout });
try {
  scaffold(description, project, { source: checkout });
  ensure(false, "scaffolding over an existing project was not refused");
} catch (error) {
  ensure(error instanceof CodegenError, `scaffolding over a project failed otherwise: ${error}`);
}

run(process.execPath, [cli, "build", project]);
run(process.execPath, [cli, "emit", project]);
const emitted = ["app.js", "app.d.ts"].map((file) => readFileSync(join(project, "lib", file), "utf8"));
run(process.execPath, [cli, "emit", project]);
const again = ["app.js", "app.d.ts"].map((file) => readFileSync(join(project, "lib", file), "utf8"));
ensure(emitted[0] === again[0] && emitted[1] === again[1], "a second emission wrote a different module");

mkdirSync(join(project, "tests"), { recursive: true });
for (const file of ["consumer.ts", "codegen.test.ts"]) copyFileSync(join(fixture, "tests", file), join(project, "tests", file));
copyFileSync(join(napi, "tests", "support.ts"), join(project, "tests", "support.ts"));
run("deno", ["check", "--config", join(project, "deno.json"), join(project, "tests", "consumer.ts"), join(project, "tests", "codegen.test.ts")], {
  cwd: project,
});

const tampered = join(project, "lib", "app-tampered.js");
writeFileSync(
  tampered,
  /** @type {string} */ (emitted[0]).replace(/"[0-9a-f]{64}"/, `"${"0".repeat(64)}"`).replace('// @ts-self-types="./app.d.ts"\n', ""),
);
for (const [command, args] of /** @type {[string, string[]][]} */ ([
  [process.execPath, [tampered]],
  ["deno", ["run", "--allow-ffi", "--allow-read", "--allow-env", tampered]],
])) {
  const refused = run(command, args, { cwd: project, expectFailure: true });
  ensure(refused.status !== 0 && /regenerate it with pgorm-napi's codegen/.test(refused.output), `${command} loaded a module its library does not match: ${refused.output}`);
}
rmSync(tampered);

const suite = join(project, "tests", "codegen.test.ts");
run(process.execPath, ["--test", suite], { cwd: project });
run("deno", ["test", "--allow-ffi", "--allow-read", "--allow-env", "--allow-run", "--config", join(project, "deno.json"), suite], {
  cwd: project,
});

writeFileSync(
  join(out, "report.json"),
  `${JSON.stringify({ project, module: description.module, deterministic: true, refusedMismatch: true, runtimes: ["node", "deno"] }, null, 2)}\n`,
);
console.error(`the generated module passed its checks in both runtimes: ${join(out, "report.json")}`);
