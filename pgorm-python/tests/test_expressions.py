"""Compose public native builders in one installed extension without HTTP."""

import concurrent.futures
import unittest

from pgorm import (
    Condition, ConstructionError, Direction, Expr, Identifier, LikeExpr,
    Nulls, OrderBy, TypeName, UnsupportedCapabilityError, Value, Window,
    bind, call, col, format_json, literal, require_capability, tuple_expr, window_function,
)


def inspection(expression):
    compiled = expression.inspect()
    return compiled.sql, [value.snapshot() for value in compiled.params]


# [spec:pgorm:req:bindings.expressions+3/test]
# [spec:pgorm:req:bindings.identifiers+2/test]
# [spec:pgorm:req:bindings.operands/test]
# [spec:pgorm:req:bindings.statements/test]
# [spec:pgorm:req:bindings.delegation/test]
class ExpressionTests(unittest.TestCase):
    def test_identifiers_and_values_have_separate_paths(self):
        name = 'A"雪.%\\'
        value = "' OR TRUE; -- %\\雪"
        expression = col(Identifier(name), table="T", schema="S").eq(value)
        compiled = expression.inspect()
        self.assertEqual(compiled.sql, 'SELECT "S"."T"."A""雪.%\\" = $1')
        self.assertEqual(compiled.params, [Value(value)])
        self.assertNotIn(value, compiled.sql)
        for name in ["", "a" * 64, "雪" * 22, "a\0b"]:
            with self.assertRaises(ConstructionError):
                col(name)
        for name in [1, Value("column"), None]:
            with self.assertRaises(TypeError):
                col(name)
        with self.assertRaises(ConstructionError):
            col("x", schema="s")
        with self.assertRaises(ConstructionError):
            bind(Identifier("x"))

    def test_literal_and_bound_values_stay_selectable(self):
        expression = bind(Value(7, "i16")) + literal(3)
        compiled = expression.inspect()
        self.assertEqual(compiled.sql, "SELECT $1 + 3")
        self.assertEqual(compiled.params, [Value(7, "i16")])
        self.assertEqual(literal("O'Brien").inspect().sql, r"SELECT E'O\'Brien'")
        self.assertEqual(literal("O'Brien").inspect().params, [])
        self.assertEqual(bind("O'Brien").inspect().sql, "SELECT $1")

    def test_comparisons_and_arithmetic_remain_expressions(self):
        left = col("x")
        comparisons = [left == 3, left != 3, left < 3, left <= 3, left > 3, left >= 3]
        for method, expression in zip(["eq", "ne", "lt", "lte", "gt", "gte"], comparisons):
            self.assertIsInstance(expression, Expr)
            self.assertEqual(inspection(expression), inspection(getattr(left, method)(3)))
        for expression, operator in [(left + 2, "+"), (left - 2, "-"),
                                     (left * 2, "*"), (left / 2, "/"), (left % 2, "%")]:
            self.assertIn(operator, expression.inspect().sql)
            self.assertEqual(expression.inspect().params, [Value(2)])
        self.assertEqual(left.inspect().sql, 'SELECT "x"')

    def test_boolean_conversion_fails_at_the_boundary(self):
        expression = col("x") > 1
        ranked = window_function("rank")
        for value in [expression, Condition.all(expression), expression.as_("result"), expression.asc(),
                      format_json(col("doc")), ranked, ranked.over(Window()), expression.inspect()]:
            with self.assertRaises(ConstructionError):
                bool(value)
        with self.assertRaises(ConstructionError):
            expression and col("y") > 2
        with self.assertRaises(ConstructionError):
            1 < col("x") < 10
        with self.assertRaises(TypeError):
            expression & True
        with self.assertRaises(TypeError):
            Condition.any(True)

    def test_nested_conditions_keep_parameter_order(self):
        condition = Condition.all(col("a") == 1, Condition.any(col("b") == 2, col("c").is_null()))
        compiled = condition.inspect()
        self.assertEqual(compiled.sql, 'SELECT TRUE WHERE "a" = $1 AND ("b" = $2 OR "c" IS NULL)')
        self.assertEqual(compiled.params, [Value(1), Value(2)])
        negated = (~condition).inspect()
        self.assertIn("NOT", negated.sql)
        self.assertEqual(negated.params, compiled.params)
        self.assertEqual(Condition.all().inspect().sql, "SELECT TRUE WHERE TRUE")
        self.assertEqual(Condition.any().inspect().sql, "SELECT TRUE WHERE FALSE")

    def test_membership_and_null_checks_use_rust_semantics(self):
        expression = col("x")
        empty_in = expression.is_in([]).inspect()
        empty_not_in = expression.is_not_in([]).inspect()
        self.assertEqual(empty_in.sql, "SELECT $1 = $2")
        self.assertEqual(empty_in.params, [Value("a"), Value("b")])
        self.assertEqual(empty_not_in.sql, "SELECT $1 = $2")
        self.assertEqual(empty_not_in.params, [Value("a"), Value("a")])
        compiled = expression.is_in([1, literal(2), Value.null("i64")]).inspect()
        self.assertEqual(compiled.sql, 'SELECT "x" IN ($1, 2, $2)')
        self.assertEqual(compiled.params, [Value(1), Value.null("i64")])
        self.assertEqual(expression.is_not_null().inspect().sql, 'SELECT "x" IS NOT NULL')
        self.assertEqual(expression.between(1, 3).inspect().params, [Value(1), Value(3)])
        self.assertIn("NOT BETWEEN", expression.not_between(1, 3).inspect().sql)
        with self.assertRaises(TypeError):
            expression.is_in("abc")

    def test_pattern_and_substring_semantics_are_distinct(self):
        needle = "%_\\雪'"
        expression = col("text")
        for method in ["starts_with", "ends_with", "contains_text"]:
            compiled = getattr(expression, method)(needle).inspect()
            self.assertTrue(compiled.params)
            self.assertTrue(all(value.value == needle for value in compiled.params))
            self.assertNotIn(needle, compiled.sql)
        pattern = LikeExpr(needle).escape("!")
        for method in ["like", "not_like", "ilike", "not_ilike"]:
            compiled = getattr(expression, method)(pattern).inspect()
            self.assertEqual(compiled.params, [Value(needle)])
            self.assertIn("LIKE", compiled.sql)
            self.assertIn("ESCAPE '!'", compiled.sql)
            plain = getattr(expression, method)(needle).inspect()
            self.assertEqual(plain.params, [Value(needle)])
            self.assertNotIn("ESCAPE", plain.sql)
            self.assertEqual(inspection(getattr(expression, method)(LikeExpr(needle))),
                             inspection(getattr(expression, method)(needle)))
        for escape in ["", "xy", "\0"]:
            with self.assertRaises(ConstructionError):
                LikeExpr("pattern").escape(escape)
        for build in [lambda: LikeExpr(1), lambda: LikeExpr("pattern").escape(1),
                      lambda: expression.like(1), lambda: expression.similar_to(Value("x"))]:
            with self.assertRaises(TypeError):
                build()
        compiled = expression.like(LikeExpr(needle).no_escape()).inspect()
        self.assertTrue(compiled.sql.endswith("ESCAPE ''"))
        self.assertEqual(compiled.params, [Value(needle)])
        replaced = expression.like(LikeExpr(needle).no_escape().escape("#")).inspect()
        self.assertTrue(replaced.sql.endswith("ESCAPE '#'"))

    def test_casts_retain_qualified_enum_identity(self):
        kind = TypeName('Mood"', schema="Tenant")
        value = Value("calm'", kind)
        compiled = bind(value).inspect()
        self.assertEqual(compiled.sql, 'SELECT CAST($1::text AS "Tenant"."Mood""")')
        self.assertEqual(compiled.params, [Value("calm'")])
        array = bind(Value.array(kind, [value])).inspect()
        self.assertIn('AS "Tenant"."Mood"""[]', array.sql)
        cast = col("state").cast(kind).inspect()
        self.assertEqual(cast.params, [])
        self.assertIn('CAST("state" AS "Tenant".', cast.sql)
        self.assertNotEqual(compiled.sql, bind(Value(value.value, TypeName(kind.name, schema="Other"))).inspect().sql)

    def test_function_capabilities_reject_unknown_calls(self):
        self.assertEqual(call("lower", col("name")).inspect().sql, 'SELECT LOWER("name")')
        self.assertEqual(call("coalesce", col("x"), bind(1)).inspect().params, [Value(1)])
        self.assertEqual(tuple_expr(col("x"), literal(2)).inspect().sql, 'SELECT ("x", 2)')
        for name, args in [("lower", []), ("lower", [1, 2]), ("coalesce", []),
                           ("unknown", [1]), ("lower); DROP TABLE x --", [1])]:
            with self.assertRaises(UnsupportedCapabilityError):
                call(name, *args)
        require_capability("expr.inspect")
        with self.assertRaises(ConstructionError):
            tuple_expr()

    def test_reuse_clones_builder_state_and_parameters(self):
        common = col("tenant") == "one"
        before = inspection(common)
        left = common & (col("id") > 1)
        right = common | (col("id") < 100)
        self.assertEqual(inspection(common), before)
        self.assertEqual(left.inspect().params, [Value("one"), Value(1)])
        self.assertEqual(right.inspect().params, [Value("one"), Value(100)])
        left.inspect().params.clear()
        self.assertEqual(inspection(common), before)
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
            results = list(executor.map(lambda _: inspection(common), range(20)))
        self.assertEqual(results, [before] * 20)

    def test_ordering_and_projection_options_are_typed(self):
        expression = col("x")
        ordering = expression.desc(nulls=Nulls.Last)
        self.assertEqual(ordering.direction, Direction.Desc)
        self.assertEqual(ordering.nulls, Nulls.Last)
        self.assertEqual(OrderBy(expression, Direction.Asc).direction, Direction.Asc)
        alias = expression.as_('result"')
        self.assertEqual(alias.alias.name, 'result"')
        self.assertEqual(inspection(alias.expr), inspection(expression))
        with self.assertRaises(TypeError):
            OrderBy(expression, "DESC; SELECT")
        with self.assertRaises(TypeError):
            expression.asc(nulls="first")
        with self.assertRaises(AttributeError):
            ordering.direction = Direction.Asc


if __name__ == "__main__":
    unittest.main()
