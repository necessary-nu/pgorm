# The query builder

pgorm-query, pgorm's SQL builder, builds statements as values. A statement is
a tree of tables, columns and expressions, which renders to PostgreSQL text
with every value bound as a parameter. In Python the builder is in `pgorm`
itself: `Table`, `select`, `insert`, `update`, `delete` and `merge` start
statements, and `col`, `bind`, `literal` and `call` start expressions. Each
holds the Rust builder it names, so the SQL a Python statement renders is
the SQL the Rust builder renders.

[Writing rows](04-writing.md) and [Reading rows](05-reading.md) cover reading
and writing one model's rows. Reach for the query builder when a statement is
about something else: a report joining several tables into columns of your
choosing, an `UPDATE` that reads another table, a `MERGE`, a `WITH` query, a
table function, or a table no model describes. It builds `SELECT`, `INSERT`,
`UPDATE`, `DELETE` and `MERGE`, and `pgorm.schema` builds the DDL statements
of [Schema generation](14-schema.md).

The builder refuses what PostgreSQL would refuse wherever it can say so: a
`MERGE` with no `WHEN` clause cannot be inspected or run, an `UPDATE` has no
`limit`, and an `INSERT` row of the wrong length is an error as it is added.

## A first statement

`select(*items)` starts a `SELECT` of the items given, or of `*` with none.
Each builder method returns a new statement and leaves the one it was called
on as it was, so a statement can be kept and refined in more than one
direction:

```python
from pgorm import RawSQL, Table, select

cake, bakery = Table("cake"), Table("bakery")

gluten_free = (
    select(cake.col("name"), bakery.col("city"))
    .from_(cake)
    .join(bakery, cake.col("bakery_id") == bakery.col("id"))
    .where(cake.col("gluten_free").eq(True))
)
query = gluten_free.order_by(cake.col("name").asc())

compiled = query.inspect()
assert compiled.sql == (
    'SELECT "cake"."name", "bakery"."city" FROM "cake" '
    'INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" '
    'WHERE "cake"."gluten_free" = $1 ORDER BY "cake"."name" ASC'
)
assert [value.value for value in compiled.params] == [True]

# The statement the order was added to is unchanged.
assert "ORDER BY" not in gluten_free.inspect().sql

assert RawSQL(compiled.sql, compiled.params).inline_sql() == (
    'SELECT "cake"."name", "bakery"."city" FROM "cake" '
    'INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" '
    'WHERE "cake"."gluten_free" = TRUE ORDER BY "cake"."name" ASC'
)
```

Here Python differs from Rust. Rust's builder methods change the statement
in place and the chain ends with `to_owned()`; Python's return a new one.
Rust renders a statement two ways, `build`, with `$1`, `$2`, … and the values
beside the SQL, and `to_string`, with each value written in as an escaped
literal. Python's `inspect()` is `build`: it returns a `Compiled`, whose
`sql` is the text and `params` the values, each a `Value` with its kind, and
that is what runs. A statement has no `to_string`; passing its SQL and values
to `RawSQL` and calling `inline_sql()` writes the values in, as above. Keep
that for tests and logs: it is a different statement text for each value.

`Table("cake")` names the table `"cake"`, and `cake.col("name")` its column,
`"cake"."name"`. `Table` also takes a `schema` and an `alias`, and
`table.as_(alias)` is the same table under an alias, which its `col` then
qualifies by; `table.star()` is `"cake".*`. A model is a table too:
`Cake.table` is its `Table` and `Cake.col("name")` its column, whose
operators convert each value through the field's declared kind. The
statements in this chapter name the bakery's tables with `Table`.

A `SELECT` takes the clauses SQL has, each a method:

- `select(*items)` on a statement replaces its projection. An item is an
  expression, or `expr.as_(name)` for one under a name.
- `from_(item)` adds a source, and a second is comma-joined. `join(item, on,
  kind=..)` joins one, `kind` being `Join.Inner` (the default), `Join.Left`,
  `Join.Right` or `Join.Full`, and `cross_join(item)` takes no condition.
  `join(item, on, lateral=True)` reads a subquery `LATERAL`, so it may read
  the columns of the sources before it.
- `where(condition)` and `having(condition)` may be called more than once;
  the conditions are joined with `AND`.
- `group_by(*items)`, `order_by(*orderings)` with `expr.asc()` and
  `expr.desc()`, which take `nulls=Nulls.First` or `Nulls.Last`, or
  `OrderBy(expr, Direction.Asc, nulls=..)` for a direction chosen at run
  time, and `distinct()`.
- `limit(n)` and `offset(n)` take a count from 0 to the largest `bigint`, and
  `None` removes the clause. A `bool` or a `float` raises `TypeError`, and a
  negative count `ConstructionError`. `limit_with_ties(n)` is `FETCH FIRST n
  ROWS WITH TIES`: the first `n` rows and every row tied with the last of
  them on the `ORDER BY`. It shares `limit`'s place, so each replaces the
  other, and a statement holding it with no `ORDER BY` raises
  `ConstructionError` when it is inspected or run.

`inspect()` checks the whole statement, so a refusal surfaces there, before
anything is sent, and so does a statement of more than 65,535 parameters,
PostgreSQL's limit. A statement, an expression and every part they are built
from raise `ConstructionError` when Python tests them for truth, so
`if query:` fails where it is written rather than reading the statement as
true.

## Running a statement

A statement runs through the methods of [Raw SQL](10-raw-sql.md): `query`
for all of its rows, `one` and `optional` for one, `execute` for the number
of rows it changed, and `stream` to read them as they arrive. A `SELECT`'s
rows, and a write's with a `RETURNING` clause, come back as records:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool, Table, col, delete, select, update

cake, bakery = Table("cake"), Table("bakery")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        query = (
            select(cake.col("name"), bakery.col("city"))
            .from_(cake)
            .join(bakery, cake.col("bakery_id") == bakery.col("id"))
            .where(cake.col("gluten_free").eq(True))
            .order_by(cake.col("name").asc())
        )
        rows = await pool.query(query)
        assert [(row["name"], row["city"]) for row in rows] == [
            ("Chocolate Torte", "Oslo"),
            ("Lemon Drizzle", "Oslo"),
            ("Vanilla Sponge", "Bergen"),
        ]

        raise_prices = update(cake).set("price", col("price") + Decimal("0.50")).where(col("bakery_id") == 2)
        assert raise_prices.inspect().sql == 'UPDATE "cake" SET "price" = "price" + $1::numeric WHERE "bakery_id" = $2'
        assert await pool.execute(raise_prices) == 2

        deleted = await pool.query(delete(cake).where(col("id") == 5).returning(col("name")))
        assert [row["name"] for row in deleted] == ["Chocolate Torte"]


