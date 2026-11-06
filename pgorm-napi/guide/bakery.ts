// The bakery the JavaScript guide's examples share: the models its entities
// chapter declares, their relations, and the tables and rows every example's
// scratch database starts with. The tables are the ones the Rust guide's
// pgorm-guide-support crate creates from its entities, so a statement an
// example shows reads the same rows in either language:
//
// | table | rows |
// | --- | --- |
// | `bakery` | 1 Sunrise (Oslo), 2 Moonlight (Bergen) |
// | `baker` | 1 Ada and 2 Bo at Sunrise, 3 Cy at Moonlight |
// | `cake` | 1 Chocolate Fudge, 2 Lemon Drizzle, 5 Chocolate Torte at Sunrise; 3 Carrot Cake, 4 Vanilla Sponge at Moonlight |
// | `cake_baker` | Ada bakes 1 and 2, Bo bakes 2 and 5, Cy bakes 3 and 4 |
// | `customer` | 1 Kim, 2 Lee |
// | `order` | 1 Kim at Sunrise, 2 Lee at Moonlight |
// | `order_line` | order 1: two Chocolate Fudge, one Lemon Drizzle; order 2: one Carrot Cake |

import { column, model, type Pool, TypeName } from "@necessary/pgorm";

/** The PostgreSQL enum a cake's flavor is. */
export const flavor = new TypeName("flavor", { schema: "public" });

export const Bakery = model("bakery", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    name: column("text"),
    city: column("text"),
  },
});

export const Baker = model("baker", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    name: column("text"),
    hiredOn: column("date", { name: "hired_on" }),
    bakeryId: column("i32", { name: "bakery_id" }),
  },
});

export const Cake = model("cake", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    name: column("text"),
    flavor: column(flavor, { values: ["chocolate", "vanilla", "lemon", "carrot"] }),
    price: column("decimal"),
    glutenFree: column("bool", { name: "gluten_free" }),
    bakeryId: column("i32", { name: "bakery_id" }),
  },
});

export const CakeBaker = model("cake_baker", {
  columns: {
    cakeId: column("i32", { name: "cake_id", primaryKey: true }),
    bakerId: column("i32", { name: "baker_id", primaryKey: true }),
  },
});

export const Customer = model("customer", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    name: column("text"),
    email: column("text"),
    notes: column("text", { nullable: true }),
  },
  uniqueKeys: [{ fields: "email" }],
});

export const Order = model("order", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    customerId: column("i32", { name: "customer_id" }),
    bakeryId: column("i32", { name: "bakery_id" }),
    placedAt: column("datetime_utc", { name: "placed_at" }),
  },
});

export const OrderLine = model("order_line", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    orderId: column("i32", { name: "order_id" }),
    cakeId: column("i32", { name: "cake_id" }),
    quantity: column("i32"),
    price: column("decimal"),
  },
});

// Each relation is named for the model it starts at and then what it reaches:
// `cakeBakery` is a cake's bakery, `bakeryCakes` a bakery's cakes.
export const bakerBakery = Baker.belongsTo(Bakery, { from: "bakeryId", to: "id" });
export const bakeryBakers = Bakery.hasMany(Baker, { from: "id", to: "bakeryId" });

export const cakeBakery = Cake.belongsTo(Bakery, { from: "bakeryId", to: "id" });
export const bakeryCakes = Bakery.hasMany(Cake, { from: "id", to: "bakeryId" });

export const cakeBakerCake = CakeBaker.belongsTo(Cake, { from: "cakeId", to: "id" });
export const cakeBakerBaker = CakeBaker.belongsTo(Baker, { from: "bakerId", to: "id" });
export const cakeCakeBakers = Cake.hasMany(CakeBaker, { from: "id", to: "cakeId" });
export const bakerCakeBakers = Baker.hasMany(CakeBaker, { from: "id", to: "bakerId" });

// A cake's bakers through the cake_baker junction, the path the Rust cake's
// `Related<baker::Entity>` declares.
export const cakeBakers = Cake.related(cakeBakerBaker, { via: cakeCakeBakers });

export const orderCustomer = Order.belongsTo(Customer, { from: "customerId", to: "id" });
export const orderBakery = Order.belongsTo(Bakery, { from: "bakeryId", to: "id" });
export const customerOrders = Customer.hasMany(Order, { from: "id", to: "customerId" });
export const bakeryOrders = Bakery.hasMany(Order, { from: "id", to: "bakeryId" });

export const orderLines = Order.hasMany(OrderLine, { from: "id", to: "orderId" });
export const lineOrder = OrderLine.belongsTo(Order, { from: "orderId", to: "id" });
export const lineCake = OrderLine.belongsTo(Cake, { from: "cakeId", to: "id" });
export const cakeLines = Cake.hasMany(OrderLine, { from: "id", to: "cakeId" });

/**
 * The bakery's schema, as pgorm's `Schema` writes it from the Rust guide's
 * entities: the enum, then each table after the tables its foreign keys
 * reference.
 */
