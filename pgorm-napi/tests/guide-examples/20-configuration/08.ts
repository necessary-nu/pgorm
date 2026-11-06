// Example 8 of guide/20-configuration.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, Pool } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  const server = "postgres://bakery@db.example.com/bakery";
  const pem = "-----BEGIN CERTIFICATE-----\nMA==\n-----END CERTIFICATE-----\n";

  // Encrypted but unverified, which only the option can ask for.
  const unverified = new Pool(server, { tls: "require" });
  await unverified.close();

  // verify-ca trusts the CA it is given, never the platform's store.
  assert.throws(() => new Pool(server, { tls: "verify-ca" }), ConstructionError);
  // A CA beside a mode that verifies nothing would trust nothing.
  assert.throws(() => new Pool(server, { tls: "require", ca: pem }), ConstructionError);
  // A client certificate comes with its key.
  assert.throws(() => new Pool(server, { cert: pem }), ConstructionError);
  // The string demands TLS, and prefer would allow plain text.
  assert.throws(() => new Pool(`${server}?sslmode=require`, { tls: "prefer" }), ConstructionError);
}
