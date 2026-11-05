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

import { column, DataType, fromModel, model, type Pool, TypeName } from "@necessary/pgorm";

/** The PostgreSQL enum a cake's flavor is. */
export const flavor = new TypeName("flavor", { schema: "public" });

/** The bakery's money: `decimal(10, 2)`, as the Rust entities' `column_type` says. */
const money = new DataType("numeric", { precision: 10, scale: 2 });

// Each field is named in camelCase and its column in snake_case, which
// `renameAll` writes for every field that names no column of its own.
export const Bakery = model("bakery", {
  renameAll: "snake_case",
  columns: {
    id: column("i32", { primaryKey: true, autoIncrement: true }),
    name: column("text", { columnType: "varchar" }),
    city: column("text", { columnType: "varchar" }),
  },
});

export const Baker = model("baker", {
  renameAll: "snake_case",
  columns: {
    id: column("i32", { primaryKey: true, autoIncrement: true }),
    name: column("text", { columnType: "varchar" }),
    hiredOn: column("date"),
    bakeryId: column("i32"),
  },
});

export const Cake = model("cake", {
  renameAll: "snake_case",
  columns: {
    id: column("i32", { primaryKey: true, autoIncrement: true }),
    name: column("text", { columnType: "varchar" }),
    flavor: column(flavor, { values: ["chocolate", "vanilla", "lemon", "carrot"] }),
    price: column("decimal", { columnType: money }),
    glutenFree: column("bool"),
    bakeryId: column("i32"),
  },
});

export const CakeBaker = model("cake_baker", {
  renameAll: "snake_case",
  columns: {
    cakeId: column("i32", { primaryKey: true }),
    bakerId: column("i32", { primaryKey: true }),
  },
});

export const Customer = model("customer", {
  renameAll: "snake_case",
  columns: {
    id: column("i32", { primaryKey: true, autoIncrement: true }),
    name: column("text", { columnType: "varchar" }),
    email: column("text", { columnType: "varchar" }),
    notes: column("text", { nullable: true, columnType: "varchar" }),
  },
  uniqueKeys: [{ fields: "email" }],
});

export const Order = model("order", {
  renameAll: "snake_case",
  columns: {
    id: column("i32", { primaryKey: true, autoIncrement: true }),
    customerId: column("i32"),
    bakeryId: column("i32"),
    placedAt: column("datetime_utc"),
  },
});

export const OrderLine = model("order_line", {
  renameAll: "snake_case",
  columns: {
    id: column("i32", { primaryKey: true, autoIncrement: true }),
    orderId: column("i32"),
    cakeId: column("i32"),
    quantity: column("i32"),
    price: column("decimal", { columnType: money }),
  },
});

// Each relation is named for the model it starts at and then what it reaches:
// `cakeBakery` is a cake's bakery, `bakeryCakes` a bakery's cakes. A
// `belongsTo` relation is a foreign key of the table it starts at.
export const bakerBakery = Baker.belongsTo(Bakery, { from: "bakeryId", to: "id" });
export const bakeryBakers = Bakery.hasMany(Baker, { from: "id", to: "bakeryId" });

export const cakeBakery = Cake.belongsTo(Bakery, { from: "bakeryId", to: "id" });
export const bakeryCakes = Bakery.hasMany(Cake, { from: "id", to: "bakeryId" });

export const cakeBakerCake = CakeBaker.belongsTo(Cake, { from: "cakeId", to: "id", onDelete: "cascade" });
export const cakeBakerBaker = CakeBaker.belongsTo(Baker, { from: "bakerId", to: "id", onDelete: "cascade" });
export const cakeCakeBakers = Cake.hasMany(CakeBaker, { from: "id", to: "cakeId" });
export const bakerCakeBakers = Baker.hasMany(CakeBaker, { from: "id", to: "bakerId" });

export const orderCustomer = Order.belongsTo(Customer, { from: "customerId", to: "id" });
export const orderBakery = Order.belongsTo(Bakery, { from: "bakeryId", to: "id" });
export const customerOrders = Customer.hasMany(Order, { from: "id", to: "customerId" });
export const bakeryOrders = Bakery.hasMany(Order, { from: "id", to: "bakeryId" });

export const orderLines = Order.hasMany(OrderLine, { from: "id", to: "orderId" });
export const lineOrder = OrderLine.belongsTo(Order, { from: "orderId", to: "id", onDelete: "cascade" });
export const lineCake = OrderLine.belongsTo(Cake, { from: "cakeId", to: "id" });
export const cakeLines = Cake.hasMany(OrderLine, { from: "id", to: "cakeId" });

/**
 * The bakery's schema, as pgorm's `Schema` writes it from the models: the
 * enum, then each table, with the foreign keys of its `belongsTo` relations,
 * after the tables those reference.
 */
function schema() {
  const cake = fromModel(Cake, { relations: [cakeBakery] });
  return [
    ...cake.enums,
    fromModel(Bakery).table,
    fromModel(Baker, { relations: [bakerBakery] }).table,
    cake.table,
    fromModel(CakeBaker, { relations: [cakeBakerCake, cakeBakerBaker] }).table,
    fromModel(Customer).table,
    fromModel(Order, { relations: [orderCustomer, orderBakery] }).table,
    fromModel(OrderLine, { relations: [lineOrder, lineCake] }).table,
  ];
}

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
  for (const statement of schema()) await pool.execute(statement);
  for (const statement of ROWS) await pool.execute(statement);
}