asyncio.run(main())
```

Here Python differs from Rust, whose `SELECT` decodes through `into_tuple`,
`into_model` and `into_values`, and whose writes run through `execute_raw`
with each value wrapped. Python runs every statement as it is, and its rows
are records, read by column name. `ModelRecord` reads one as a model's record
([Raw SQL](10-raw-sql.md)).

## Where a bound value's type comes from

Each value a statement binds is a parameter, and PostgreSQL decides its type
from where it stands. Beside a column it takes the column's type, which is
the common case: `"price" + $1` reads `$1` as a `numeric`, and the value is
converted to it as it is bound. Where nothing around it says, the server
cannot choose: a value in a `SELECT` list or a `VALUES` row, the argument of
a function that takes any array such as `unnest`, a number added to a date.
There, cast it with `cast`, which also tells the server the type the value is
sent as: `bind(1).cast(TypeName("integer"))` is `CAST($1::int8 AS integer)`.
The examples under [WITH](#with), [Table functions](#table-functions) and
[Ranges](#ranges) do.

A plain Python value is bound with the kind its type gives it: an `int` is an
`i64`, PostgreSQL's `bigint`, a `str` is `text`, a `dict` is `json`.
`Value(7, "i16")` names another kind, and [Values and types](13-values.md)
lists them all.
`literal(value)` is the one way to write a value into the SQL text instead of
binding it; reach for it for a constant the statement's text should hold:

```python
from pgorm import TypeName, Value, bind, col, literal

expression = col("price") + 1
assert expression.inspect().sql == 'SELECT "price" + $1::int4'
assert [value.kind for value in expression.inspect().params] == ["i64"]

typed = bind(1).cast(TypeName("integer"))
assert typed.inspect().sql == "SELECT CAST($1::int8 AS integer)"

small = bind(Value(7, "i16")) + literal(3)
assert small.inspect().sql == "SELECT $1::int2 + 3"
assert [value.kind for value in small.inspect().params] == ["i16"]
```

An expression's `inspect()` renders it as `SELECT expression`, and a
condition's as `SELECT TRUE WHERE condition`, which is how the examples in
this chapter show a piece of a statement on its own.

## Names

A table, a column, an alias and a type are *names*. Here Python differs from
Rust. Rust takes a name only as a name, never as a `&str`, so a string where
a column belongs does not compile. In Python a name is a `str`, always, and
it is never SQL: each is checked as it is given, 1 to 63 bytes of UTF-8 with
no NUL (`ConstructionError` otherwise), and rendered quoted, exactly as
written, case and all. Text from a user that reaches a name is a name and
nothing more, and a value is never a name: values are bound. A type is a
`TypeName`, which takes a `schema` too.

The four places Rust takes a name from are one in Python. A table no model
describes is `Table("price_list")` and its columns `col("new_price")`; an
alias a statement introduces is `.as_("total")`; a name computed while the
program runs is any string, such as a table chosen by configuration. A dot is
part of a name, not a qualifier: `col("a.b")` is one column, and
`col("b", table="a", schema="s")` is `"s"."a"."b"`. Because every name is
quoted, a reserved word needs no care:

```python
from pgorm import ConstructionError, Table, call, col, select

price_list = Table("price_list")
query = select(call("sum", col("new_price")).as_("total")).from_(price_list)
assert query.inspect().sql == 'SELECT SUM("new_price") AS "total" FROM "price_list"'

archive = Table(f"price_list_{2025}")
query = select(col("name")).from_(archive)
assert query.inspect().sql == 'SELECT "name" FROM "price_list_2025"'

order = Table("order")
assert select(order.col("id")).from_(order).inspect().sql == 'SELECT "order"."id" FROM "order"'
assert col("a.b").inspect().sql == 'SELECT "a.b"'

try:
    col("")
except ConstructionError:
    pass
else:
    raise AssertionError("an empty name is refused")
```

## Expressions

`col(name)` is a column, `bind(value)` a bound value and `literal(value)` a
value written into the SQL. Python's operators build the comparisons and the
arithmetic: `==`, `!=`, `<`, `<=`, `>` and `>=`, or the methods `eq`, `ne`,
`lt`, `lte`, `gt` and `gte`; and `+`, `-`, `*`, `/` and `%`, with the
expression on the left. A plain value on the right is bound. For a value on
the left, write `bind(value)` or `literal(value)`.

`&`, `|` and `~` are `AND`, `OR` and `NOT`. Python's precedence still holds,
so parenthesise each comparison: `(col("a") == 1) & (col("b") > 2)`. Python's
own `and`, `or`, `not` and chained comparisons ask an expression for its
truth, which raises `ConstructionError`, so they cannot build a wrong
condition by accident. `Condition.all(*terms)` and `Condition.any(*terms)`
group conditions under `AND` and `OR`, and `condition.add(term)` returns one
with another term; an empty `all` is true and an empty `any` false.

The predicates are methods: `is_null`, `is_not_null`, `between` and
`not_between` (both with `symmetric=True` for `BETWEEN SYMMETRIC`), `is_in`
and `is_not_in`, which take a list or a tuple, or a `Select` whose rows they
test, and `like`, `not_like`, `ilike` and `not_ilike`. Test for `NULL` with
`is_null()`. `None` has no kind to bind, so `col("notes") == None` raises
`ConstructionError`; a typed `NULL`, `Value.null("text")`, binds, but an
equality with `NULL` is never true.

`call(name, *arguments)` calls a function: the aggregates `count`,
`count_distinct`, `sum`, `avg`, `min` and `max`; `lower`, `upper`, `abs`,
`char_length`, `round` (with one or two arguments) and `coalesce` (one or
more); `random`, `gen_random_uuid`, and PostgreSQL 18's `uuidv4`, `uuidv7`,
`uuid_extract_timestamp` and `uuid_extract_version`; and the set-returning
and record-returning functions of [Table functions](#table-functions).
`pgorm.capabilities()["expression_functions"]` lists them with the argument
counts each takes. A name is chosen from that list, never written into the
SQL, and another name or count raises `UnsupportedCapabilityError`. Rust's
`Func::named`, which calls any function by name, and its full-text search
functions are not available in Python yet.

`case_when(condition, result)` starts a searched `CASE`, `.when(condition,
result)` adds an arm, and `.else_(result)` ends it:

```python
from decimal import Decimal

from pgorm import Condition, Table, call, case_when, col, select

query = (
    select(
        col("name"),
        call("lower", col("name")).as_("key"),
        case_when(col("price") >= 5, "premium")
        .when(col("price") >= 3, "standard")
        .else_("budget")
        .as_("tier"),
    )
    .from_(Table("cake"))
    .where(Condition.any(
        col("name").like("%Choc%"),
        Condition.all(col("gluten_free").eq(True), col("price").between(Decimal(2), Decimal(4))),
    ))
)

