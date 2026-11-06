"""The bakery the pgorm Python guide's examples share.

The models below declare the bakery's seven tables, the relations between
them and the ``flavor`` enum, as the guide's Entities chapter describes.
``create`` makes the tables in an empty database and ``seed`` fills them with
the rows the guide's text describes, their ids assigned by the database in
this order:

- ``bakery``: 1 Sunrise (Oslo), 2 Moonlight (Bergen)
- ``baker``: 1 Ada and 2 Bo at Sunrise, 3 Cy at Moonlight
- ``cake``: 1 Chocolate Fudge, 2 Lemon Drizzle, 5 Chocolate Torte at Sunrise;
  3 Carrot Cake, 4 Vanilla Sponge at Moonlight
- ``cake_baker``: Ada bakes 1 and 2, Bo bakes 2 and 5, Cy bakes 3 and 4
- ``customer``: 1 Kim, 2 Lee
- ``order``: 1 Kim at Sunrise, 2 Lee at Moonlight
- ``order_line``: order 1, two Chocolate Fudge and one Lemon Drizzle;
  order 2, one Carrot Cake

The tables are the ones pgorm-guide-support creates from the Rust guide's
entities, so a Rust and a Python program read the same bakery.
"""

from datetime import date, datetime, timezone
from decimal import Decimal

from pgorm import Column, Connection, Model, Pool, Transaction, TypeName
from pgorm import schema as s

Database = Pool | Connection | Transaction

FLAVOR = TypeName("flavor", schema="public")
FLAVORS = ["chocolate", "vanilla", "lemon", "carrot"]

Bakery = Model("bakery", {
    "id": Column("i32", primary_key=True, default=True),
    "name": Column("text"),
    "city": Column("text"),
})

Baker = Model("baker", {
    "id": Column("i32", primary_key=True, default=True),
    "name": Column("text"),
    "hired_on": Column("date"),
    "bakery_id": Column("i32"),
})

Cake = Model("cake", {
    "id": Column("i32", primary_key=True, default=True),
    "name": Column("text"),
    "flavor": Column(FLAVOR, values=FLAVORS),
    "price": Column("decimal"),
    "gluten_free": Column("bool"),
    "bakery_id": Column("i32"),
})

CakeBaker = Model("cake_baker", {
    "cake_id": Column("i32", primary_key=True),
    "baker_id": Column("i32", primary_key=True),
})

Customer = Model("customer", {
    "id": Column("i32", primary_key=True, default=True),
    "name": Column("text"),
    "email": Column("text"),
    "notes": Column("text", nullable=True),
}, unique_keys=[{"fields": ("email",)}])

Order = Model("order", {
    "id": Column("i32", primary_key=True, default=True),
    "customer_id": Column("i32"),
    "bakery_id": Column("i32"),
    "placed_at": Column("datetime_utc"),
})

OrderLine = Model("order_line", {
    "id": Column("i32", primary_key=True, default=True),
    "order_id": Column("i32"),
    "cake_id": Column("i32"),
    "quantity": Column("i32"),
    "price": Column("decimal"),
})

# Each relation is named for where it starts and what it reaches.
bakery_bakers = Bakery.has_many(Baker, from_="id", to="bakery_id")
bakery_cakes = Bakery.has_many(Cake, from_="id", to="bakery_id")
bakery_orders = Bakery.has_many(Order, from_="id", to="bakery_id")
baker_bakery = Baker.belongs_to(Bakery, from_="bakery_id", to="id")
cake_bakery = Cake.belongs_to(Bakery, from_="bakery_id", to="id")
cake_order_lines = Cake.has_many(OrderLine, from_="id", to="cake_id")
cake_bakers = Cake.has_many(CakeBaker, from_="id", to="cake_id")
cake_baker_cake = CakeBaker.belongs_to(Cake, from_="cake_id", to="id")
cake_baker_baker = CakeBaker.belongs_to(Baker, from_="baker_id", to="id")
baker_cake_bakers = Baker.has_many(CakeBaker, from_="id", to="baker_id")
customer_orders = Customer.has_many(Order, from_="id", to="customer_id")
order_customer = Order.belongs_to(Customer, from_="customer_id", to="id")
order_bakery = Order.belongs_to(Bakery, from_="bakery_id", to="id")
order_lines = Order.has_many(OrderLine, from_="id", to="order_id")
order_line_order = OrderLine.belongs_to(Order, from_="order_id", to="id")
order_line_cake = OrderLine.belongs_to(Cake, from_="cake_id", to="id")

# A cake's bakers through the cake_baker junction, the path the Rust cake's
# Related<baker::Entity> declares.
cake_related_bakers = Cake.related(cake_baker_baker, via=cake_bakers)


def _key(name: str) -> s.ColumnDef:
    return s.ColumnDef(name, "integer").not_null()


def _id() -> s.ColumnDef:
    return s.ColumnDef("id", "integer").not_null().auto_increment()


def _text(name: str) -> s.ColumnDef:
    return s.ColumnDef(name, "varchar").not_null()


