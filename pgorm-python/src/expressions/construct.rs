use pgorm::pgorm_query::{
    ColumnRef, Condition, ConditionExpression, Expr, Func, Query, SimpleExpr,
};
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyTuple};

use super::{PyExpr, compiled::Compiled};
use crate::{errors::ConstructionError, identifiers::Identifier, values::PyValue};

// [spec:pgorm:req:bindings.delegation]
// [spec:pgorm:req:bindings.identifiers+3]
#[pyfunction]
#[pyo3(signature = (name, *, table=None, schema=None))]
pub(crate) fn col(
    name: &Bound<'_, PyAny>,
    table: Option<&Bound<'_, PyAny>>,
    schema: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyExpr> {
    let name = Identifier::new(name)?.name();
    let table = table.map(Identifier::new).transpose()?;
    let schema = schema.map(Identifier::new).transpose()?;
    let column = match (schema, table) {
        (Some(schema), Some(table)) => {
            ColumnRef::SchemaTableColumn(schema.name(), table.name(), name)
        }
        (None, Some(table)) => ColumnRef::TableColumn(table.name(), name),
        (None, None) => ColumnRef::Column(name),
        (Some(_), None) => {
            return Err(ConstructionError::new_err(
                "a schema-qualified column requires a table",
            ));
        }
    };
    Ok(PyExpr::from_rust(Expr::col(column).into()))
}

/// The keyword `VALUE`, the value a domain's `CHECK` tests; anywhere else
/// PostgreSQL reads it as a column named `value`.
// [spec:pgorm:req:bindings.schema-domains+2]
#[pyfunction]
pub(crate) fn domain_value() -> PyExpr {
    PyExpr::from_rust(Expr::domain_value().into())
}

/// `CURRENT_DATE`, the date the transaction started.
// [spec:pgorm:req:bindings.current-keywords]
#[pyfunction]
pub(crate) fn current_date() -> PyExpr {
    PyExpr::from_rust(Expr::current_date().into())
}

/// `CURRENT_TIME`, the time of day the transaction started.
// [spec:pgorm:req:bindings.current-keywords]
#[pyfunction]
pub(crate) fn current_time() -> PyExpr {
    PyExpr::from_rust(Expr::current_time().into())
}

/// `CURRENT_TIMESTAMP`, the instant the transaction started.
// [spec:pgorm:req:bindings.current-keywords]
#[pyfunction]
pub(crate) fn current_timestamp() -> PyExpr {
    PyExpr::from_rust(Expr::current_timestamp().into())
}

#[pyfunction]
pub(crate) fn bind(value: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
    value_expr(value, false)
}

// [spec:pgorm:req:bindings.operands]
#[pyfunction]
pub(crate) fn literal(value: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
    value_expr(value, true)
}

fn value_expr(value: &Bound<'_, PyAny>, literal: bool) -> PyResult<PyExpr> {
    let value = PyValue::coerce(value)?;
    let inner = if literal {
        SimpleExpr::Constant(value.rust_value().clone())
    } else {
        Expr::value(value.rust_value().clone())
    };
    Ok(PyExpr::from_rust(value.typed(inner)))
}

/// An operand: an expression, or a value to bind. `ANY (..)` and `ALL (..)`
/// are refused by name, since only the operators PostgreSQL reads one after
/// take them, and those read them before coercing.
// [spec:pgorm:req:bindings.quantifiers+4]
pub(crate) fn coerce(value: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
    if let Ok(expr) = value.extract::<PyRef<'_, PyExpr>>() {
        Ok(expr.clone())
    } else if value.is_instance_of::<crate::statements::PyQuantified>() {
        Err(ConstructionError::new_err(
            "ANY and ALL are the right operand of a comparison, LIKE or ILIKE, a regular-expression \
             match, containment, a range position, matches or path_exists, and of nothing else",
        ))
    } else {
        bind(value)
    }
}

pub(super) fn sequence(values: &Bound<'_, PyAny>) -> PyResult<Vec<SimpleExpr>> {
    if !values.is_exact_instance_of::<pyo3::types::PyList>()
        && !values.is_exact_instance_of::<PyTuple>()
    {
        return Err(PyTypeError::new_err(
            "expression sequence requires a list or tuple",
        ));
    }
    values
        .try_iter()?
        .map(|value| coerce(&value?).map(|expr| expr.inner))
        .collect()
}

