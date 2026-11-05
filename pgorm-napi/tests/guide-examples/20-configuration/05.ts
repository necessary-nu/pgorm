// Example 5 of guide/20-configuration.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // Verified against the platform's trust store; nothing is sent yet.
  const managed = new Pool("postgres://bakery@db.example.com/bakery");
  await managed.close();

  // A CA that holds no certificate, and plain text beside a CA, are refused.
  assert.throws(() => new Pool("postgres://bakery@db.example.com/bakery", { ca: "not a certificate" }), ConstructionError);
  assert.throws(
    () => new Pool("postgres://bakery@db.example.com/bakery", { tls: "disable", ca: "not a certificate" }),
    ConstructionError,
  );

  // The bakery's own server is reached in plain text: its URL says sslmode=disable.
  const { ssl } = await pool.one("SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()");
  assert.equal(ssl, false);
}
