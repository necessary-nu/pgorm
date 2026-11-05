//! Operator precedence: which operands of an operator render bare and which
//! keep parentheses.

use super::typed_operands::{Side, operator_reads_type};
use super::*;

impl QueryBuilder {
    /// Translate a binary expr to SQL.
    // [spec:pgorm:req:sql.render.parens+5]
    pub(super) fn binary_expr(
        &self,
        left: &SimpleExpr,
        op: &BinOper,
        right: &SimpleExpr,
        sql: &mut dyn SqlWriter,
    ) {
        let outer = Oper::BinOper(*op);
        let left_bare = self.inner_expr_well_known_greater_precedence(left, &outer)
            || (left.get_bin_oper() == Some(op) && self.well_known_left_associative(op));
        if operator_reads_type(op, Side::Left, left) {
            self.prepare_typed_operand(left, sql);
        } else {
            self.prepare_operand(left, left_bare, sql);
        }

        write!(sql, " ").unwrap();
        self.prepare_bin_oper(op, sql);
        write!(sql, " ").unwrap();

        match right {
            // The ternary BETWEEN travels as a binary whose right operand is an
            // AND over its bounds. That AND is BETWEEN's own keyword, so each
            // bound is an operand of the BETWEEN: judged against the AND, a
            // comparison bound would render bare and the grammar would take
            // `x BETWEEN a AND c = d` as `(x BETWEEN a AND c) = d`.
            SimpleExpr::Binary(lower, BinOper::And, upper) if outer.is_between() => {
                let lower_bare = self.inner_expr_well_known_greater_precedence(lower, &outer);
                self.prepare_operand(lower, lower_bare, sql);
                write!(sql, " AND ").unwrap();
                let upper_bare = self.inner_expr_well_known_greater_precedence(upper, &outer);
                self.prepare_operand(upper, upper_bare, sql);
            }
            _ if operator_reads_type(op, Side::Right, right) => {
                self.prepare_typed_operand(right, sql);
            }
            _ => {
                let right_bare = self.inner_expr_well_known_greater_precedence(right, &outer);
                self.prepare_operand(right, right_bare, sql);
            }
        }
    }

    pub(super) fn prepare_operand(
        &self,
        operand: &SimpleExpr,
        bare: bool,
        sql: &mut dyn SqlWriter,
    ) {
        if !bare {
            write!(sql, "(").unwrap();
        }
        self.prepare_simple_expr(operand, sql);
        if !bare {
            write!(sql, ")").unwrap();
        }
    }

    /// Whether parentheses around `inner` can be dropped inside `outer_oper`.
    ///
    /// One table, because there is one dialect. Atoms never need them —
    /// all but a column and a tuple already wrap themselves. A binary inner
    /// expression keeps them unless it binds tighter than the operator it
    /// sits under: arithmetic and shifts bind tighter than comparison,
    /// BETWEEN, IN, LIKE and the logical operators; a boolean generic
    /// operator binds tighter than all of those and IS; anything else that
    /// returns a boolean, BETWEEN included, binds tighter than
    /// `AND`/`OR`/`NOT`. Every other pairing is unknown and keeps its
    /// parentheses.
    // [spec:pgorm:def:sql.render.precedence+14]
    pub(super) fn inner_expr_well_known_greater_precedence(
        &self,
        inner: &SimpleExpr,
        outer_oper: &Oper,
    ) -> bool {
        match inner {
            SimpleExpr::Column(_)
            | SimpleExpr::Tuple(_)
            | SimpleExpr::Constant(_)
            | SimpleExpr::FunctionCall(_)
            | SimpleExpr::Value(_)
            | SimpleExpr::Keyword(_)
            | SimpleExpr::Case(_)
            | SimpleExpr::SimpleCase(_)
            | SimpleExpr::Subscript(_, _)
            | SimpleExpr::Collate(_, _)
            | SimpleExpr::Grouping(_)
            | SimpleExpr::LikePattern(_)
            | SimpleExpr::SqlJson(_)
            | SimpleExpr::AsEnum(_, _)
            | SimpleExpr::SubQuery(_, _) => true,
            SimpleExpr::Binary(_, inner_bin_oper, _) => {
                let inner_oper: Oper = (*inner_bin_oper).into();
                if inner_oper.is_arithmetic() || inner_oper.is_shift() {
                    outer_oper.is_comparison()
                        || outer_oper.is_between()
                        || outer_oper.is_in()
                        || outer_oper.is_like()
                        || outer_oper.is_logical()
                } else if returns_boolean(inner_bin_oper) {
                    outer_oper.is_comparison()
                        || outer_oper.is_between()
                        || outer_oper.is_in()
                        || outer_oper.is_like()
                        || outer_oper.is_is()
                        || outer_oper.is_logical()
                } else if inner_oper.is_comparison()
                    || inner_oper.is_between()
                    || inner_oper.is_in()
                    || inner_oper.is_like()
                    || inner_oper.is_is()
                {
                    outer_oper.is_logical()
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// The operators whose repetition renders flat: `a AND b AND c`,
    /// `a || b || c`.
    // [spec:pgorm:req:sql.render.parens+5]
    fn well_known_left_associative(&self, op: &BinOper) -> bool {
        matches!(
            op,
            BinOper::And
                | BinOper::Or
                | BinOper::Add
                | BinOper::Sub
                | BinOper::Mul
                | BinOper::Mod
                | BinOper::Concatenate
        )
    }
}

/// The operators outside the comparison/BETWEEN/IN/LIKE/IS families that
/// still yield a boolean. Each is a generic operator, which the grammar ranks
/// above every comparison, IS, BETWEEN, IN and LIKE, or the similarity `%`,
/// ranked with `*`, so each binds tighter than any of those and than a
/// logical operator. Membership is exactly "returns
/// boolean", which is why the JSON *existence* tests are here and the JSON
/// accessors — `->`, `->>`, `#>`, `#>>`, which return JSON or text — are not;
/// a boolean-valued variant missing here keeps parentheses its peers drop.
// [spec:pgorm:def:sql.render.precedence+14]
fn returns_boolean(b: &BinOper) -> bool {
    matches!(
        b,
        BinOper::Contained
            | BinOper::Contains
            | BinOper::Overlap
            | BinOper::Similarity
            | BinOper::WordSimilarity
            | BinOper::StrictWordSimilarity
            | BinOper::Matches
            | BinOper::HasJsonKey
            | BinOper::HasAnyJsonKeys
            | BinOper::HasAllJsonKeys
            | BinOper::JsonPathExists
            | BinOper::Regex
            | BinOper::RegexCaseInsensitive
            | BinOper::NotRegex
            | BinOper::NotRegexCaseInsensitive
            | BinOper::NotExtendRightOf
            | BinOper::NotExtendLeftOf
            | BinOper::AdjacentTo
    )
}
