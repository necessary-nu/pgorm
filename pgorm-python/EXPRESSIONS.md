# Expressions

Python expressions own `pgorm_query::SimpleExpr` values. Every operation clones
its inputs and composes another Rust expression. An expression can be reused
across queries and Python threads in the supported interpreter. Composing or
inspecting it does not connect to a database or invoke a compiler.

```python
from pgorm import Condition, TypeName, Value, bind, call, col, literal

name = col("name", table="account", schema="application")
predicate = Condition.all(
    col("active", table="account") == True,
    Condition.any(name.starts_with("A%"), name.is_null()),
)
computed = call("lower", name)
mixed_values = bind(Value(7, "i16")) + literal(3)

assert mixed_values.inspect().sql == "SELECT $1 + 3"
assert mixed_values.inspect().params == [Value(7, "i16")]
```

`Expr.inspect()` places the expression in a Rust `Query::select().expr(...)`
and calls `build()`. `Condition.inspect()` uses `SELECT TRUE` with the condition
in `cond_where`. Both return a `Compiled` object with `sql` and `params`.
`params` are detached `Value` wrappers of the Rust builder's ordered wire
parameters. Their `snapshot()` method preserves the native value tags.
Qualified enum identity lives in the expression's Rust cast and appears in
the SQL; its wire parameter retains the underlying Rust string/array tag.

## Input paths

| Python | Corresponding Rust boundary |
| --- | --- |
| `col(name, table=..., schema=...)` | `Expr::col` with `ColumnRef` qualification |
| `bind(value)` | `Expr::value`; a separately collected parameter |
| `literal(value)` | `SimpleExpr::Constant`; rendered by Rust as a literal |
| `TypeName(name, schema=...)` | identifier-only Rust `TypeName` |
| `expr.cast(type_name, array=False)` | `SimpleExpr::cast_as_type` |
| `expr.collate(name, schema=...)` | `Expr::collate`; a validated `Name`, schema-qualified when `schema` is given |
| `array(select)` | `Expr::array`; `ARRAY(select)`, the subquery's one column as one array value |
| `exists(select)` | `Expr::exists`; `EXISTS (select)` |
| `scalar(select)` | `SimpleExpr::SubQuery`; `(select)`, the subquery's one value |
| `any_(source)` / `all_(source)` | `Expr::any` / `Expr::all` over a `Select`, `Func::any` / `Func::all` over an array |
| `case_when(condition, result)` | `Expr::case`; a searched `CASE`, `SearchedCase.when` adding arms through `CaseStatement::case` |
| `case_of(operand).when(value, result)` | `Expr::case_of` and `CaseOperand::when`; a simple `CASE`, `SimpleCase.when` adding arms |
| `expr.at(index)` | `Expr::index`; `a[index]` |
| `expr.slice(lower, upper)` | `Expr::subscript(Subscript::Slice)`; `a[lower:upper]`, `None` an open end |
| `LikeExpr(pattern).escape(c)` / `.no_escape()` | `LikeExpr::new`, `LikeExpr::escape` and `LikeExpr::no_escape` |
| `LikeExpr.literal(text)`, `.containing(text)`, `.starting_with(text)`, `.ending_with(text)` | `LikeExpr::literal`, `containing`, `starting_with` and `ending_with`: literal text, its `%`, `_` and escape character escaped |
| `expr.as_(name)` | owned projection expression plus validated `Name` |
| `expr.asc()` / `expr.desc()` | owned expression plus Rust `Order` |
| `OrderBy(expr, Direction.Asc, nulls=Nulls.First)` | typed direction and `NullOrdering` |

Column, table, schema and alias names are strings, each minted as Rust's
`Name::runtime` mints a name computed at run time, where the method reads it.
Each part is 1–63 UTF-8 bytes without NUL. Dots are part of an identifier, not
qualification syntax: `col("a.b")` is one column name; use `table="a"` to
qualify column `b`. Type names are separate objects. Case, quotes, percent
signs, backslashes and Unicode reach the Rust builders unchanged.

