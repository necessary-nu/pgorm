// The schema family's parity cases — schemas, tables, their columns, keys and
// constraints and their alterations, indexes, types, domains, sequences,
// views, materialized views and tables created from a query, extensions and
// comments — held to `schema.json` as `select.ts`'s are to theirs. A DDL
// statement binds nothing, so every case's values are empty.
// [spec:pgorm:req:bindings.schema/test]
// [spec:pgorm:req:bindings.schema-tables/test]
// [spec:pgorm:req:bindings.schema-alter/test]
// [spec:pgorm:req:bindings.schema-indexes/test]
// [spec:pgorm:req:bindings.schema-types/test]
// [spec:pgorm:req:bindings.schema-sequences/test]
// [spec:pgorm:req:bindings.schema-views+3/test]
// [spec:pgorm:req:bindings.schema-schemas/test]
// [spec:pgorm:req:bindings.schema-domains+2/test]

import {
  alterDomain,
  alterSequence,
  alterTable,
  alterType,
  type Builder,
  call,
  col,
  ColumnDef,
  commentOnColumn,
  commentOnTable,
  CreatedRange,
  createDomain,
  createExtension,
  createIndex,
  createMaterializedView,
  createRecursiveView,
  createSchema,
  createSequence,
  createTable,
  createTableAs,
  createType,
  createView,
  DataType,
  Decimal,
  domainValue,
  dropDomain,
  dropExtension,
  dropIndex,
  dropMaterializedView,
  dropSchema,
  dropSequence,
  dropTable,
  dropType,
  dropView,
  refreshMaterializedView,
  renameColumn,
  renameConstraint,
  renameSchema,
  renameSequence,
  renameTable,
  select,
  Table,
  truncateTable,
  TypeName,
  Value,
} from "../../lib/index.js";

const booking = new Table("booking", { schema: "app" });
const quantity = new TypeName("quantity", { schema: "app" });
const room = new Table("room", { schema: "app" });
const mood = new TypeName("mood", { schema: "app" });

/** Bookings of a room above 10 whose note is not `it's`, its values written as literals. */
const largeRooms = () =>
  select(col("id"), col("room")).from(booking).where(col("room").gt(10)).where(col("note").ne("it's"));

const counting = () =>
  select(col("id").as("n"))
    .from(booking)
    .unionAll(select(col("n").add(1)).from(new Table("counter")).where(col("n").lt(5)));

const app = (name: string) => new Table(name, { schema: "app" });

