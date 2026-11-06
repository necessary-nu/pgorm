// A pool's TLS modes, its CA and client certificate, and a listener of its own
// over TLS. The modes are tried against a server that speaks TLS when
// PGORM_TEST_CA names the CA its certificate chains to, as
// pgorm-python/tests/with_postgres.py's server does, and the refusals and the
// plaintext server's answers run against any server.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import process from "node:process";
import { test } from "node:test";

import { ConnectionError, ConstructionError, connectListener, DatabaseError, Pool } from "../lib/index.js";
import { dsn, tlsCa } from "./support.ts";

/** The connection string with no `sslmode`, so the `tls` option decides, reaching `host` at the server's address. */
function tlsDsn(host?: string): string {
  const url = new URL(dsn());
  url.searchParams.delete("sslmode");
  if (host) {
    url.searchParams.set("hostaddr", url.hostname === "localhost" ? "127.0.0.1" : url.hostname);
    url.hostname = host;
  }
  return url.toString();
}

/** The PEM file environment variable `name` names, or `undefined`. */
function pem(name: string): string | undefined {
  const path = process.env[name];
  return path ? readFileSync(path, "utf8") : undefined;
}

async function ssl(pool: Pool): Promise<boolean> {
  return (await pool.one("SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()")).ssl as boolean;
}

const certificate = "-----BEGIN CERTIFICATE-----\nMA==\n-----END CERTIFICATE-----\n";

// [spec:pgorm:req:bindings.tls/test]
test("a TLS option the pool would not act on as asked is refused before anything is sent", () => {
  const server = "postgres://bakery@db.example.com/bakery";
  for (
    const [options, message] of [
      [{ tls: "disable", cert: certificate, key: "key" }, 'TLS configuration conflicts with tls: "disable"'],
      [{ tls: "require", ca: certificate }, 'ca is trusted only to verify the server, which tls: "require" does not do'],
      [{ tls: "prefer", ca: certificate }, 'ca is trusted only to verify the server, which tls: "prefer" does not do'],
      [{ tls: "verify-ca" }, 'tls: "verify-ca" needs ca: the certificates the server\'s certificate must chain to'],
      [{ tls: "require", cert: certificate }, "cert and key go together: give the client certificate and its private key, or neither"],
      [{ tls: "require", key: "key" }, "cert and key go together: give the client certificate and its private key, or neither"],
      [{ tls: "require", cert: "not a certificate", key: "key" }, "cert holds no certificate"],
    ] as const
  ) {
    assert.throws(() => new Pool(server, options), (error: unknown) => {
      assert.ok(error instanceof ConstructionError, `${JSON.stringify(options)}: ${error}`);
      assert.equal(error.message, message);
      return true;
    });
  }
  assert.throws(
    () => new Pool(`${server}?sslmode=require`, { tls: "prefer" }),
    (error: unknown) => error instanceof ConstructionError && /allows plaintext/.test(error.message),
  );
  // @ts-expect-error: libpq's allow is not offered
  assert.throws(() => new Pool(server, { tls: "allow" }), TypeError);
  // @ts-expect-error: a certificate is PEM text
  assert.throws(() => new Pool(server, { cert: 7 }), TypeError);
});

// [spec:pgorm:req:bindings.tls/test]
test("verify-ca checks the chain to the given CA and not the host name", async (context) => {
  const ca = tlsCa();
  if (ca === undefined) {
    context.skip("PGORM_TEST_CA names no CA: the server runs without TLS");
    return;
  }
  await using chained = new Pool(tlsDsn("wrong.test"), { tls: "verify-ca", ca });
  assert.equal(await ssl(chained), true);
  await using named = new Pool(tlsDsn("wrong.test"), { tls: "verify-full", ca });
  await assert.rejects(named.ping(), ConnectionError);
  const other = pem("PGORM_TEST_CLIENT_CERT");
  if (other !== undefined) {
    // The client certificate is no certificate authority's, so nothing chains to it.
    await using stranger = new Pool(tlsDsn("localhost"), { tls: "verify-ca", ca: other });
    await assert.rejects(stranger.ping(), ConnectionError);
  }
});

// [spec:pgorm:req:bindings.tls/test]
test("require and prefer encrypt without verifying, when the program asks", async (context) => {
  if (tlsCa() === undefined) {
    // A server without TLS: prefer falls back to plaintext, and require fails.
    await using preferring = new Pool(tlsDsn(), { tls: "prefer" });
    assert.equal(await ssl(preferring), false);
    await using requiring = new Pool(tlsDsn(), { tls: "require" });
    await assert.rejects(requiring.ping(), ConnectionError);
    context.diagnostic("PGORM_TEST_CA names no CA: checked prefer and require against plaintext");
    return;
  }
  for (const tls of ["require", "prefer"] as const) {
    await using unverified = new Pool(tlsDsn("wrong.test"), { tls });
    assert.equal(await ssl(unverified), true, tls);
  }
  // The string's own require still verifies, so with no CA it fails.
  await using strict = new Pool(`${tlsDsn("localhost")}&sslmode=require`);
  await assert.rejects(strict.ping(), ConnectionError);
});

// [spec:pgorm:req:bindings.tls/test]
test("a client certificate authenticates a role the server lets in by it alone", async (context) => {
  const ca = tlsCa();
  const cert = pem("PGORM_TEST_CLIENT_CERT");
  const key = pem("PGORM_TEST_CLIENT_KEY");
  const user = process.env.PGORM_TEST_CERT_USER;
  if (ca === undefined || cert === undefined || key === undefined || !user) {
    context.skip("needs a TLS server with a certificate-authenticated role: PGORM_TEST_CA and PGORM_TEST_CLIENT_*");
    return;
  }
  const url = new URL(tlsDsn("localhost"));
  url.username = user;
  url.password = "";
  await using certified = new Pool(url.toString(), { ca, cert, key });
  const row = await certified.one("SELECT current_user AS name, ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()");
  assert.deepEqual(row, { name: user, ssl: true });
  await using anonymous = new Pool(url.toString(), { ca });
  await assert.rejects(
    anonymous.ping(),
    (error: unknown) => error instanceof DatabaseError || error instanceof ConnectionError,
  );
});

// [spec:pgorm:req:bindings.listen+3/test]
// [spec:pgorm:req:bindings.tls/test]
test("a listener of its own connects with the TLS options a pool takes", async (context) => {
  const ca = tlsCa();
  if (ca === undefined) {
    context.skip("PGORM_TEST_CA names no CA: the server runs without TLS");
    return;
  }
  await using listener = await connectListener(tlsDsn("localhost"), { ca });
  await listener.listen("bakery_tls");
  await using pool = new Pool(tlsDsn("localhost"), { ca });
  await pool.notify("bakery_tls", "fresh");
  const { value } = await listener.next();
  assert.equal(value?.payload, "fresh");
  await assert.rejects(connectListener(tlsDsn("wrong.test"), { ca }), ConnectionError);
});