Ordinary operands are converted through `Value` and bound: a scalar, a `list`
as an array of the one kind its items infer as, and a `dict` as JSON. Use
`literal(...)`, the one way to write a value into the SQL text, to retain a
literal node within an otherwise parameterized expression. `bind` on an enum value carries its qualified Rust cast; enum
arrays carry the corresponding array cast. Explicit casts keep Rust's
source-typed parameter pins, such as `$1::text`, where the renderer uses them.

`expr.collate("C")` writes `("expr" COLLATE "C")`, and `schema="pg_catalog"`
qualifies the name, both identifiers as `ColumnDef.collate` reads them, quoted
so `C` keeps its case. The result is still an expression, so it is compared
(`col("name").collate("C") < "b"`) or ordered: `col("name").collate("C").asc()`
writes the clause into the sort key, which is where PostgreSQL reads an
`ORDER BY`'s collation. An empty or overlong name raises `ConstructionError`;
a collation the server does not have, or one on a type without collations,
is its `DatabaseError` (`42704`, `42804`).

The statement builder's ordinary parameters have no lifetime brand and are
safe to reuse across independent statement builds: each Rust build starts
its own parameter collection. This does not expose or extend the borrowed
`pipeline::Binder` lifetime. Pipeline scopes are a separate API.

## Operators and conditions

Use `==`, `!=`, `<`, `<=`, `>`, `>=`, or the methods `eq`, `ne`, `lt`, `lte`,
`gt`, `gte`. Arithmetic uses `+`, `-`, `*`, `/` and `%` with the expression on
the left. These lower to the corresponding Rust `BinOper` through
`SimpleExpr::binary`. For a scalar on the left, write `bind(scalar)` or
`literal(scalar)` explicitly.

Combine expressions with `&`, `|` and `~`. Parenthesize comparisons because
Python's operator precedence still applies. SQL expressions, conditions,
projection aliases, orderings, `format_json`'s `JsonInput`, `window_function`'s
`WindowFunction`, a windowed call, a `LikeExpr`, `any_` and `all_`'s
`Quantified`, `case_of`'s `CaseOperand`, `Direction`, `Nulls` and
the SQL/JSON kinds, behaviours, defaults and `JSON_TABLE` columns raise
`ConstructionError` on Python truth testing. Python `and`, `or`, `not`, chained comparisons and implicit truth
tests cannot compose SQL.

`Condition.all(*terms)` and `Condition.any(*terms)` invoke Rust's `Condition`
constructors, accepting expressions and nested conditions. `condition.add`
returns a new condition. Empty All is true; empty Any is false. Grouping,
flattening and negation follow the Rust condition implementation.

`is_null`, `is_not_null`, `between`, `not_between`, `is_in` and `is_not_in`
invoke the corresponding Rust `Expr` methods. Membership accepts a list or
tuple of values/expressions, including an empty list, or a `Select`, whose
rows it tests through Rust's `in_subquery` / `not_in_subquery`, the subquery
checked as any embedded one is. The empty-list
implementation remains Rust's constant comparison, which `build()` renders
as `$1 = $2` with `("a", "b")` for IN and `("a", "a")` for NOT IN.
Use `is_null()` explicitly; an equality against a typed NULL retains ordinary
SQL equality semantics. `tuple_expr(*items)` constructs a nonempty Rust tuple.
`array(select)` collects a `Select`'s one column into one array value — empty,
not NULL, when it yields no row — which casts and compares as a value where the
subquery's rows cannot: `array(names).cast(TypeName("text"), array=True)`. The
subquery is checked as any embedded one is, and PostgreSQL refuses one of more
than one column (`42601`).

`exists(select)` is `EXISTS (select)`, whether the subquery yields a row, and
`scalar(select)` is `(select)`, its one row's one value, NULL for no row; the
server refuses a scalar subquery of more than one column (`42601`) and, as it
runs, one yielding more than one row (`21000`). Both read the subquery as any
embedded one is read.