compiled = query.inspect()
assert compiled.sql == (
    'SELECT "name", LOWER("name") AS "key", '
    '(CASE WHEN ("price" >= $1) THEN $2 WHEN ("price" >= $3) THEN $4 ELSE $5 END) AS "tier" '
    'FROM "cake" '
    'WHERE "name" LIKE $6 OR ("gluten_free" = $7 AND "price" BETWEEN $8 AND $9)'
)
assert [value.value for value in compiled.params] == [
    5, "premium", 3, "standard", "budget", "%Choc%", True, Decimal(2), Decimal(4),
]
```

`case_of(operand)` writes the simple form of `CASE`, comparing one operand
with each arm's value: `case_of(col("flavor")).when("chocolate",
"rich").else_("light")`. A `CASE` is an expression wherever one goes, and
without `else_` a row no arm matches is `NULL`. PostgreSQL types a `CASE`
from its results, and reads one whose results are all bound values, which
carry no type to it, as `text`; give one result a type, a column, a `literal`
or a cast, where the `CASE` is not text.

A few expressions take a little more care:

- A `LIKE` pattern given as a string is bound, with the backslash escaping
  `%` and `_`. `LikeExpr(pattern).escape("!")` escapes with another
  character, `ESCAPE '!'`, and `LikeExpr(pattern).no_escape()` turns escaping
  off. A pattern holding text from a user must have that text's wildcards
  escaped first. `starts_with(text)`, `contains_text(text)` and
  `ends_with(text)` match text as it is, `%`, `_` and backslashes included,
  through PostgreSQL's string functions rather than a pattern. `similar_to`
  and `not_similar_to` are `SIMILAR TO`, and `regex`, `not_regex`, `iregex`
  and `not_iregex` are the POSIX matches `~`, `!~`, `~*` and `!~*`.
- `cast(TypeName(name), array=False)` writes `CAST(.. AS type)`, or
  `type[]` with `array=True`. Here Python differs from Rust, whose cast
  returns a finished expression with fewer operators than a column has: a
  Python cast is an `Expr` like any other, compared and built on directly.
- `collate("C")` puts text under a collation, `("name" COLLATE "C")`, which
  decides how it compares and sorts; `schema="pg_catalog"` qualifies it.
  Ordering by it, `col("name").collate("C").asc()`, is how an `ORDER BY`
  takes a collation.
- `at(index)` and `slice(lower, upper)` subscript an array, counting from 1;
  `None` leaves a slice's end open. An `Expr` has no `[]`, which would make
  every expression look iterable to Python.
- `is_in(select)` tests against a subquery's rows, `exists(select)` whether
  it has any, `scalar(select)` is its one value, and `array(select)` its one
  column as an array. `select.as_("sold")` reads a query as a table, a
  source `from_` and the joins take.
- `any_(source)` and `all_(source)` are `ANY (..)` and `ALL (..)` over a
  subquery or an array, the right operand of a comparison: `col("id") ==
  any_([1, 3])` is `"id" = ANY($1)`, one placeholder whatever the list's
  length, so the statement's text, and the statement its connection prepares,
  stay the same from one list to the next. They also follow `like` and its
  kin, the regular-expression matches, the range and array operators,
  `matches` and `path_exists`.
- `tuple_expr(*items)` is a row of expressions, `("a", "b")`.
- Rust's `Expr::template`, a piece of SQL with bound values for an operation
  the builder lacks, is not available in Python yet. Python has no way to put
  SQL text into a built statement: what the builder does not have is written
  as a whole [raw statement](10-raw-sql.md).

```python
import asyncio
import os

from pgorm import Pool, Table, TypeName, any_, call, col, select

cake, order_line = Table("cake"), Table("order_line")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        query = (
            select(col("name"))
            .from_(cake)
            .where(col("id").is_in(select(col("cake_id")).from_(order_line).where(col("quantity") > 1)))
            .where(col("price").cast(TypeName("integer")) < 10)
            .where(call("char_length", col("name")) < 40)
            .order_by(col("name").collate("C").asc())
        )
        assert query.inspect().sql == (
            'SELECT "name" FROM "cake" '
            'WHERE "id" IN (SELECT "cake_id" FROM "order_line" WHERE "quantity" > $1) '
            'AND CAST("price" AS integer) < $2 '
            'AND CHAR_LENGTH("name") < $3 '
            'ORDER BY ("name" COLLATE "C") ASC'
        )
        assert [row["name"] for row in await pool.query(query)] == ["Chocolate Fudge"]

        chosen = select(col("name")).from_(cake).where(col("id") == any_([1, 3])).order_by(col("id").asc())
        assert chosen.inspect().sql == 'SELECT "name" FROM "cake" WHERE "id" = ANY($1) ORDER BY "id" ASC'
        assert [row["name"] for row in await pool.query(chosen)] == ["Chocolate Fudge", "Carrot Cake"]


asyncio.run(main())
```

PostgreSQL's other binary operators are methods too, the value on the right
bound unless it is an expression:

| Method | Operator | Reads |
| --- | --- | --- |
| `is_distinct_from`, `is_not_distinct_from` | `IS [NOT] DISTINCT FROM` | any two values, `NULL` as a value |
| `concat` | `\|\|` | text, arrays and `jsonb` |
| `contains`, `contained_by`, `overlaps` | `@>`, `<@`, `&&` | ranges, multiranges, arrays and `jsonb` |
| `strictly_left_of`, `strictly_right_of` | `<<`, `>>` | ranges and multiranges |
| `not_extend_right_of`, `not_extend_left_of` | `&<`, `&>` | ranges and multiranges |
| `adjacent_to` | `-\|-` | ranges and multiranges |
| `matches` | `@@` | full-text search, or a `jsonb` against a `jsonpath` predicate |
| `path_exists` | `@?` | a `jsonb` against a `jsonpath` |

`matches` binds a `str` as `text`, which reads it as full-text search; a
`jsonpath` predicate is written cast,
`bind("$.a == 1").cast(TypeName("jsonpath"))`. `path_exists` takes a
`str`, bound as `text` cast to `jsonpath`, so a path never becomes statement
text.

## INSERT, ON CONFLICT and RETURNING

`insert(table)` names its table, `columns(*names)` its columns, once, and
each `values(*items)` call adds one row. A row whose length does not match
the columns raises `ConstructionError` as it is added; Rust's `values`
returns that error and `values_panic` panics, and Python has the one form.
`select(query)` inserts a query's rows instead, Rust's `select_from`, and
`default_values()` inserts one row of defaults. An insert with no rows is an
error when it is inspected. `overriding(Overriding.SystemValue)` writes a
value into an `ALWAYS` identity column, and `Overriding.UserValue` discards
one for the generated value.

`on_conflict` takes what to do when a row conflicts. `Conflict.on(*targets)`
names the columns, or index expressions such as `call("lower",
col("email"))`, that conflict, and `Conflict.on_constraint(name)` names the
constraint instead. A target then does nothing, `.do_nothing()`, or updates:
`.update(*columns)` takes those columns from the row that was refused,
`excluded`, and `.set(column, value)` sets one of its own. A sign-up form
that may name an existing customer:

```python
import asyncio
import os

from pgorm import Conflict, Insert, Pool, Table, col, insert

from bakery import Customer

customer = Table("customer")


