// pgorm-napi's code generator without a build, in either runtime: what a
// build description must say, and the TypeScript type each described column
// kind becomes. tests/codegen-entities and checks/codegen.js build and hold a
// whole generated module.

import assert from "node:assert/strict";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { CodegenError, validate } from "../codegen/config.js";
import { manifest } from "../codegen/scaffold.js";
import { columnType, propertyKey } from "../codegen/types.js";

const fixture = join(dirname(fileURLToPath(import.meta.url)), "codegen-entities");

const base = {
  schema_version: 1,
  module: "app",
  entity_crate: ".",
  entities: [{ name: "app.Account", rust: "account::Entity", typescript: "Account" }],
};

function refused(pattern: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof CodegenError, `expected a CodegenError, got ${error}`);
    assert.match(error.message, pattern);
    return true;
  };
}

// [spec:pgorm:req:bindings.codegen/test]
test("a build description names a crate, unique registrations and exports, and Rust paths of items", () => {
  const described = validate(
    {
      ...base,
      graphs: [{ name: "app.AccountNotes", rust: "optional", typescript: "AccountNotes" }],
      sources: [{ name: "app.Pair", rust: ["account::Entity", "account::Entity"], typescript: "Pair" }],
    },
    fixture,
  );
  assert.equal(described.entity_crate, fixture);
  assert.deepStrictEqual(described.sources[0]?.rust, ["account::Entity", "account::Entity"]);
  const invalid = (change: object) => () => validate({ ...base, ...change }, fixture);
  assert.throws(invalid({ schema_version: 2 }), refused(/schema_version is 1/));
  assert.throws(invalid({ module: "my-app" }), refused(/module is a public JavaScript identifier/));
  assert.throws(invalid({ entity_crate: "missing" }), refused(/names no Cargo crate/));
  assert.throws(invalid({ extra: 1 }), refused(/"extra" is no field/));
  assert.throws(
    invalid({ entities: [{ name: "app.A", rust: "account::Entity<T>", typescript: "A" }] }),
    refused(/without expressions or generics/),
  );
  assert.throws(
    invalid({ entities: [{ name: "app.A", rust: "crate::account::Entity", typescript: "A" }] }),
    refused(/names no keyword/),
  );
  assert.throws(
    invalid({ entities: [{ name: "app.A", rust: "account::Entity", typescript: "class" }] }),
    refused(/public JavaScript identifier/),
  );
  assert.throws(
    invalid({
      entities: [
        { name: "app.A", rust: "account::Entity", typescript: "A" },
        { name: "app.B", rust: "note::Entity", typescript: "A" },
      ],
    }),
    refused(/export A is named twice/),
  );
  assert.throws(
    invalid({
      entities: [
        { name: "app.A", rust: "account::Entity", typescript: "A" },
        { name: "app.A", rust: "note::Entity", typescript: "B" },
      ],
    }),
    refused(/registration app.A is named twice/),
  );
  assert.throws(
    invalid({
      entities: [
        { name: "app.A", rust: "account::Entity", typescript: "A" },
        { name: "app.B", rust: "account::Entity", typescript: "B" },
      ],
    }),
    refused(/each Rust entity is registered once/),
  );
  assert.throws(
    invalid({ sources: [{ name: "app.S", rust: ["note::Entity"], typescript: "S" }] }),
    refused(/note::Entity is not among the entities/),
  );
  assert.throws(invalid({ sources: [{ name: "app.S", rust: [], typescript: "S" }] }), refused(/one to six/));
  assert.throws(invalid({ entities: [{ name: "", rust: "account::Entity", typescript: "A" }] }), refused(/1–255/));
});