`any_(source)` and `all_(source)` are `ANY (..)` and `ALL (..)`, named with a
trailing underscore so they do not shadow Python's builtins. Each is the right
operand of every operator PostgreSQL reads a quantifier after, and of nothing
else: the comparisons — `==`, `!=`, `<`, `<=`, `>`, `>=` or their methods —
`like`, `not_like`, `ilike` and `not_ilike`, the regular-expression matches,
`contains`, `contained_by` and `overlaps`, the range positions, `matches` and
`path_exists`. `similar_to`, `is_distinct_from`, `between`, arithmetic and
`concat`, which the grammar or the server takes no quantifier after, a
membership list and `bind` refuse it with `ConstructionError`, and it has no
`inspect` of its own. The source is a `Select`, whose rows it ranges over
(Rust's `Expr::any` / `Expr::all`), or an array (Rust's `Func::any` /
`Func::all`): an expression, such as an array column or `array(select)`, or a
value bound as one array parameter, which names its kind as an array always
does:

```python
from pgorm import Value, all_, any_, array, col, select, Table

ids = Value.array("i64", [1, 3])
col("id") == any_(ids)                 # "id" = ANY($1), one parameter however long
col("id") != all_(ids)                 # "id" <> ALL($1)
col("id") > all_(select(col("id")).from_(Table("t")))   # > ALL (SELECT ..)
col("id") == any_(array(select(col("id")).from_(Table("t"))))
```

`= ANY` over one array parameter is the membership `IN` tests, with one
placeholder for any length, so the statement's text, and the prepared
statement a connection caches for it, is the same at every length. Over an
empty array, or a subquery with no row, `ANY` is false and `ALL` true.

A quantified LIKE pattern carries no escape, since PostgreSQL reads no
`ESCAPE` after `ANY (..)`: `col("name").like(any_(["a%", "b%"]))` is
`"name" LIKE ANY($1)`. The server types a bound array by the operator it
follows, so an element type the operator does not settle needs a cast: a
range's `@>` reads an untyped array as ranges, so elements are
`bind(Value.array("i32", [3, 11])).cast(TypeName("int4"), array=True)`, and
`path_exists` and `matches` over a `jsonb` read one as `jsonpath[]`, which a
bound list of `str` cannot be sent as (`ConstructionError` as it runs), so it
is `bind(["$.a", "$.b"]).cast(TypeName("jsonpath"), array=True)`.

`expr.at(index)` is one array element, `a[index]`, which counts from 1 and is
NULL outside the array's bounds; `expr.slice(lower, upper)` is the slice
`a[lower:upper]`, both bounds inclusive, `None` leaving an end open, so
`slice(2)` is `a[2:]`, `slice(upper=3)` `a[:3]` and `slice()` `a[:]`. Each
bound is bound unless it is an expression. `Expr` has no `__getitem__`: one
would make every expression look iterable to Python.

PostgreSQL's other binary operators are methods, each the Rust builder of its
meaning, the other operand bound unless it is an expression — the same set
JavaScript's expressions have:

| Method | Operator | Reads |
| --- | --- | --- |
| `is_distinct_from(other)`, `is_not_distinct_from(other)` | `IS [NOT] DISTINCT FROM` | any two values, NULL as a value |
| `concat(other)` | `\|\|` | text, arrays and `jsonb` |
| `regex(pattern)`, `not_regex(pattern)` | `~`, `!~` | text against a POSIX regular expression |
| `iregex(pattern)`, `not_iregex(pattern)` | `~*`, `!~*` | the same with case ignored |
| `contains(other)`, `contained_by(other)`, `overlaps(other)` | `@>`, `<@`, `&&` | ranges, multiranges and arrays |
| `strictly_left_of(other)`, `strictly_right_of(other)` | `<<`, `>>` | ranges and multiranges |
| `not_extend_right_of(other)`, `not_extend_left_of(other)` | `&<`, `&>` | ranges and multiranges |
| `adjacent_to(other)` | `-\|-` | ranges and multiranges |
| `matches(other)` | `@@` | full-text search, or a `jsonb` against a `jsonpath` predicate |
| `path_exists(path)` | `@?` | a `jsonb` against a `jsonpath` |