def sign_up(name: str, email: str) -> Insert:
    return (
        insert(customer)
        .columns("name", "email")
        .values(name, email)
        .on_conflict(Conflict.on("email").update("name"))
        .returning(col("id"))
    )


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        assert sign_up("Kimberly", "kim@example.com").inspect().sql == (
            'INSERT INTO "customer" ("name", "email") VALUES ($1, $2) '
            'ON CONFLICT ("email") DO UPDATE SET "name" = "excluded"."name" RETURNING "id"'
        )

        # Kim's address is taken, so her row is updated and keeps its id.
        assert (await pool.one(sign_up("Kimberly", "kim@example.com")))["id"] == 1
        # A new address is a new customer. Its id is 4, not 3: the insert that
        # conflicted drew 3 from the id's sequence before it conflicted.
        assert (await pool.one(sign_up("Mo", "mo@example.com")))["id"] == 4

        kim = await Customer.find_by_key({"id": 1}).one(pool)
        assert kim["name"] == "Kimberly"


asyncio.run(main())
```

`Conflict.do_nothing()` skips a conflicting row whatever it conflicts on. A
target named by columns takes a `where(condition)` before its action, the
predicate of a partial unique index; one named by its constraint takes none.
An update takes a `where(condition)` after it, choosing which conflicting
rows to update, and `.update` and `.set` can be chained to set more columns.
`Conflict`, `ConflictTarget` and `ConflictUpdate` are made only by these
functions; the classes are there for annotations and `isinstance`, and
calling one raises `TypeError`.

`returning(*items)` reads the rows a write touched, and with no items is
`RETURNING *`. PostgreSQL 18 also lets a `RETURNING` list read the row as it
was before the write and as it is after it, which `ReturningRow.Old` and
`ReturningRow.New` name: `.col(name)` is one column of that version and
`.star()` all of them. An update's old row is the row before it; a deleted
row's new version, and an inserted row's old one, are `NULL` in every column.
A row an upsert updated has the existing row as its old version, so
`ReturningRow.Old.col("id").is_null()` tells the rows it inserted from the
ones it updated.

Here Python differs from Rust: a record maps column names to values, so a
result with two columns of the same name raises `DecodeError`, and
`RETURNING old."price", new."price"` needs each under an alias of its own:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Conflict, Insert, Pool, ReturningRow, Table, col, insert, update

cake, customer = Table("cake"), Table("customer")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        doubled = (
            update(cake)
            .set("price", col("price") * 2)
            .where(col("id") == 1)
            .returning(ReturningRow.Old.col("price").as_("was"), ReturningRow.New.col("price").as_("now"))
        )
        assert doubled.inspect().sql == (
            'UPDATE "cake" SET "price" = "price" * $1::int4 WHERE "id" = $2 '
            'RETURNING old."price" AS "was", new."price" AS "now"'
        )
        row = await pool.one(doubled)
        assert (row["was"], row["now"]) == (Decimal("4.50"), Decimal("9.00"))

        def upsert(name: str, email: str) -> Insert:
            return (
                insert(customer).columns("name", "email").values(name, email)
                .on_conflict(Conflict.on("email").update("name"))
                .returning(col("id"), ReturningRow.Old.col("id").is_null().as_("inserted"))
            )

        kim = await pool.one(upsert("Kimberly", "kim@example.com"))
        assert (kim["id"], kim["inserted"]) == (1, False)
        mo = await pool.one(upsert("Mo", "mo@example.com"))
        assert mo["inserted"] is True


asyncio.run(main())
```

`returning(..., old_as="before", new_as="after")` renames the two versions,
`RETURNING WITH (OLD AS "before", NEW AS "after")`, and the list then reads
them as tables, `col("price", table="before")`. Use it when the target, its
alias or another relation of the statement is called `old` or `new`, which
would otherwise take the keyword's place silently.

## UPDATE … FROM and DELETE … USING

`update(table).set(column, value)` assigns a column, named bare, and each
`set` adds one; an update with none, or one column set twice, is an error.
An `UPDATE` can read other tables with `from_(item)`, and a `DELETE` with
`using(item)`, each joined in the `WHERE` clause. Neither takes `ORDER BY` or
`LIMIT`, which PostgreSQL refuses on both; to write some rows and not others,
filter on a subquery.

Here Python differs from Rust: an `UPDATE` or a `DELETE` needs a `where(..)`
or an explicit `all_rows()` before it is inspected or run, and raises
`ConstructionError` without one. A condition can still be always true; the
check asks only that the program said which rows it meant. `all_rows()` does
not remove a condition already given.

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool, Table, delete, update

cake, bakery, order, order_line = Table("cake"), Table("bakery"), Table("order"), Table("order_line")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # Every cake sold in Bergen goes up by fifty cents.
        bergen = (
            update(cake)
            .set("price", cake.col("price") + Decimal("0.50"))
            .from_(bakery)
            .where(cake.col("bakery_id") == bakery.col("id"))
            .where(bakery.col("city") == "Bergen")
        )
        assert bergen.inspect().sql == (
            'UPDATE "cake" SET "price" = "cake"."price" + $1::numeric FROM "bakery" '
            'WHERE "cake"."bakery_id" = "bakery"."id" AND "bakery"."city" = $2'
        )
        assert await pool.execute(bergen) == 2

        # Lee's order lines go.
        lees = (
            delete(order_line)
            .using(order)
            .where(order_line.col("order_id") == order.col("id"))
            .where(order.col("customer_id") == 2)
        )
        assert lees.inspect().sql == (
            'DELETE FROM "order_line" USING "order" '
            'WHERE "order_line"."order_id" = "order"."id" AND "order"."customer_id" = $1'
        )
        assert await pool.execute(lees) == 1


asyncio.run(main())
```

## MERGE

`MERGE` writes a table from a second relation, row by row: each row of the
target the condition pairs with a source row is *matched*, each source row
with no target row is *not matched*, and each target row with no source row
is *not matched by source*. Each kind takes its own arms, each arm an action
and an optional condition of its own, and a row takes the first arm whose
condition holds. A matched row can be updated or deleted, an unmatched
source row inserted.

`merge(target, source, on)` takes the target, the source and the condition,
and returns a `PendingMerge`, which has no `inspect()` and cannot run: the
statement exists from its first arm on, so a `MERGE` with no arm cannot be
sent. `when_matched`, `when_not_matched` and `when_not_matched_by_source`
each add an arm for their kind of row: an action, and `condition=` for an
arm that applies only where it holds.

Here the nightly price list for Sunrise holds two rows, Chocolate Fudge at
4.80 and Lemon Drizzle at 3.20. A cake on the list whose price differs takes
the new price, and a Sunrise cake missing from the list is withdrawn:

```python
import asyncio
import os

from pgorm import MergeAction, Pool, RawSQL, Table, merge

cake, price_list = Table("cake"), Table("price_list")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(RawSQL(
            "CREATE TABLE price_list (name text PRIMARY KEY, new_price numeric(10, 2) NOT NULL)"
        ))
        await pool.execute(RawSQL(
            "INSERT INTO price_list VALUES ('Chocolate Fudge', 4.80), ('Lemon Drizzle', 3.20)"
        ))

        nightly = (
            merge(cake, price_list, cake.col("name") == price_list.col("name"))
            .when_matched(
                MergeAction.update("price", price_list.col("new_price")),
                condition=cake.col("price") != price_list.col("new_price"),
            )
            .when_not_matched_by_source(MergeAction.delete(), condition=cake.col("bakery_id") == 1)
            .returning_action()
            .returning(cake.col("name"))
        )
        assert nightly.inspect().sql == (
            'MERGE INTO "cake" USING "price_list" ON "cake"."name" = "price_list"."name" '
            'WHEN MATCHED AND "cake"."price" <> "price_list"."new_price" '
            'THEN UPDATE SET "price" = "price_list"."new_price" '
            'WHEN NOT MATCHED BY SOURCE AND "cake"."bakery_id" = $1 THEN DELETE '
            'RETURNING merge_action(), "cake"."name"'
        )

        changes = sorted((row["merge_action"], row["name"]) for row in await pool.query(nightly))
        assert changes == [("DELETE", "Chocolate Torte"), ("UPDATE", "Chocolate Fudge")]


