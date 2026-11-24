//! Range and multirange literals, and `CREATE TYPE ... AS RANGE`'s option
//! list.

use std::ops::Bound;

use super::*;
use crate::extension::RangeDefinition;

impl QueryBuilder {
    /// A range literal, written as a call of the range type's own
    /// constructor: `int4range(1, 5, '[)')`.
    ///
    /// The constructor takes each bound as a value of the subtype, so a bound
    /// is written by the same literal rendering as any other value of its
    /// variant and the range needs no quoting rules of its own; an unbounded
    /// side is `NULL`, which is how the constructor reads one, and a NULL
    /// bound therefore means what it means on the bound path. The empty range
    /// has no bounds to pass and is the one textual literal,
    /// `'empty'::int4range`.
    // [spec:pgorm:sem:sql.value.render+2]
    pub(super) fn write_range(&self, ty: RangeType, range: &Range<Value>, s: &mut String) {
        match range {
            Range::Empty => write!(s, "'empty'::{}", ty.range_type_name()).unwrap(),
            Range::Bounds { lower, upper } => {
                let (lower, lower_flag) = self.range_bound(lower, '[', '(');
                let (upper, upper_flag) = self.range_bound(upper, ']', ')');
                write!(
                    s,
                    "{}({lower}, {upper}, '{lower_flag}{upper_flag}')",
                    ty.range_type_name()
                )
                .unwrap()
            }
        }
    }

    /// One side of a range literal: the bound's value, or `NULL` for no
    /// bound, and the bracket that says whether the value is inside.
    fn range_bound(&self, bound: &Bound<Value>, included: char, excluded: char) -> (String, char) {
        match bound {
            Bound::Included(value) => (self.value_to_string(value), included),
            Bound::Excluded(value) => (self.value_to_string(value), excluded),
            Bound::Unbounded => ("NULL".to_owned(), excluded),
        }
    }

    /// A multirange literal, its ranges passed to the multirange type's
    /// constructor: `int4multirange(int4range(1, 3, '[)'), ...)`, and
    /// `int4multirange()` for the empty one.
    // [spec:pgorm:sem:sql.value.render+2]
    pub(super) fn write_multirange(
        &self,
        ty: RangeType,
        multirange: &Multirange<Value>,
        s: &mut String,
    ) {
        write!(s, "{}(", ty.multirange_type_name()).unwrap();
        multirange.iter().fold(true, |first, range| {
            if !first {
                write!(s, ", ").unwrap();
            }
            self.write_range(ty, range, s);
            false
        });
        write!(s, ")").unwrap();
    }

    /// A range type's `RANGE (SUBTYPE = <type>, ...)`: the subtype written as
    /// a column's type is, then each option that is set, in the order
    /// PostgreSQL documents them. Every name is quoted; the multirange's is
    /// the type name it will be, schema-qualified as the range's can be.
    // [spec:pgorm:req:sql.ddl.type-range+3]
    pub(super) fn prepare_range_definition(
        &self,
        range: &RangeDefinition,
        sql: &mut dyn SqlWriter,
    ) {
        write!(sql, "RANGE (SUBTYPE = ").unwrap();
        self.prepare_column_type(&range.subtype, sql);
        if let Some(opclass) = &range.subtype_opclass {
            write!(sql, ", SUBTYPE_OPCLASS = ").unwrap();
            opclass.prepare(sql.as_writer());
        }
        if let Some(collation) = &range.collation {
            write!(sql, ", COLLATION = ").unwrap();
            collation.prepare(sql.as_writer());
        }
        if let Some(function) = &range.subtype_diff {
            write!(sql, ", SUBTYPE_DIFF = ").unwrap();
            function.prepare(sql.as_writer());
        }
        if let Some(name) = &range.multirange_type_name {
            write!(sql, ", MULTIRANGE_TYPE_NAME = ").unwrap();
            self.prepare_type_ref(name, sql);
        }
        write!(sql, ")").unwrap();
    }
}