`between` and `not_between` take `symmetric=True` for `BETWEEN SYMMETRIC`,
which sorts the bounds first. A range or multirange operand binds as a
`Value` naming its kind, `Value(Range(1, 5), "int4range")`, as a range always
does. `path_exists` takes a `str`, bound as text cast to `jsonpath` (Rust's
`json_path_exists`), so the path never becomes statement text; anything else
is a `ConstructionError`. Unlike `json_exists`, `@?` is served by a GIN index
on the column, and a strict-mode path that would raise answers NULL.
`matches` binds a `str` as text, which reads it as full-text search; a
`jsonpath` predicate is written cast,
`bind("$.a == 1").cast(TypeName("jsonpath"))`.

`Expr`'s stub declares its methods in one stub-only mixin per family
(`_expr_comparison.pyi`, `_expr_patterns.pyi` and the rest), which exist for
the type checker alone: at runtime `Expr` has no base but `object`, so
`help(Expr)` and `dir(Expr)` list every method on the class itself.

## CASE

`case_when(condition, result)` is a searched `CASE` with its first arm, Rust's
`Expr::case`, and `case_of(operand)` begins a simple `CASE`, Rust's
`Expr::case_of`, which compares its operand with each arm's value by `=`:

```python
from pgorm import Condition, case_of, case_when, col

band = (case_when(col("score") > 10, "high")
        .when(Condition.any(col("score").is_null(), col("score") < 0), "none")
        .else_("low"))
assert band.inspect().sql == (
    'SELECT (CASE WHEN ("score" > $1) THEN $2 '
    'WHEN ("score" IS NULL OR "score" < $3) THEN $4 ELSE $5 END)'
)
calm = case_of(col("mood")).when("calm", 1).when("glad", 2).else_(0)
```

A `SearchedCase` and a `SimpleCase` are each an `Expr`, so a CASE goes
wherever an expression goes — a projection, `WHERE`, `ORDER BY`, a written
value — and is compared, combined and cast as one. Each takes further `when`
arms, tried in order, and one `else_`, after which it is a plain `Expr` that
takes neither. A searched arm's condition is an expression or a `Condition`;
every value and result is bound unless it is an expression. `case_of` gives a
`CaseOperand`, which is no expression until its first `when`, as PostgreSQL's
grammar requires a `WHEN`: given where an expression goes, it is refused as
any other non-expression is. Without an `else_`, a row no arm matches is
NULL, and so is a simple CASE whose operand is NULL, which equals no value.
PostgreSQL types a CASE from its results, and reads one whose results are all
bound values, which carry no type to it, as text: give one a type, a column,
a `literal` or `bind(1).cast(TypeName("int4"))`, where the CASE is not text.

## Text and functions

`starts_with`, `ends_with` and `contains_text` treat their arguments as text,
including `%`, `_`, backslashes and quotes. They lower to these Rust builders:

| Helper | Rust composition |
| --- | --- |
| `starts_with(text)` | `Func::starts_with(expression, text)` |
| `contains_text(text)` | `Func::named("strpos").args(...) > 0` |
| `ends_with(text)` | `Func::named("right").args(expression, Func::char_length(text)) == text` |

There is no second escaping implementation in the bindings. These helpers
use PostgreSQL string functions and ordinary Rust value expressions.