asyncio.run(main())
```

Lemon Drizzle matched but its price had not changed, and no arm applied, so
it was left as it was. `returning_action()` adds `merge_action()`, the action
each row took, as the first column; it has no expression form, because
PostgreSQL reads it only in a `MERGE`'s `RETURNING` list. `returning` takes
`old_as` and `new_as` as the other writes do. A column both the target and
the source have must be qualified, and `*` there is the source's columns
followed by the target's. `only()` writes `ONLY` before the target, leaving
the tables that inherit from it alone.

Here Python differs from Rust in how an arm is spelled and checked. Rust's
`when_matched_and(condition, action)` is `when_matched(action,
condition=condition)`. An action is made by `MergeAction`'s functions: a
target row is updated, `MergeAction.update(column, value)`, deleted,
`MergeAction.delete()`, or left alone, `MergeAction.do_nothing()`; a source
row is inserted, `MergeAction.insert(column, value)` or
`MergeAction.insert_defaults()`, or skipped, `MergeAction.do_nothing()`.
`update` and `insert` take their first column as they are called, so neither
is ever empty, and `.set(column, value)` adds another. An insert's
`.overriding(Overriding.SystemValue)` writes a supplied value into an
`ALWAYS` identity column, and `Overriding.UserValue` discards one for the
generated value.
Rust refuses an action an arm cannot take, such as an insert for a matched
row, when the program compiles. Python's stubs give the two kinds of arm
different action types, so a type checker refuses the pairing, and when the
program runs it raises `ConstructionError`. The classes `MergeAction`,
`MergeUpdate` and `MergeInsert` are there for annotations and `isinstance`;
calling one raises `TypeError`.

Within a kind, the arms with a condition are tried in the order they were
added, and the kind's one arm without a condition, which PostgreSQL accepts
only last, renders after them whenever it was added; a second replaces the
first. A row `do_nothing()` left alone is not returned.

The source is a `Table` or what `from_` takes, a subquery from
`select.as_(alias)` among them. A query may instead be named in a `WITH`
clause the `MERGE` takes with `with_`, and read as `Table(name)`. A value in
the source is bound like any other, and since the source assigns it to no
column, it may need a cast to the type it stands for.

## WITH

`With(name, query)` names a query a statement reads like a table, and
`.cte(name, query)` adds another, which may read the ones before it; a
statement takes the clause with `with_`. `columns=[..]` names the query's
columns, and `materialized=True` or `False` writes `MATERIALIZED` or `NOT
MATERIALIZED`. The query can be a `SELECT`, or a write with a `RETURNING`
clause, which then runs once and hands its rows to the rest of the
statement; a `MERGE` can be one too, and so a `SELECT` can read the rows a
`MERGE` returns.

```python
import asyncio
import os

from pgorm import Pool, Table, With, call, col, select

cake, order_line, sold = Table("cake"), Table("order_line"), Table("sold")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        units = (
            select(col("cake_id"), call("sum", col("quantity")).as_("units"))
            .from_(order_line)
            .group_by(col("cake_id"))
        )
        query = (
            select(cake.col("name"), sold.col("units"))
            .from_(cake)
            .join(sold, cake.col("id") == sold.col("cake_id"))
            .with_(With("sold", units))
        )
        assert query.inspect().sql == (
            'WITH "sold" AS (SELECT "cake_id", SUM("quantity") AS "units" FROM "order_line" GROUP BY "cake_id") '
            'SELECT "cake"."name", "sold"."units" FROM "cake" '
            'INNER JOIN "sold" ON "cake"."id" = "sold"."cake_id"'
        )

        rows = await pool.query(query.order_by(cake.col("name").asc()))
        assert [(row["name"], row["units"]) for row in rows] == [
            ("Carrot Cake", 1), ("Chocolate Fudge", 2), ("Lemon Drizzle", 1),
        ]


asyncio.run(main())
```

`With.recursive(name, query)` holds one query that reads its own rows: a
starting query, `UNION ALL`, and a query over the rows so far, repeated until
it adds none. It takes no second query, and a `MERGE` takes no recursive
clause, both of which PostgreSQL refuses. The days from the first of
November to the fourth, each with the orders placed on it, the days without
an order included:

```python
import asyncio
import os
from datetime import date

from pgorm import Join, Pool, Table, TypeName, With, bind, call, col, select

days, order = Table("days"), Table("order")
day_type = TypeName("date")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        one = bind(1).cast(TypeName("integer"))
        calendar = select(bind(date(2026, 11, 1)).cast(day_type).as_("day")).union_all(
            select(col("day") + one)
            .from_(days)
            .where(col("day") < bind(date(2026, 11, 4)).cast(day_type))
        )
        query = (
            select(days.col("day"), call("count", order.col("id")).as_("orders"))
            .from_(days)
            .join(order, order.col("placed_at").cast(day_type) == days.col("day"), kind=Join.Left)
            .group_by(days.col("day"))
            .order_by(days.col("day").asc())
            .with_(With.recursive("days", calendar, columns=["day"]))
        )
        assert query.inspect().sql.startswith(
            'WITH RECURSIVE "days" ("day") AS (SELECT CAST($1::date AS date) AS "day" '
            'UNION ALL (SELECT "day" + CAST($2::int8 AS integer) FROM "days" WHERE "day" < CAST($3::date AS date)))'
        )

        rows = await pool.query(query)
        assert [(row["day"], row["orders"]) for row in rows] == [
            (date(2026, 11, 1), 0),
            (date(2026, 11, 2), 1),
            (date(2026, 11, 3), 1),
            (date(2026, 11, 4), 0),
        ]


asyncio.run(main())
```

The starting row's date and the `1` added to each day are cast because
nothing around them gives them a type: the first stands in a `SELECT` list,
and the second beside a date, which PostgreSQL can add an integer or an
interval to.

A recursive query also takes `SEARCH` and `CYCLE` clauses for walking graphs.
`search=Search(SearchOrder.Breadth, by, "ord")` or `SearchOrder.Depth` adds
a column ordering the rows breadth first or depth first, and
`cycle=Cycle(by, "looped", "path")` stops a walk that returns to a row it has
been through:

```python
from pgorm import Cycle, Search, SearchOrder, Table, TypeName, With, bind, col, select