def _money(name: str) -> s.ColumnDef:
    return s.ColumnDef(name, s.DataType("numeric", precision=10, scale=2)).not_null()


def statements() -> list[s.DDL]:
    """The enum type and the tables, in the order they are created."""
    return [
        s.create_type("flavor").as_enum().values(FLAVORS),
        s.create_table("bakery").column(_id()).column(_text("name")).column(_text("city"))
        .primary_key("id"),
        s.create_table("baker").column(_id()).column(_text("name"))
        .column(s.ColumnDef("hired_on", "date").not_null()).column(_key("bakery_id"))
        .primary_key("id")
        .foreign_key("bakery_id", "bakery", "id", name="fk-baker-bakery_id"),
        s.create_table("cake").column(_id()).column(_text("name"))
        .column(s.ColumnDef("flavor", TypeName("flavor")).not_null()).column(_money("price"))
        .column(s.ColumnDef("gluten_free", "boolean").not_null()).column(_key("bakery_id"))
        .primary_key("id")
        .foreign_key("bakery_id", "bakery", "id", name="fk-cake-bakery_id"),
        s.create_table("cake_baker").column(_key("cake_id")).column(_key("baker_id"))
        .primary_key(["cake_id", "baker_id"], name="pk-cake_baker")
        .foreign_key("cake_id", "cake", "id", name="fk-cake_baker-cake_id", on_delete="cascade")
        .foreign_key("baker_id", "baker", "id", name="fk-cake_baker-baker_id", on_delete="cascade"),
        s.create_table("customer").column(_id()).column(_text("name")).column(_text("email"))
        .column(s.ColumnDef("notes", "text")).primary_key("id").unique("email"),
        s.create_table("order").column(_id()).column(_key("customer_id"))
        .column(_key("bakery_id")).column(s.ColumnDef("placed_at", "timestamptz").not_null())
        .primary_key("id")
        .foreign_key("customer_id", "customer", "id", name="fk-order-customer_id")
        .foreign_key("bakery_id", "bakery", "id", name="fk-order-bakery_id"),
        s.create_table("order_line").column(_id()).column(_key("order_id"))
        .column(_key("cake_id")).column(s.ColumnDef("quantity", "integer").not_null())
        .column(_money("price")).primary_key("id")
        .foreign_key("order_id", "order", "id", name="fk-order_line-order_id", on_delete="cascade")
        .foreign_key("cake_id", "cake", "id", name="fk-order_line-cake_id"),
    ]


async def create(db: Database) -> None:
    """Create the bakery's enum type and tables in an empty database."""
    for statement in statements():
        await db.execute(statement)


async def seed(db: Database) -> None:
    """Insert the bakery's rows."""
    await Bakery.insert_many([
        {"name": "Sunrise", "city": "Oslo"},
        {"name": "Moonlight", "city": "Bergen"},
    ]).execute(db)
    await Baker.insert_many([
        {"name": "Ada", "hired_on": date(2019, 3, 1), "bakery_id": 1},
        {"name": "Bo", "hired_on": date(2022, 9, 15), "bakery_id": 1},
        {"name": "Cy", "hired_on": date(2021, 5, 10), "bakery_id": 2},
    ]).execute(db)

    def cake(name: str, flavor: str, price: str, gluten_free: bool, bakery_id: int) -> dict[str, object]:
        return {"name": name, "flavor": flavor, "price": Decimal(price),
                "gluten_free": gluten_free, "bakery_id": bakery_id}

    await Cake.insert_many([
        cake("Chocolate Fudge", "chocolate", "4.50", False, 1),
        cake("Lemon Drizzle", "lemon", "3.20", True, 1),
        cake("Carrot Cake", "carrot", "3.80", False, 2),
        cake("Vanilla Sponge", "vanilla", "2.90", True, 2),
        cake("Chocolate Torte", "chocolate", "6.00", True, 1),
    ]).execute(db)
    await CakeBaker.insert_many([
        {"cake_id": cake_id, "baker_id": baker_id}
        for cake_id, baker_id in [(1, 1), (2, 1), (2, 2), (5, 2), (3, 3), (4, 3)]
    ]).execute(db)
    await Customer.insert_many([
        {"name": "Kim", "email": "kim@example.com", "notes": None},
        {"name": "Lee", "email": "lee@example.com", "notes": "Allergic to nuts"},
    ]).execute(db)
    await Order.insert_many([
        {"customer_id": 1, "bakery_id": 1,
         "placed_at": datetime(2026, 11, 2, 9, 30, tzinfo=timezone.utc)},
        {"customer_id": 2, "bakery_id": 2,
         "placed_at": datetime(2026, 11, 3, 14, 0, tzinfo=timezone.utc)},
    ]).execute(db)
    await OrderLine.insert_many([
        {"order_id": 1, "cake_id": 1, "quantity": 2, "price": Decimal("4.50")},
        {"order_id": 1, "cake_id": 2, "quantity": 1, "price": Decimal("3.20")},
        {"order_id": 2, "cake_id": 3, "quantity": 1, "price": Decimal("3.80")},
    ]).execute(db)