#[pyfunction]
#[pyo3(signature = (*expressions))]
pub(crate) fn tuple_expr(expressions: &Bound<'_, PyTuple>) -> PyResult<PyExpr> {
    if expressions.is_empty() {
        return Err(ConstructionError::new_err(
            "tuple expressions require at least one item",
        ));
    }
    Ok(PyExpr::from_rust(
        Expr::tuple(sequence(expressions.as_any())?).into(),
    ))
}

/// A reusable Rust condition; empty All/Any preserve the builder's identities.
#[pyclass(name = "Condition", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyCondition {
    pub inner: Condition,
}

#[pymethods]
impl PyCondition {
    #[staticmethod]
    #[pyo3(signature = (*conditions))]
    fn all(conditions: &Bound<'_, PyTuple>) -> PyResult<Self> {
        condition(Condition::all(), conditions)
    }

    #[staticmethod]
    #[pyo3(signature = (*conditions))]
    fn any(conditions: &Bound<'_, PyTuple>) -> PyResult<Self> {
        condition(Condition::any(), conditions)
    }

    fn add(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        let inner = if let Ok(other) = other.extract::<PyRef<'_, Self>>() {
            self.inner.clone().add(other.inner.clone())
        } else {
            self.inner.clone().add(require_expr(other)?.inner)
        };
        Ok(Self { inner })
    }

    // [spec:pgorm:req:bindings.condition-option]
    /// The condition with `other` added, or as it is when `other` is `None`,
    /// as Rust's `Condition::add_option` adds an optional condition.
    fn add_option(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        let other: Option<ConditionExpression> = if other.is_none() {
            None
        } else if let Ok(other) = other.extract::<PyRef<'_, Self>>() {
            Some(other.inner.clone().into())
        } else {
            Some(require_expr(other)?.inner.into())
        };
        Ok(Self {
            inner: self.inner.clone().add_option(other),
        })
    }

    fn __invert__(&self) -> Self {
        Self {
            inner: self.inner.clone().not(),
        }
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL conditions cannot be tested as Python booleans",
        ))
    }

    /// Inspect the condition in the WHERE clause of a Rust-built SELECT TRUE.
    fn inspect(&self) -> Compiled {
        let (sql, values) = Query::select()
            .expr(SimpleExpr::Constant(true.into()))
            .cond_where(self.inner.clone())
            .build();
        Compiled { sql, values }
    }

    fn __repr__(&self) -> String {
        format!("Condition(terms={})", self.inner.len())
    }
}

fn condition(mut inner: Condition, values: &Bound<'_, PyTuple>) -> PyResult<PyCondition> {
    for value in values.iter() {
        inner = if let Ok(condition) = value.extract::<PyRef<'_, PyCondition>>() {
            inner.add(condition.inner.clone())
        } else {
            inner.add(require_expr(&value)?.inner)
        };
    }
    Ok(PyCondition { inner })
}

pub(crate) fn require_expr(value: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
    value
        .extract::<PyRef<'_, PyExpr>>()
        .map(|value| value.clone())
        .map_err(|_| PyTypeError::new_err("expected a query expression"))
}