r = Table("r")
one = bind(1).cast(TypeName("integer"))
counting = With.recursive(
    "r",
    select(one.as_("n")).union_all(select(col("n") + one).from_(r).where(col("n") < 5)),
    columns=["n"],
    search=Search(SearchOrder.Breadth, col("n"), "ord"),
    cycle=Cycle(col("n"), "looped", "path"),
)
query = select(col("n")).from_(r).order_by(col("ord").asc()).with_(counting)
assert query.inspect().sql.endswith(
    'SEARCH BREADTH FIRST BY "n" SET "ord" CYCLE "n" SET "looped" USING "path" '
    'SELECT "n" FROM "r" ORDER BY "ord" ASC'
)
```

## Window functions

A window function computes a value for each row from the rows around it.
`expr.over(window)` puts a function call over a window, and `.as_(name)`
names the column. A `Window()` partitions the rows with
`partition_by(*expressions)`, orders them within each partition with
`order_by(*orderings)`, and can narrow them to a frame with `frame(..)`:
`FrameType.Rows`, `Range` or `Groups` starts one, and its bounds follow. Each
cake beside the running total of its bakery's prices, cheapest first:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import FrameType, Pool, Table, Window, call, col, select


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        running = (
            Window()
            .partition_by(col("bakery_id"))
            .order_by(col("price").asc())
            .frame(FrameType.Rows.unbounded_preceding().and_current_row())
        )
        query = (
            select(col("name"), call("sum", col("price")).over(running).as_("running"))
            .from_(Table("cake"))
            .where(col("bakery_id") == 1)
            .order_by(col("price").asc())
        )
        assert query.inspect().sql == (
            'SELECT "name", SUM("price") OVER ( PARTITION BY "bakery_id" ORDER BY "price" ASC '
            'ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW ) AS "running" '
            'FROM "cake" WHERE "bakery_id" = $1 ORDER BY "price" ASC'
        )

        rows = await pool.query(query)
        assert [(row["name"], row["running"]) for row in rows] == [
            ("Lemon Drizzle", Decimal("3.20")),
            ("Chocolate Fudge", Decimal("7.70")),
            ("Chocolate Torte", Decimal("13.70")),
        ]


asyncio.run(main())
```

A window used by several columns can be named once in the statement's
`WINDOW` clause, with `.window(name, window)`, and each column refers to it
by name, `over("name")`. `ExtendedWindow(base)` builds on a named window,
taking its partitions and adding an order or a frame; it is an entry of the
clause or written inline in `over`. Each `.window` call adds an entry, and a
later one may extend an earlier one.

The functions PostgreSQL computes only over a window come from
`window_function(name, *arguments)`: `row_number`, `rank`, `dense_rank`,
`percent_rank` and `cume_dist` take no argument, `ntile`, `first_value` and
`last_value` one, `nth_value` two, and `lag` and `lead` one to three. Such a
function has `over` and nothing else, so it cannot stand without its window.
A count PostgreSQL types `integer`, such as `ntile`'s or `lag`'s offset, is
written with `literal(n)`, since a Python `int` is bound as a `bigint`. Each
cake's place by price within its bakery, beside its bakery's total:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import ExtendedWindow, Pool, Table, Window, call, col, select, window_function


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        query = (
            select(
                col("name"),
                call("sum", col("price")).over("by_bakery").as_("total"),
                window_function("rank").over(ExtendedWindow("by_bakery").order_by(col("price").desc())).as_("place"),
            )
            .from_(Table("cake"))
            .window("by_bakery", Window().partition_by(col("bakery_id")))
            .order_by(col("id").asc())
        )
        assert query.inspect().sql == (
            'SELECT "name", SUM("price") OVER "by_bakery" AS "total", '
            'rank() OVER ( "by_bakery" ORDER BY "price" DESC ) AS "place" '
            'FROM "cake" WINDOW "by_bakery" AS ( PARTITION BY "bakery_id" ) ORDER BY "id" ASC'
        )

        rows = await pool.query(query)
        assert [(row["name"], row["total"], row["place"]) for row in rows] == [
            ("Chocolate Fudge", Decimal("13.70"), 2),
            ("Lemon Drizzle", Decimal("13.70"), 3),
            ("Carrot Cake", Decimal("6.70"), 1),
            ("Vanilla Sponge", Decimal("6.70"), 2),
            ("Chocolate Torte", Decimal("13.70"), 1),
        ]


asyncio.run(main())
```

`over` follows a function call, from `call(..)`, `window_function(..)`,
`json_arrayagg(..)` or `json_objectagg(..)`; on anything else, a column or a
sum, it raises `ConstructionError`. What PostgreSQL refuses of a `WINDOW`
clause depends on its other entries, so a statement is checked as it is
inspected, run or read into another: a name declared twice, an extended
window whose base is no earlier entry, has a frame, or already orders its
rows when it adds an order, and an `over(name)` no entry declares each raise
`ConstructionError`.

A frame starts from its mode, whose four methods name the start:
`unbounded_preceding()`, `preceding(offset)`, `current_row()` and
`following(offset)`. Each start offers only the ends that may follow it:
after a preceding start `and_preceding`, `and_current_row`, `and_following`
and `and_unbounded_following`; after `current_row()` all but
`and_preceding`; after a following start only `and_following` and
`and_unbounded_following`. So `ROWS BETWEEN CURRENT ROW AND 1 PRECEDING`
cannot be written. A preceding or current-row start stands alone as a whole
frame; a following start does not, since PostgreSQL would read it as
running to the current row behind it, and `frame` refuses one with
`TypeError`. `exclude(FrameExclusion.CurrentRow)`, `Group`, `Ties`
or `NoOthers`, is a method of a frame. An offset is a value or an expression
reading no column: under `Rows` and `Groups` a count of rows or peer groups,
under `Range` a distance in the ordering column's own values, such as a
`Decimal` over a numeric column. The builder does not know the ordering
column's type, so an offset that does not pair with it is the server's to
refuse.

The [pipeline](12-pipeline.md) writes windows in fewer lines, when its shape
fits.

## Grouping sets

A `GROUP BY` list can group by several sets of columns in one query.
`GroupingElement` builds them: `rollup(a, b)` for subtotals down a
hierarchy, the sets `(a, b)`, `(a)` and `()`; `cube(a, b)` for every
combination; `sets(first, *rest)` for sets of your choosing, each an
expression or another element; `set(a, b)` for one set, `(a, b)`; and
`empty()` for `()`, the whole input as one group. `group_by` takes them
beside plain expressions. A row grouped by a set that leaves a column out
holds `NULL` there. The cakes counted by bakery and by whether they are
gluten-free, with a subtotal per bakery and a grand total:

```python
import asyncio
import os

from pgorm import GroupingElement, Pool, Table, call, col, select


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        query = (
            select(col("bakery_id"), col("gluten_free"), call("count", col("id")).as_("cakes"))
            .from_(Table("cake"))
            .group_by(GroupingElement.rollup(col("bakery_id"), col("gluten_free")))
            .order_by(col("bakery_id").asc(), col("gluten_free").asc())
        )
        assert query.inspect().sql == (
            'SELECT "bakery_id", "gluten_free", COUNT("id") AS "cakes" FROM "cake" '
            'GROUP BY ROLLUP ("bakery_id", "gluten_free") '
            'ORDER BY "bakery_id" ASC, "gluten_free" ASC'
        )

        rows = await pool.query(query)
        assert [(row["bakery_id"], row["gluten_free"], row["cakes"]) for row in rows] == [
            (1, False, 1),
            (1, True, 2),
            (1, None, 3),
            (2, False, 1),
            (2, True, 1),
            (2, None, 2),
            (None, None, 5),
        ]


