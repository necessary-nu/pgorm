from typing import Any, Never
from ._conditions import Condition as Condition
from ._expr_comparison import _ExprComparison
from ._expr_arithmetic import _ExprArithmetic
from ._expr_logic import _ExprLogic
from ._expr_predicates import _ExprPredicates
from ._expr_patterns import _ExprPatterns
from ._expr_ranges import _ExprRanges
from ._expr_search import _ExprSearch
from ._expr_shaping import _ExprShaping
from ._expr_subscripts import _ExprSubscripts

class Expr(
    _ExprComparison,
    _ExprArithmetic,
    _ExprLogic,
    _ExprPredicates,
    _ExprPatterns,
    _ExprRanges,
    _ExprSearch,
    _ExprShaping,
    _ExprSubscripts,
):
    def __init__(self, _native_only: Never, /) -> None: ...

class _DeclaredColumn(Expr):
    def __init__(self, expression: Expr) -> None: ...

def col(name: str, *, table: str | None = ...,
        schema: str | None = ...) -> Expr: ...
def bind(value: Any) -> Expr: ...
def literal(value: Any) -> Expr: ...
def call(name: str, *arguments: Any) -> Expr: ...
def tuple_expr(*expressions: Any) -> Expr: ...
def domain_value() -> Expr: ...