const SCHEMA = [
  `CREATE TYPE "flavor" AS ENUM ('chocolate', 'vanilla', 'lemon', 'carrot')`,
  `CREATE TABLE "bakery" ( "id" serial NOT NULL, "name" varchar NOT NULL, "city" varchar NOT NULL, PRIMARY KEY ("id") )`,
  `CREATE TABLE "baker" ( "id" serial NOT NULL, "name" varchar NOT NULL, "hired_on" date NOT NULL, "bakery_id" integer NOT NULL, PRIMARY KEY ("id"), CONSTRAINT "fk-baker-bakery_id" FOREIGN KEY ("bakery_id") REFERENCES "bakery" ("id") )`,
  `CREATE TABLE "cake" ( "id" serial NOT NULL, "name" varchar NOT NULL, "flavor" flavor NOT NULL, "price" decimal(10, 2) NOT NULL, "gluten_free" bool NOT NULL, "bakery_id" integer NOT NULL, PRIMARY KEY ("id"), CONSTRAINT "fk-cake-bakery_id" FOREIGN KEY ("bakery_id") REFERENCES "bakery" ("id") )`,
  `CREATE TABLE "cake_baker" ( "cake_id" integer NOT NULL, "baker_id" integer NOT NULL, CONSTRAINT "pk-cake_baker" PRIMARY KEY ("cake_id", "baker_id"), CONSTRAINT "fk-cake_baker-cake_id" FOREIGN KEY ("cake_id") REFERENCES "cake" ("id") ON DELETE CASCADE, CONSTRAINT "fk-cake_baker-baker_id" FOREIGN KEY ("baker_id") REFERENCES "baker" ("id") ON DELETE CASCADE )`,
  `CREATE TABLE "customer" ( "id" serial NOT NULL, "name" varchar NOT NULL, "email" varchar NOT NULL, "notes" varchar, PRIMARY KEY ("id"), UNIQUE ("email") )`,
  `CREATE TABLE "order" ( "id" serial NOT NULL, "customer_id" integer NOT NULL, "bakery_id" integer NOT NULL, "placed_at" timestamp with time zone NOT NULL, PRIMARY KEY ("id"), CONSTRAINT "fk-order-customer_id" FOREIGN KEY ("customer_id") REFERENCES "customer" ("id"), CONSTRAINT "fk-order-bakery_id" FOREIGN KEY ("bakery_id") REFERENCES "bakery" ("id") )`,
  `CREATE TABLE "order_line" ( "id" serial NOT NULL, "order_id" integer NOT NULL, "cake_id" integer NOT NULL, "quantity" integer NOT NULL, "price" decimal(10, 2) NOT NULL, PRIMARY KEY ("id"), CONSTRAINT "fk-order_line-order_id" FOREIGN KEY ("order_id") REFERENCES "order" ("id") ON DELETE CASCADE, CONSTRAINT "fk-order_line-cake_id" FOREIGN KEY ("cake_id") REFERENCES "cake" ("id") )`,
];

/** The bakery's rows, their ids assigned by the database in this order. */
const ROWS = [
  `INSERT INTO "bakery" ("name", "city") VALUES ('Sunrise', 'Oslo'), ('Moonlight', 'Bergen')`,
  `INSERT INTO "baker" ("name", "hired_on", "bakery_id") VALUES ('Ada', '2019-03-01', 1), ('Bo', '2022-09-15', 1), ('Cy', '2021-05-10', 2)`,
  `INSERT INTO "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") VALUES ` +
  `('Chocolate Fudge', 'chocolate', 4.50, false, 1), ('Lemon Drizzle', 'lemon', 3.20, true, 1), ` +
  `('Carrot Cake', 'carrot', 3.80, false, 2), ('Vanilla Sponge', 'vanilla', 2.90, true, 2), ` +
  `('Chocolate Torte', 'chocolate', 6.00, true, 1)`,
  `INSERT INTO "cake_baker" ("cake_id", "baker_id") VALUES (1, 1), (2, 1), (2, 2), (5, 2), (3, 3), (4, 3)`,
  `INSERT INTO "customer" ("name", "email", "notes") VALUES ('Kim', 'kim@example.com', NULL), ('Lee', 'lee@example.com', 'Allergic to nuts')`,
  `INSERT INTO "order" ("customer_id", "bakery_id", "placed_at") VALUES (1, 1, '2026-11-02T09:30:00Z'), (2, 2, '2026-11-03T14:00:00Z')`,
  `INSERT INTO "order_line" ("order_id", "cake_id", "quantity", "price") VALUES (1, 1, 2, 4.50), (1, 2, 1, 3.20), (2, 3, 1, 3.80)`,
];

/** Create the bakery's tables and fill them, on the database `pool` reaches. */
export async function createBakery(pool: Pool): Promise<void> {
  for (const statement of [...SCHEMA, ...ROWS]) await pool.execute(statement);
}