asyncio.run(main())
```

Rust's `Func::grouping`, which tells a `NULL` a set left out from one in the
data, is not available in Python yet: `call("grouping", ..)` raises
`UnsupportedCapabilityError`.

The list groups by every combination of its items' sets, and two
combinations can name the same set: `ROLLUP (a, b), ROLLUP (a)` reaches `(a)`
three ways, grouping its rows three times. `group_by_distinct()` is `GROUP
BY DISTINCT`, which groups each set once.

## Table functions

A function that returns rows, such as `generate_series` or `unnest`, can be
read as a table. `TableFunction(call(..))` wraps the call, `columns(*names)`
names its columns, and `with_ordinality()` adds a last column numbering the
rows from 1; `alias(name)` finishes it as something `from_` and the joins
take. `call` makes the calls `generate_series` (with two or three arguments)
and `unnest` (one), and the record functions below; Rust's `Func::named`
reaches any other, which Python cannot yet. `.and_(call)` reads a second
call beside the first, `ROWS FROM (..)`, the shorter padded with `NULL`. A
function in `FROM` may read the columns of the sources before it.

Unnesting a list of ids with its ordinal reads rows back in the order the
program asked for them. `unnest` takes an array of any type, and PostgreSQL
overloads it by its argument's type, so the bound array is cast to say which:

```python
import asyncio
import os

from pgorm import Pool, Table, TableFunction, TypeName, bind, call, select

cake = Table("cake")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        wanted = (
            TableFunction(call("unnest", bind([4, 1, 3]).cast(TypeName("integer"), array=True)))
            .with_ordinality()
            .columns("id", "position")
            .alias("wanted")
        )
        query = (
            select(cake.col("name"))
            .from_(wanted)
            .join(cake, cake.col("id") == wanted.col("id"))
            .order_by(wanted.col("position").asc())
        )
        assert query.inspect().sql == (
            'SELECT "cake"."name" FROM UNNEST(CAST($1::int8[] AS integer[])) '
            'WITH ORDINALITY AS "wanted"("id", "position") '
            'INNER JOIN "cake" ON "cake"."id" = "wanted"."id" ORDER BY "wanted"."position" ASC'
        )

        rows = await pool.query(query)
        assert [row["name"] for row in rows] == ["Vanilla Sponge", "Chocolate Fudge", "Carrot Cake"]


asyncio.run(main())
```

A function returning `record`, `json_to_record`, `jsonb_to_record`,
`json_to_recordset` or `jsonb_to_recordset`, has no columns until
`column_definition(name, type)` defines them, one call each; the type is a
`DataType` or what makes one, `"integer"` or a `TypeName`. Which function
returns `record` is the server's to know, so a record function with no
definitions, or definitions on another function, is its refusal:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool, TableFunction, Value, call, select


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        line = (
            TableFunction(call("json_to_record", Value.json({"cake_id": 2, "price": "3.20"})))
            .column_definition("cake_id", "integer")
            .column_definition("price", "numeric")
            .alias("line")
        )
        query = select(line.col("cake_id"), line.col("price")).from_(line)
        assert query.inspect().sql == (
            'SELECT "line"."cake_id", "line"."price" FROM JSON_TO_RECORD($1) '
            'AS "line"("cake_id" integer, "price" decimal)'
        )
        row = await pool.one(query)
        assert (row["cake_id"], row["price"]) == (2, Decimal("3.20"))


asyncio.run(main())
```

### Samples

`table.tablesample(method, percentage, repeatable=seed)` reads a sample of a
table through `TABLESAMPLE`: `SampleMethod.Bernoulli` draws each row,
`SampleMethod.System` each page, with a probability of `percentage` out of
100, and `repeatable` seeds the draw, so the same seed over unchanged data
draws the same sample. Both numbers are bound. The sampled table is read by
its alias, or by its name when it has none:

```python
from pgorm import SampleMethod, Table, select

sample = Table("cake").tablesample(SampleMethod.Bernoulli, 10, repeatable=42)
query = select(sample.col("name")).from_(sample)

compiled = query.inspect()
assert compiled.sql == 'SELECT "cake"."name" FROM "cake" TABLESAMPLE BERNOULLI ($1) REPEATABLE ($2)'
assert [value.value for value in compiled.params] == [10.0, 42.0]
```

## Set operations

`union(other)`, `union_all(other)`, `intersect(other)`,
`intersect_all(other)`, `except_(other)` and `except_all(other)` combine the
rows of two queries. The forms without `ALL` return each distinct row once.
`union_all` keeps every row of both; `intersect_all` returns a row as many
times as the query returning it less often does; `except_all` returns a row
as many times as this query returns it beyond the times `other` does.
`except` is a Python keyword, hence `except_`.

```python
import asyncio
import os

from pgorm import Pool, Table, col, select

bakery, customer = Table("bakery"), Table("customer")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        everyone = (
            select(bakery.col("name")).from_(bakery)
            .union(select(customer.col("name")).from_(customer))
            .order_by(col("name").asc())
        )
        assert everyone.inspect().sql == (
            'SELECT "bakery"."name" FROM "bakery" '
            'UNION (SELECT "customer"."name" FROM "customer") ORDER BY "name" ASC'
        )
        rows = await pool.query(everyone)
        assert [row["name"] for row in rows] == ["Kim", "Lee", "Moonlight", "Sunrise"]


asyncio.run(main())
```

The other query is read in parenthesised, and may carry its own `ORDER BY`
and limit, which bound only its rows. This statement's `ORDER BY`, `LIMIT`
and `OFFSET` are written after the last query, so they order and bound the
combined rows whether they were set before the combination or after. An
order of the combined rows names a result column, `col("name")` by the first
query's output name; PostgreSQL refuses a column qualified by one of the
first query's tables, or any other expression. The statement's `WITH` clause
precedes the whole, so every query reads it, while its `WHERE`, `GROUP BY`,
`HAVING`, `DISTINCT` and `WINDOW` belong to the first query alone.

A chain combines in the order it reads: `a.union(b).intersect(c)` is `(a
UNION b) INTERSECT c`, with the parentheses written in, since PostgreSQL's
`INTERSECT` otherwise binds tighter than `UNION` and `EXCEPT`. To combine
with a combination, pass it as the other query: `a.union(b.intersect(c))`.

## SQL/JSON

PostgreSQL's SQL/JSON functions query a JSON document with a path:
`json_exists(context, path)` tests that the path finds something,
`json_value` reads one scalar, `json_query` reads JSON, and `json_table`
turns what a path finds into rows. The constructors `json_object`,
`json_array`, `json_array_query`, `json_objectagg` and `json_arrayagg` build
JSON; `json_parse(input)` is `JSON(..)`, so named that it does not shadow
Python's `json` module; `json_scalar` and `json_serialize` are the functions
of those names; and `is_json` and `is_not_json` test text. Each clause is a
keyword argument. A path is a `str`, bound as `text` cast to `jsonpath`,
except in `json_table`, whose paths the server takes only as literals; a
value the path needs is passed with `passing`, a `dict` from variable name to
value, and bound.