For pattern semantics, use `expr.like(pattern)`, `not_like`, `ilike` or
`not_ilike`. The pattern is a string, bound with the default backslash
escape, or a `LikeExpr`, Rust's `LikeExpr`: `LikeExpr(pattern).escape("!")`
escapes with `!` instead, `ESCAPE '!'`, and `LikeExpr(pattern).no_escape()`
turns escaping off, `ESCAPE ''`, so a backslash in the pattern is itself.
`LikeExpr.containing(text)` builds the pattern from literal text instead, a
user's search among it: its `%`, `_` and escape character are escaped, so
`col.like(p.LikeExpr.containing("50%"))` matches the text `50%` and not
`500`; `literal`, `starting_with` and `ending_with` are the exact, `text%`
and `%text` shapes.
`similar_to` and `not_similar_to` take the same pattern for `SIMILAR TO`,
a whole-string match in SQL's regular-expression dialect (LIKE's `%` and `_`
with `|`, `*`, `+`, `?`, `{m,n}`, grouping and bracket classes); its default
escape is a backslash.

`call(name, *arguments)` invokes the named Rust `Func` constructor for:
`lower`, `upper`, `abs`, `char_length`, `count`, `count_distinct`, `sum`, `avg`,
`min`, `max`, `round`, `coalesce`, `random`, `gen_random_uuid`, and PostgreSQL
18's `uuidv4`, `uuidv7`, `uuid_extract_timestamp` and `uuid_extract_version`,
the set-returning `generate_series` and `unnest` a `TableFunction` reads, and
the record-returning `json_to_record`, `jsonb_to_record`, `json_to_recordset`
and `jsonb_to_recordset` it reads through a column definition list
(see [STATEMENTS.md](STATEMENTS.md#table-functions-and-samples)).
`round` accepts one or two arguments, `coalesce` one or more, `random`,
`gen_random_uuid` and `uuidv4` none, `uuidv7` none or one (the `interval` its
embedded time is shifted by, Rust's `Func::uuidv7_shifted`), `generate_series`
two or three (`Func::generate_series`, and `generate_series_step` with its
step), and the others one. PostgreSQL overloads `generate_series` and `unnest`
by argument type, so a bound argument carries its own, as Rust's does:
`call("generate_series", 1, 3)` is `GENERATE_SERIES($1::int4, $2::int4)`,
an integer that fits 32 bits typed as its literal would be, and a cast,
`bind(1).cast(TypeName("int8"))`, chooses another overload. A call is an expression wherever one is taken, so
`ColumnDef("id", "uuid").default(call("uuidv7"))` gives a table a time-ordered
key. Unsupported names or argument
counts raise `UnsupportedCapabilityError`. The capability manifest records
these signatures in `expression_functions`. This name selects a supported
function constructor; it is not raw SQL.

The installed module's capability manifest maps each logical operation
to its Rust API. Arithmetic names such as `expr.add` refer to Python operators
(`Expr.__add__`). Projection/ordering objects retain their state until a
statement consumes a clone. They cannot accidentally become predicates.

## SQL/JSON

PostgreSQL's SQL/JSON functions, constructors and `IS JSON` are functions
returning an `Expr`. Each clause is a keyword argument applied through the
Rust builder's own method, so the SQL is `pgorm_query`'s:

```python
from pgorm import (JsonDefault, JsonQueryBehavior, JsonValueBehavior, col,
                   format_json, json_exists, json_object, json_query, json_value)

doc = col("doc")
size = json_value(doc, "$.size", returning="integer",
                  on_empty=JsonDefault(0), on_error=JsonValueBehavior.Error)
assert size.inspect().sql == (
    'SELECT JSON_VALUE("doc", CAST($1::text AS jsonpath) RETURNING integer '
    "DEFAULT 0 ON EMPTY ERROR ON ERROR)"
)
tagged = json_exists(doc, "$.tags[*] ? (@ == $Tag)", passing={"Tag": "blue"})
tags = json_query(doc, "$.tags[*]", shaping="with_wrapper",
                  on_empty=JsonQueryBehavior.EmptyArray)
row = json_object({"id": col("id"), "body": format_json(col("body"))},
                  absent_on_null=True, returning="jsonb")
```

| Python | Rust builder |
| --- | --- |
| `json_exists(context, path, *, passing, on_error)` | `Func::json_exists` |
| `json_value(context, path, *, passing, returning, on_empty, on_error)` | `Func::json_value` |
| `json_query(context, path, *, passing, returning, shaping, on_empty, on_error)` | `Func::json_query` |
| `json_object(entries, *, absent_on_null, unique_keys, returning)` | `Func::json_object` and `entry` |
| `json_array(*elements, null_on_null, returning)` | `Func::json_array` and `element` |
| `json_array_query(select, *, returning)` | `Func::json_array_query` |
| `json_objectagg(key, value, *, absent_on_null, unique_keys, returning, filter)` | `Func::json_objectagg` |
| `json_arrayagg(value, *, order_by, null_on_null, returning, filter)` | `Func::json_arrayagg` |
| `json_parse(input, *, unique_keys)` | `Func::json` (`JSON(..)`) |
| `json_scalar(operand)` | `Func::json_scalar` |
| `json_serialize(input, *, returning)` | `Func::json_serialize` |
| `format_json(operand)` | `Expr::format_json`, a `JsonInput` |
| `is_json(operand, kind=JsonKind.Value, *, unique_keys)` / `is_not_json` | `Expr::is_json` / `is_not_json` |

The path is a `str` that Rust binds as `text` cast to `jsonpath`, so it never
becomes statement text. `PASSING` takes a `dict` from variable name to value.
Each name is an identifier, quoted, so `$Tag` in the path names `"Tag"` exactly.
Values in positions that give them no type (`PASSING`, constructor members,
aggregate operands, `json_scalar`, `IS JSON`, the context) carry their type in
both render paths: `json_scalar(5)` is the JSON number 5, not the string `"5"`.
`format_json` marks an operand as JSON text. It returns a `JsonInput`, accepted
only in the positions SQL/JSON reads JSON, never as an expression.

Each function takes its own behaviour class, so a choice PostgreSQL refuses
cannot be written. `JsonExistsBehavior` is `True_`, `False_` (with the
underscore Python's keywords need), `Unknown` or `Error`. `JsonValueBehavior`
is `Null` or `Error`, and `JsonQueryBehavior` adds `EmptyArray` and
`EmptyObject`. `JsonDefault(value)` is `DEFAULT value` for either. PostgreSQL
refuses a parameter there (`42804`), so Rust writes the value as an escaped
literal in both render paths. A value carrying an enum cast is refused, because
the literal cannot keep the cast.

`json_query`'s `shaping` is one slot: `"with_wrapper"`,
`"with_conditional_wrapper"` or `"omit_quotes"`, because PostgreSQL refuses
`OMIT QUOTES` beside a wrapper. `returning` takes a `DataType` or a built-in
type name. `json_value` refuses `json` and `jsonb` with `ConstructionError`,
since PostgreSQL 18.6 returns `NULL` for every later row once one of those
evaluations is `NULL` (bug #19695); `json_query` reads JSON out of a
document instead. `json_object` takes a `dict` of string keys, bound as
values, or a list of `(key, value)` pairs whose keys may be expressions, so a
repeated key can still be tested against `unique_keys`. `json_arrayagg`'s
`order_by` is a list of `Expr.asc()` / `desc()`. The Rust builder places no
`NULLS FIRST` or `NULLS LAST` there, so an ordering that asks for one is
refused rather than dropped. `filter` takes an expression or a `Condition`.

`JSON(..)` is `json_parse` so that importing it cannot shadow Python's `json`
module. `json_serialize` reads its input through `JSON(..)`, so a `jsonb`
value serializes as its document on PostgreSQL 18.6. Both aggregates also run
as window functions, `json_arrayagg(..).over(window)` (see
[STATEMENTS.md](STATEMENTS.md#window-functions)).

### JSON_TABLE

`json_table` reads rows out of a JSON document. It is a `FROM` item, not an
expression: it returns a `FromItem`, which `Select.from_`, `join` and
`cross_join` take beside a `Table` (see [STATEMENTS.md](STATEMENTS.md)).

```python
from pgorm import Join, JsonQueryBehavior, JsonTableColumn as C, Table, json_table, literal, select

docs = Table("docs", alias="d")
items = json_table(
    docs.col("doc"), "$.items[*] ? (@.n >= $Min)",
    C.ordinality("i"),
    C.value("n", "integer"),
    C.query("tags", "jsonb", on_empty=JsonQueryBehavior.EmptyArray),
    C.exists("flagged", "boolean", path="$.flag"),
    C.nested("$.parts[*]", C.value("part", "text", path="$")),
    alias="jt", passing={"Min": 1},
)
query = select(docs.col("id"), items.col("n"), items.col("part")).from_(docs).from_(items)
labels = json_table(docs.col("doc"), "$.items[*]", C.value("label", "text"), alias="l")
kept = select(docs.col("id"), labels.col("label")).from_(docs).join(labels, literal(True), kind=Join.Left)
```

| Python | Rust builder |
| --- | --- |
| `json_table(context, path, column, *columns, alias, passing, path_name, on_error)` | `Func::json_table`, `column`, `passing`, `path_name`, `on_error`, `alias` |
| `JsonTableColumn.ordinality(name)` | `JsonTableColumn::ordinality` |
| `JsonTableColumn.value(name, kind, *, path, on_empty, on_error)` | `JsonTableColumn::value` |
| `JsonTableColumn.query(name, kind, *, path, shaping, on_empty, on_error)` | `JsonTableColumn::query` |
| `JsonTableColumn.exists(name, kind, *, path, on_error)` | `JsonTableColumn::exists` |
| `JsonTableColumn.nested(path, column, *columns, path_name)` | `JsonTableColumn::nested`, `column`, `path_name` |

The first column and the alias are required arguments, so a table with no
column (`42601` from the server) or no name cannot be built. Like a function in
`FROM`, it is implicitly `LATERAL`: its context may read the columns of the
items before it, after a comma or in a join, and `LEFT JOIN .. ON TRUE` keeps a
row whose document yields nothing. A nested path's rows join their parent's as
an outer join would. `items.col(name)` qualifies a column by the alias, and
`items.star()` selects them all.

Unlike the query functions' paths, these paths are literals: PostgreSQL
refuses a parameter for the root path (`0A000`) and its grammar takes only a
string constant for a column's or a nested path (`42601`). Rust writes each as
an escaped literal, never interpolated; a value the path needs goes through
`passing`, which binds it. Column names, path names, `PASSING` names and the
alias are identifiers, quoted case-exactly. A column without `path` reads
`$."name"`, its own name exactly as written.

Each column kind takes only its own behaviours, as the query function it
reads like does: `value` takes `JsonValueBehavior` or `JsonDefault`, `query`
takes `JsonQueryBehavior` or `JsonDefault` and the same `shaping` slot as
`json_query`, and `exists` takes `JsonExistsBehavior` for `on_error`, with no
`on_empty` because finding nothing is `false`. The table's own `on_error` is a
`JsonTableBehavior`, `Error` or `Empty`, the only two PostgreSQL admits there;
without one, a failing root path yields no rows. The server still refuses
what the builder does not track: a name used twice across the table and its
nested levels (`42712`), and `FORMAT JSON` on a `query` column whose type is
not a string, `json`, `jsonb` or `bytea` (`0A000`).

Run installed expression tests and Rust parity tests from the repository root:

```sh
target/python-check/bin/python -m unittest discover \
  -s pgorm-python/tests -p 'test_expressions.py' -v
PYO3_PYTHON="$PWD/target/python-dev/bin/python" cargo test \
  --manifest-path pgorm-python/Cargo.toml --target-dir target --locked --lib
```