// [spec:pgorm:req:bindings.expressions+4]
#[pyfunction]
#[pyo3(signature = (name, *arguments))]
pub(crate) fn call(name: &str, arguments: &Bound<'_, PyTuple>) -> PyResult<PyExpr> {
    let args = sequence(arguments.as_any())?;
    if let ("grouping", [first, rest @ ..]) = (name, args.as_slice()) {
        let grouping = rest
            .iter()
            .fold(Func::grouping(first.clone()), |grouping, arg| {
                grouping.arg(arg.clone())
            });
        return Ok(PyExpr::from_rust(grouping.into()));
    }
    let expr = match (name, args.as_slice()) {
        ("lower", [value]) => Func::lower(value.clone()),
        ("upper", [value]) => Func::upper(value.clone()),
        ("abs", [value]) => Func::abs(value.clone()),
        ("char_length", [value]) => Func::char_length(value.clone()),
        ("count", [value]) => Func::count(value.clone()),
        ("count_distinct", [value]) => Func::count_distinct(value.clone()),
        ("sum", [value]) => Func::sum(value.clone()),
        ("avg", [value]) => Func::avg(value.clone()),
        ("min", [value]) => Func::min(value.clone()),
        ("max", [value]) => Func::max(value.clone()),
        ("round", [value]) => Func::round(value.clone()),
        ("round", [value, precision]) => {
            Func::round_with_precision(value.clone(), precision.clone())
        }
        ("coalesce", [_, ..]) => Func::coalesce(args),
        ("random", []) => Func::random(),
        ("gen_random_uuid", []) => Func::gen_random_uuid(),
        ("uuidv4", []) => Func::uuidv4(),
        ("uuidv7", []) => Func::uuidv7(),
        ("uuidv7", [shift]) => Func::uuidv7_shifted(shift.clone()),
        ("uuid_extract_timestamp", [value]) => Func::uuid_extract_timestamp(value.clone()),
        ("uuid_extract_version", [value]) => Func::uuid_extract_version(value.clone()),
        ("generate_series", [start, stop]) => Func::generate_series(start.clone(), stop.clone()),
        ("generate_series", [start, stop, step]) => {
            Func::generate_series_step(start.clone(), stop.clone(), step.clone())
        }
        ("unnest", [array]) => Func::unnest(array.clone()),
        ("json_to_record", [json]) => Func::json_to_record(json.clone()),
        ("jsonb_to_record", [jsonb]) => Func::jsonb_to_record(jsonb.clone()),
        ("json_to_recordset", [json]) => Func::json_to_recordset(json.clone()),
        ("jsonb_to_recordset", [jsonb]) => Func::jsonb_to_recordset(jsonb.clone()),
        ("to_tsvector", [text]) => Func::to_tsvector(text.clone(), None),
        ("to_tsquery", [text]) => Func::to_tsquery(text.clone(), None),
        ("plainto_tsquery", [text]) => Func::plainto_tsquery(text.clone(), None),
        ("phraseto_tsquery", [text]) => Func::phraseto_tsquery(text.clone(), None),
        ("websearch_to_tsquery", [text]) => Func::websearch_to_tsquery(text.clone(), None),
        ("ts_rank", [vector, query]) => Func::ts_rank(vector.clone(), query.clone()),
        ("ts_rank_cd", [vector, query]) => Func::ts_rank_cd(vector.clone(), query.clone()),
        _ => {
            return Err(crate::UnsupportedCapabilityError::new_err(
                "unsupported function or argument count",
            ));
        }
    };
    Ok(PyExpr::from_rust(expr.into()))
}

/// A call of the function `name`, any function the server has, with each
/// argument bound unless it is an expression: Rust's `Func::named`. The name
/// is an identifier, written bare when it is a lowercase name PostgreSQL
/// does not restrict and quoted otherwise, so it names a function and is
/// never read as syntax.
// [spec:pgorm:req:bindings.named-functions]
#[pyfunction]
#[pyo3(signature = (name, *arguments))]
pub(crate) fn func(name: &Bound<'_, PyAny>, arguments: &Bound<'_, PyTuple>) -> PyResult<PyExpr> {
    let name = Identifier::new(name)?.name();
    let args = sequence(arguments.as_any())?;
    Ok(PyExpr::from_rust(Func::named(name).args(args).into()))
}

/// A piece of SQL the builder has no method for, written as a t-string:
/// its text is SQL, and each interpolation goes between the pieces, an
/// expression embedded as it is and any other value bound, as Rust's
/// `Expr::template` binds its values. A `str` is refused, so text built at
/// run time cannot become a template by accident: the text is the
/// program's, as Rust's `&'static str` holds it to be.
// [spec:pgorm:req:bindings.template]
#[pyfunction]
pub(crate) fn template(text: &Bound<'_, PyAny>) -> PyResult<PyExpr> {
    let py = text.py();
    let templatelib = py.import("string.templatelib")?;
    if !text.is_instance(&templatelib.getattr("Template")?)? {
        return Err(PyTypeError::new_err(
            "a template is a t-string, t\"..\", whose text is the program's own",
        ));
    }
    let pieces: Vec<String> = text.getattr("strings")?.extract()?;
    let mut values = Vec::new();
    for interpolation in text.getattr("interpolations")?.try_iter()? {
        let interpolation = interpolation?;
        if !interpolation.getattr("conversion")?.is_none()
            || !interpolation
                .getattr("format_spec")?
                .extract::<String>()?
                .is_empty()
        {
            return Err(ConstructionError::new_err(
                "a template's interpolation is a value bound as it is: it takes no conversion or format",
            ));
        }
        values.push(coerce(&interpolation.getattr("value")?)?.inner);
    }
    let template = pgorm::pgorm_query::SqlTemplate::from_pieces(&pieces, values)
        .map_err(|error| ConstructionError::new_err(error.to_string()))?;
    Ok(PyExpr::from_rust(SimpleExpr::Template(template)))
}
