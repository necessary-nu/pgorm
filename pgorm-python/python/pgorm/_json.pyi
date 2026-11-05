from typing import Any, ClassVar, Literal, Never, TypeAlias
from ._expressions import Expr, Condition
from ._query_types import OrderBy
from ._schema import BuiltinType, DataType
from ._statements import Select
from ._from_items import FromItem

JsonType: TypeAlias = DataType | BuiltinType
JsonValueType: TypeAlias = DataType | Literal["char", "varchar", "text", "smallint", "integer", "bigint", "real", "double", "numeric", "boolean", "date", "time", "timestamp", "timestamptz", "interval", "bytea", "bit", "varbit", "money", "uuid", "vector", "cidr", "inet", "macaddr", "ltree"]
JsonShaping: TypeAlias = Literal["with_wrapper", "with_conditional_wrapper", "omit_quotes"]
Predicate: TypeAlias = Expr | Condition

class JsonInput:
    def __init__(self, _native_only: Never, /) -> None: ...

class JsonKind:
    def __init__(self, _native_only: Never, /) -> None: ...
    Value: ClassVar[JsonKind]
    Scalar: ClassVar[JsonKind]
    Array: ClassVar[JsonKind]
    Object: ClassVar[JsonKind]

class JsonExistsBehavior:
    def __init__(self, _native_only: Never, /) -> None: ...
    True_: ClassVar[JsonExistsBehavior]
    False_: ClassVar[JsonExistsBehavior]
    Unknown: ClassVar[JsonExistsBehavior]
    Error: ClassVar[JsonExistsBehavior]

class JsonValueBehavior:
    def __init__(self, _native_only: Never, /) -> None: ...
    Null: ClassVar[JsonValueBehavior]
    Error: ClassVar[JsonValueBehavior]

class JsonQueryBehavior:
    def __init__(self, _native_only: Never, /) -> None: ...
    Null: ClassVar[JsonQueryBehavior]
    Error: ClassVar[JsonQueryBehavior]
    EmptyArray: ClassVar[JsonQueryBehavior]
    EmptyObject: ClassVar[JsonQueryBehavior]

class JsonDefault:
    def __init__(self, value: Any) -> None: ...

class JsonTableBehavior:
    def __init__(self, _native_only: Never, /) -> None: ...
    Error: ClassVar[JsonTableBehavior]
    Empty: ClassVar[JsonTableBehavior]

class JsonTableColumn:
    def __init__(self, _native_only: Never, /) -> None: ...
    @staticmethod
    def ordinality(name: str) -> JsonTableColumn: ...
    @staticmethod
    def value(name: str, kind: JsonType, *, path: str | None = ...,
              on_empty: JsonValueBehavior | JsonDefault | None = ...,
              on_error: JsonValueBehavior | JsonDefault | None = ...) -> JsonTableColumn: ...
    @staticmethod
    def query(name: str, kind: JsonType, *, path: str | None = ...,
              shaping: JsonShaping | None = ...,
              on_empty: JsonQueryBehavior | JsonDefault | None = ...,
              on_error: JsonQueryBehavior | JsonDefault | None = ...) -> JsonTableColumn: ...
    @staticmethod
    def exists(name: str, kind: JsonType, *, path: str | None = ...,
               on_error: JsonExistsBehavior | None = ...) -> JsonTableColumn: ...
    @staticmethod
    def nested(path: str, column: JsonTableColumn, *columns: JsonTableColumn,
               path_name: str | None = ...) -> JsonTableColumn: ...

def json_exists(context: Any, path: str, *, passing: dict[str, Any] | None = ...,
                on_error: JsonExistsBehavior | None = ...) -> Expr: ...
def json_value(context: Any, path: str, *, passing: dict[str, Any] | None = ...,
               returning: JsonValueType | None = ...,
               on_empty: JsonValueBehavior | JsonDefault | None = ...,
               on_error: JsonValueBehavior | JsonDefault | None = ...) -> Expr: ...
def json_query(context: Any, path: str, *, passing: dict[str, Any] | None = ...,
               returning: JsonType | None = ..., shaping: JsonShaping | None = ...,
               on_empty: JsonQueryBehavior | JsonDefault | None = ...,
               on_error: JsonQueryBehavior | JsonDefault | None = ...) -> Expr: ...
def json_table(context: Any, path: str, column: JsonTableColumn, *columns: JsonTableColumn,
               alias: str, passing: dict[str, Any] | None = ...,
               path_name: str | None = ...,
               on_error: JsonTableBehavior | None = ...) -> FromItem: ...
def json_object(entries: dict[str, Any] | list[tuple[Any, Any]] | tuple[tuple[Any, Any], ...] | None = ..., /, *,
                absent_on_null: bool = ..., unique_keys: bool = ...,
                returning: JsonType | None = ...) -> Expr: ...
def json_array(*elements: Any, null_on_null: bool = ..., returning: JsonType | None = ...) -> Expr: ...
def json_array_query(query: Select, *, returning: JsonType | None = ...) -> Expr: ...
def json_objectagg(key: Any, value: Any, *, absent_on_null: bool = ..., unique_keys: bool = ...,
                   returning: JsonType | None = ..., filter: Predicate | None = ...) -> Expr: ...
def json_arrayagg(value: Any, *, order_by: list[OrderBy] | tuple[OrderBy, ...] | None = ...,
                  null_on_null: bool = ..., returning: JsonType | None = ...,
                  filter: Predicate | None = ...) -> Expr: ...
def json_parse(input: Any, *, unique_keys: bool = ...) -> Expr: ...
def json_scalar(operand: Any) -> Expr: ...
def json_serialize(input: Any, *, returning: JsonType | None = ...) -> Expr: ...
def format_json(operand: Any) -> JsonInput: ...
def is_json(operand: Any, kind: JsonKind = ..., *, unique_keys: bool = ...) -> Expr: ...
def is_not_json(operand: Any, kind: JsonKind = ..., *, unique_keys: bool = ...) -> Expr: ...
