// The `bench-drizzle` nplan check: install the harness's locked dependencies,
// then type-check it. A check's command runs without a shell, so the two
// steps are one script.

import { spawnSync } from "node:child_process";
import { dirname } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const harness = dirname(fileURLToPath(import.meta.url));

for (const args of [["ci", "--prefer-offline", "--no-audit", "--no-fund"], ["run", "typecheck"]]) {
  const { status, error } = spawnSync("npm", args, { cwd: harness, stdio: "inherit" });
  if (error) throw error;
  if (status !== 0) process.exit(status ?? 1);
}