An order arriving as JSON, read as rows of cake and quantity:

```python
import asyncio
import os

from pgorm import JsonTableColumn, Pool, Value, json_table, select


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        incoming = {
            "customer": "kim@example.com",
            "lines": [{"cake": 2, "qty": 3}, {"cake": 4, "qty": 1}],
        }
        line = json_table(
            Value.json(incoming),
            "$.lines[*]",
            JsonTableColumn.value("cake_id", "integer", path="$.cake"),
            JsonTableColumn.value("quantity", "integer", path="$.qty"),
            alias="line",
        )
        query = select(line.star()).from_(line)
        assert query.inspect().sql == (
            """SELECT "line".* FROM JSON_TABLE($1::jsonb, '$.lines[*]' COLUMNS """
            """("cake_id" integer PATH '$.cake', "quantity" integer PATH '$.qty')) AS "line\""""
        )

        rows = await pool.query(query)
        assert [(row["cake_id"], row["quantity"]) for row in rows] == [(2, 3), (4, 1)]


asyncio.run(main())
```

`json_table` returns a source, as a table function does, and its first
column and its `alias` are required, so a table with neither cannot be
built. Its columns are `JsonTableColumn.ordinality(name)`, `value(name,
type)`, `query(name, type)`, `exists(name, type)` and `nested(path, *columns)`;
a column with no `path` reads `$."name"`, its own name. It is implicitly
`LATERAL`, so its context may read the columns of the sources before it,
and a left join `on literal(True)` keeps a row whose document yields nothing.

Each function takes its own behaviour class, so a choice PostgreSQL refuses
cannot be written: `JsonExistsBehavior` is `True_`, `False_`, `Unknown` or
`Error`; `JsonValueBehavior` is `Null` or `Error`; `JsonQueryBehavior` adds
`EmptyArray` and `EmptyObject`; `JsonDefault(value)` is `DEFAULT value` for
`on_empty` and `on_error`, written as a literal since the server takes no
parameter there; and `json_table`'s own `on_error` is a `JsonTableBehavior`,
`Error` or `Empty`. `json_query`'s `shaping` is one of `"with_wrapper"`,
`"with_conditional_wrapper"` or `"omit_quotes"`. `returning` takes a
`DataType` or a built-in type's name. `format_json(value)` marks an operand
as JSON text, where SQL/JSON reads JSON:

```python
from pgorm import JsonDefault, JsonValueBehavior, col, json_exists, json_object, json_value

size = json_value(col("doc"), "$.size", returning="integer",
                  on_empty=JsonDefault(0), on_error=JsonValueBehavior.Error)
assert size.inspect().sql == (
    'SELECT JSON_VALUE("doc", CAST($1::text AS jsonpath) RETURNING integer '
    "DEFAULT 0 ON EMPTY ERROR ON ERROR)"
)

tagged = json_exists(col("doc"), "$.tags[*] ? (@ == $Tag)", passing={"Tag": "blue"})
assert tagged.inspect().sql == (
    'SELECT JSON_EXISTS("doc", CAST($1::text AS jsonpath) PASSING $2::text AS "Tag")'
)

row = json_object({"id": col("id"), "name": col("name")}, returning="jsonb")
assert row.inspect().sql == 'SELECT JSON_OBJECT($1::text : "id", $2::text : "name" RETURNING jsonb)'
```

`json_value` refuses `returning="json"` and `"jsonb"` with
`ConstructionError`: PostgreSQL 18.6 returns `NULL` for every later row once
one such evaluation is `NULL`, so `json_query` reads JSON out of a document
instead. `json_arrayagg`'s `order_by` takes no `NULLS FIRST` or `NULLS
LAST`, which the builder would have to drop, so an order asking for one is
refused.

Of PostgreSQL's operators for `jsonb`, `contains` is `@>`, `path_exists`
`@?`, `matches` `@@` and `concat` `||`. Rust also has `get_json_field`,
`cast_json_field` and `has_json_key` for `->`, `->>` and `?`, which are not
available in Python yet; `json_value` and `json_query` read a field instead.

## Ranges

A range value is bound as one of PostgreSQL's range types:
`Value(Range(lower, upper), "tstzrange")` names its type, as a range always
does. The range operators are methods: `contains` (`@>`), `contained_by`
(`<@`, Rust's `contained`), `overlaps` (`&&`) and the positional operators in
the table under [Expressions](#expressions). Beside a range column a bound
range takes the column's type. Here the range stands on its own beside a
timestamp, which many operators accept, so it is cast to `tstzrange`. The
orders placed on the second of November:

```python
import asyncio
import os
from datetime import datetime, timezone

from pgorm import Pool, Range, Table, TypeName, Value, bind, col, select


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        start = datetime(2026, 11, 2, tzinfo=timezone.utc)
        end = datetime(2026, 11, 3, tzinfo=timezone.utc)
        that_day = bind(Value(Range(start, end), "tstzrange")).cast(TypeName("tstzrange"))
        query = select(col("id")).from_(Table("order")).where(that_day.contains(col("placed_at")))
        assert query.inspect().sql == (
            'SELECT "id" FROM "order" WHERE CAST($1::tstzrange AS tstzrange) @> "placed_at"'
        )

        rows = await pool.query(query)
        assert [row["id"] for row in rows] == [1]


asyncio.run(main())
```

[Values and types](13-values.md) covers ranges and multiranges as column
types, including range types a schema creates.

## Locking

A `SELECT` can lock the rows it reads with `lock(strength)`:
`LockStrength.Update` for `FOR UPDATE`, `NoKeyUpdate`, `Share` and
`KeyShare` for the others; `of=[..]` limits the lock to the sources listed,
each by the name it answers to in the statement; and `wait=LockWait.Nowait`
or `LockWait.SkipLocked` decides what happens at a row another transaction
holds. `SKIP LOCKED` is the usual way for several workers to take jobs from
one queue table. A later `lock` replaces the earlier one, and PostgreSQL
refuses `limit_with_ties` beside `SKIP LOCKED`, which `inspect()` and every
run refuse with `ConstructionError`. A lock lasts until the transaction ends,
so it belongs inside one ([Transactions](08-transactions.md)):

```python
import asyncio
import os

from pgorm import LockStrength, LockWait, Pool, Table, col, select


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        query = (
            select(col("id"))
            .from_(Table("order"))
            .where(col("bakery_id") == 1)
            .lock(LockStrength.Update, wait=LockWait.SkipLocked)
        )
        assert query.inspect().sql == (
            'SELECT "id" FROM "order" WHERE "bakery_id" = $1 FOR UPDATE SKIP LOCKED'
        )

        async with pool.transaction() as transaction:
            rows = await transaction.query(query)
            assert [row["id"] for row in rows] == [1]


asyncio.run(main())
```