export const cases: Record<string, () => Builder> = {
  "create-table-columns": () =>
    createTable(booking)
      .column(new ColumnDef("id", "bigint").identity("always", { startWith: 10, incrementBy: 2, maxValue: null }))
      .column(new ColumnDef("legacy", "integer").autoIncrement())
      .column(
        new ColumnDef("seats", "smallint")
          .notNull({ name: "seats_present", noInherit: true })
          .default(1)
          .check(col("seats").gt(0), { name: "seats_positive", enforcement: "notEnforced" }),
      )
      .column(new ColumnDef("note", "text").null().collate("C", { schema: "pg_catalog" }))
      .column(new ColumnDef("price", new DataType("numeric", { precision: 10, scale: 2 })).default(new Decimal("9.50")))
      .column(new ColumnDef("doubled", "integer").generated(col("seats").mul(2), "stored"))
      .column(new ColumnDef("halved", "integer").generated(col("seats").div(2), "virtual"))
      .column(new ColumnDef("serial", "bigint").identity("byDefault")),
  "create-table-keys": () =>
    createTable(booking)
      .ifNotExists()
      .column(new ColumnDef("id", "bigint"))
      .column(new ColumnDef("room", "integer"))
      .column(new ColumnDef("during", "tstzrange"))
      .primaryKey("id", { name: "booking_key", include: ["room"], deferrability: "deferrableInitiallyImmediate" })
      .unique(["room", "id"], { nullsNotDistinct: true })
      .unique("room", { withoutOverlaps: "during", name: "no_double_booking" })
      .foreignKey(["room", "id"], room, ["id", "booking"], {
        name: "booking_room",
        onDelete: "cascade",
        onUpdate: "setNull",
        deferrability: "deferrableInitiallyDeferred",
        enforcement: "notEnforced",
      })
      .foreignKey("room", room, "id", { period: ["during", "open"] })
      .check(col("room").lt(1000), { name: "room_small", noInherit: true, enforcement: "enforced" }),
  "create-table-typed-columns": () =>
    createTable("feeling")
      .column(new ColumnDef("mood", mood).default(new Value("calm", mood)))
      .column(new ColumnDef("moods", new DataType(mood).array()))
      .column(new ColumnDef("tags", new DataType("varchar", { length: 20 }).array()))
      .column(new ColumnDef("span", new CreatedRange("floatrange", "f64", { schema: "app" })))
      .column(new ColumnDef("token", "uuid").default(call("gen_random_uuid"))),
  "alter-table-actions": () =>
    alterTable(booking)
      .addColumn(new ColumnDef("extra", "text").notNull(), { ifNotExists: true })
      .dropColumn("legacy")
      .addPrimaryKey(["id"], { name: "booking_pkey" })
      .addUnique("room", { nullsNotDistinct: true, deferrability: "notDeferrable" })
      .addForeignKey("room", room, "id", { name: "room_fk", notValid: true, onDelete: "restrict" })
      .addCheck(col("seats").lte(9), { name: "few_seats", notValid: true, noInherit: true })
      .addNotNull("note", { name: "note_present", noInherit: true, notValid: true })
      .dropConstraint("old", { ifExists: true, behavior: "cascade" })
      .validateConstraint("room_fk")
      .alterConstraint("room_fk", "notEnforced")
      .alterConstraint("note_present", "inherit")
      .setExpression("doubled", col("seats").mul(3))
      .dropExpression("halved", { ifExists: true }),
  "alter-table-modify-column": () =>
    alterTable(booking)
      .modifyColumn(new ColumnDef("note", "varchar").collate("C").default("none").notNull())
      .modifyColumn(new ColumnDef("seats").null())
      .modifyColumn(new ColumnDef("room").notNull({ name: "room_present" }))
      .modifyColumn(new ColumnDef("serial").identity("always", { cache: 5 }))
      .modifyColumn(new ColumnDef("price").check(col("price").gte(0))),
  "drop-tables": () => dropTable([booking, "scratch"], { ifExists: true, behavior: "restrict" }),
  "rename-table": () => renameTable(booking, "reservation"),
  "rename-column": () => renameColumn(booking, "note", "remark"),
  "rename-constraint": () => renameConstraint(booking, "room_fk", "booking_room_fk"),
  "truncate-table": () => truncateTable(booking),
  "create-index-entries": () =>
    createIndex(booking, { on: call("lower", col("note")), order: "desc", operatorClass: "text_pattern_ops" }, {
      name: "booking_note",
    })
      .column("room")
      .column({ on: "id", order: "asc" })
      .nullsNotDistinct()
      .ifNotExists()
      .include(["seats"])
      .where(col("seats").gt(0))
      .where(col("room").isNull().not()),
  "create-index-methods": () => createIndex("doc", col("body")).using("gin").column({ on: col("tags") }),
  "create-index-access-method": () => createIndex(booking, "during").using("gist"),
  "drop-index": () => dropIndex(booking, "booking_note", { ifExists: true }),
  "create-shell-type": () => createType("later"),
  "create-enum": () => createType(mood).values(["calm", "O'Brien"]).values([""]),
  "create-composite": () =>
    createType("pair").attribute("a", "integer").attribute("b", "text", {
      collation: { name: "C", schema: "pg_catalog" },
    }).attribute("c", mood),
  "create-empty-composite": () => createType("nothing").values(["x"]).asComposite(),
  "create-range": () =>
    createType(new TypeName("floatrange", { schema: "app" })).asRange("double", {
      subtypeDiff: "float8mi",
      subtypeOpclass: "float8_ops",
      multirangeTypeName: new TypeName("floatmultirange", { schema: "app" }),
    }),
  "create-text-range": () => createType("textrange").asRange("text", { collation: "C" }),
  "alter-enum-add-value": () => alterType(mood).addValue("tense", { after: "calm" }),
  "alter-enum-rename-value": () => alterType(mood).renameValue("calm", "serene"),
  "alter-type-rename": () => alterType(mood).renameTo("feeling"),
  "alter-composite": () =>
    alterType("pair")
      .addAttribute("d", "bigint")
      .addAttribute("e", "text", { collation: "C" })
      .dropAttribute("a", { ifExists: true })
      .dropAttribute("b")
      .alterAttribute("c", "text", { collation: "C" })
      .alterAttribute("d", "integer")
      .behavior("cascade"),
  "rename-attribute": () => alterType("pair").renameAttribute("a", "z").behavior("restrict"),
  "drop-types": () => dropType([mood, "pair"], { ifExists: true, behavior: "cascade" }),
  "create-sequence": () =>
    createSequence(new Table("ticket", { schema: "app" }))
      .ifNotExists()
      .asType("integer")
      .options({ incrementBy: 5, minValue: null, maxValue: 9_007_199_254_740_991n, startWith: -3, cache: 2 })
      .options({ cycle: true, minValue: 1 })
      .ownedBy(booking, "id"),
  "alter-sequence": () =>
    alterSequence("ticket").restart(100).ifExists().asType("bigint").options({ cycle: false }).ownedBy(null),
  "alter-sequence-restart": () => alterSequence("ticket").restart(),
  "drop-sequences": () => dropSequence(["ticket", new Table("other", { schema: "app" })], { behavior: "cascade" }),
  "rename-sequence": () => renameSequence(new Table("ticket", { schema: "app" }), "voucher"),
  "create-extension": () =>
    createExtension("btree_gist", { ifNotExists: true, schema: "public", version: "1.7", cascade: true }),
  "drop-extension": () => dropExtension("btree_gist", { ifExists: true, behavior: "restrict" }),
  "create-schema": () => createSchema("app"),
  "create-schema-if-not-exists": () => createSchema('App "x"', { ifNotExists: true }),
  "drop-schema": () => dropSchema("app", { behavior: "restrict" }),
  "drop-schemas": () => dropSchema(["app", "archive"], { ifExists: true, behavior: "cascade" }),
  "rename-schema": () => renameSchema("app", 'App "x"'),
  "create-domain": () =>
    createDomain(quantity, "integer")
      .check(domainValue().gt(0), { name: "positive" })
      .notNull()
      .default(1)
      .check(domainValue().lt(1000)),
  "create-domain-collated": () =>
    createDomain('Code "x"', "text").collate("C", { schema: "pg_catalog" }).null().default("none"),
  "create-domain-over-types": () => createDomain("tags", new DataType("varchar", { length: 20 }).array()),
  "create-domain-over-domain": () => createDomain("small", quantity).check(domainValue().between(0, 10)),
  "alter-domain-set-default": () => alterDomain(quantity).setDefault(5),
  "alter-domain-drop-default": () => alterDomain(quantity).dropDefault(),
  "alter-domain-set-not-null": () => alterDomain(quantity).setNotNull(),
  "alter-domain-drop-not-null": () => alterDomain(quantity).dropNotNull(),
  "alter-domain-add-check": () => alterDomain(quantity).addCheck(domainValue().ne(13)),
  "alter-domain-add-check-not-valid": () =>
    alterDomain(quantity).addCheck(domainValue().lt(100), { name: 'Small "x"', notValid: true }),
  "alter-domain-drop-constraint": () => alterDomain(quantity).dropConstraint("small", { ifExists: true, behavior: "cascade" }),
  "alter-domain-rename-constraint": () => alterDomain(quantity).renameConstraint("small", "tiny"),
  "alter-domain-validate-constraint": () => alterDomain(quantity).validateConstraint("small"),
  "alter-domain-rename": () => alterDomain(quantity).renameTo("units"),
  "drop-domain": () => dropDomain("quantity"),
  "drop-domains": () => dropDomain(["quantity", quantity], { ifExists: true, behavior: "restrict" }),
  "create-view": () => createView(app("large_room"), largeRooms()),
  "create-view-options": () =>
    createView(app("large_room"), largeRooms())
      .orReplace()
      .columns(["booking"])
      .columns(["room_no"])
      .checkOption("local")
      .checkOption("cascaded")
      .securityBarrier()
      .securityInvoker(),
  "create-recursive-view": () => createRecursiveView("counter", ["n"], counting()).orReplace().securityBarrier(),
  "drop-views": () => dropView(["large_room", app("other")], { ifExists: true, behavior: "cascade" }),
  "create-materialized-view": () =>
    createMaterializedView(app("room_total"), largeRooms()).ifNotExists().columns(["booking", "room_no"]).withNoData(),
  "refresh-materialized-view": () => refreshMaterializedView(app("room_total")),
  "refresh-materialized-view-concurrently": () => refreshMaterializedView(app("room_total"), { concurrently: true }),
  "refresh-materialized-view-no-data": () => refreshMaterializedView(app("room_total"), { withNoData: true }),
  "drop-materialized-views": () => dropMaterializedView([app("room_total"), "other"], { behavior: "restrict" }),
  "create-table-as": () => createTableAs("large_copy", largeRooms()).ifNotExists().columns(["booking"]).withNoData(),
  "comment-on-table": () => commentOnTable(booking, "Rooms booked, and when"),
  "comment-on-column": () => commentOnColumn(booking, "note", "it's a \\ note\nover two lines"),
};