// [spec:pgorm:req:bindings.codegen-disjoint/test]    a build description's disjoint enums are named by Rust path, registration and export, each unique and none colliding with another export, or refused before anything is written
test("a build description names each disjoint enum once", () => {
  const disjoints = [{ name: "app.Shipments", rust: "shipping::shipment::Shipment", typescript: "Shipments" }];
  assert.deepStrictEqual(validate({ ...base, disjoints }, fixture).disjoints, disjoints);
  assert.deepStrictEqual(validate(base, fixture).disjoints, []);
  const invalid = (entry: object) => () => validate({ ...base, disjoints: [...disjoints, entry] }, fixture);
  assert.throws(
    invalid({ name: "app.Shipments", rust: "shipping::Other", typescript: "Other" }),
    refused(/registration app.Shipments is named twice/),
  );
  assert.throws(
    invalid({ name: "app.Other", rust: "shipping::shipment::Shipment", typescript: "Other" }),
    refused(/each Rust disjoint enum is registered once/),
  );
  assert.throws(invalid({ name: "app.Other", rust: "shipping::Other", typescript: "Account" }), refused(/export Account is named twice/));
  assert.throws(invalid({ name: "app.Other", rust: "shipping::Other", typescript: "disjointEntity" }), refused(/public JavaScript identifier/));
  assert.throws(invalid({ name: "app.Other", rust: "shipping::Other<T>", typescript: "Other" }), refused(/without expressions or generics/));
  assert.throws(invalid({ name: "app.Other", rust: "shipping::Other", typescript: "Other", fields: {} }), refused(/"fields" is no field/));
});

// [spec:pgorm:req:napi.codegen-types/test]
test("each described kind becomes the TypeScript type its records read and its writes take", () => {
  const read = (kind: string | null, extra: object = {}) => columnType({ kind, nullable: false, ...extra }, false);
  const write = (kind: string | null, extra: object = {}) => columnType({ kind, nullable: false, ...extra }, true);
  assert.equal(read("i32"), "number");
  assert.equal(read("i64"), "bigint");
  assert.equal(write("i64"), "bigint | number");
  assert.equal(read("text", { nullable: true }), "string | null");
  assert.equal(read("decimal"), "Decimal");
  assert.equal(read("uuid"), "Uuid");
  assert.equal(read("json"), "JsonValue");
  assert.equal(read("datetime_utc"), "Temporal.Instant");
  assert.equal(read("date"), "Temporal.PlainDate");
  assert.equal(read("interval"), "Interval");
  assert.equal(write("interval"), "Interval | Temporal.Duration");
  assert.equal(read("bytes"), "Uint8Array");
  assert.equal(read("vector"), "Float32Array");
  assert.equal(read("int8range"), "Range<bigint>");
  assert.equal(read("tstzmultirange"), "Multirange<Temporal.Instant>");
  assert.equal(read('range "measure"."floatrange" of f64'), "Range<number>");
  assert.equal(read('multirange "measure"."floatmultirange" of decimal'), "Multirange<Decimal>");
  assert.equal(read('enum "app"."mood"', { values: ["calm", "busy"] }), '"calm" | "busy"');
  assert.equal(read('enum "app"."mood"'), "string");
  assert.equal(read("text[]"), "(string | null)[]");
  assert.equal(read("text[]", { rustType: "Vec<String>" }), "string[]");
  assert.equal(read("text[]", { rustType: "Vec<Option<String>>" }), "(string | null)[]");
  assert.equal(read('enum "app"."mood"[]', { values: ["calm"], rustType: "Vec<Mood>" }), '"calm"[]');
  assert.equal(read('enum "app"."mood"[]', { values: ["a", "b"], rustType: "Vec<Mood>", nullable: true }), '("a" | "b")[] | null');
  assert.equal(read(null), "PlainValue");
  assert.equal(write(null), "unknown");
  assert.equal(propertyKey("id"), "id");
  assert.equal(propertyKey("display name"), '"display name"');
});

// [spec:pgorm:req:napi.codegen+1/test]
test("a manifest's package, dependency versions and library name are read as scaffolding needs them", () => {
  const napi = manifest(join(dirname(fileURLToPath(import.meta.url)), "..", "Cargo.toml"));
  assert.equal(napi.get("package.name"), "pgorm-napi");
  assert.match(napi.get("dependencies.neon.version") ?? "", /^\d+\.\d+\.\d+$/);
  assert.equal(napi.get("lib.name"), "pgorm_napi");
  const entities = manifest(join(fixture, "Cargo.toml"));
  assert.equal(entities.get("package.name"), "pgorm-napi-codegen-entities");
});
