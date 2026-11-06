//! Windows over the Rust `WindowStatement` and its frame builder. The frame
//! classes follow the Rust typestate: a start knows which side of the current
//! row it lies on and offers only the ends PostgreSQL's grammar lets follow
//! it, so no end on an earlier side of the current row than its start can be
//! built. Two offsets on the same side are not compared, as the grammar does
//! not compare them: `Rows.preceding(1).and_preceding(5)` builds a frame that
//! holds no rows.

use pgorm::pgorm_query::{
    ExtendedWindow, FrameClause, FrameCurrentRow, FrameExclusion, FrameFollowing, FramePreceding,
    FrameStart, FrameType, Func, FunctionCall, JsonArrayAgg, JsonObjectAgg, Name, OverStatement,
    SelectStatement, SimpleExpr, SqlJson, WindowDefinition, WindowFunction, WindowStatement,
};
use pyo3::{prelude::*, types::PyTuple};

use super::common;
use crate::{
    errors::ConstructionError,
    expressions::{coerce, require_expr},
    identifiers::Identifier,
};

/// A frame's mode, and where a frame is begun. There is no
/// `unbounded_following` start: PostgreSQL's grammar refuses one.
#[pyclass(name = "FrameType", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PyFrameType {
    Range,
    Rows,
    Groups,
}

impl PyFrameType {
    fn rust(&self) -> FrameType {
        match self {
            Self::Range => FrameType::Range,
            Self::Rows => FrameType::Rows,
            Self::Groups => FrameType::Groups,
        }
    }
}

#[pymethods]
impl PyFrameType {
    /// `UNBOUNDED PRECEDING`: the partition's first row.
    fn unbounded_preceding(&self) -> PyFramePreceding {
        PyFramePreceding {
            inner: self.rust().unbounded_preceding(),
        }
    }

    /// `offset PRECEDING`: a count under `Rows` and `Groups`, a distance in
    /// the ordering column's values under `Range`.
    fn preceding(&self, offset: &Bound<'_, PyAny>) -> PyResult<PyFramePreceding> {
        Ok(PyFramePreceding {
            inner: self.rust().preceding(coerce(offset)?.inner),
        })
    }

    /// `CURRENT ROW`: under `Range` and `Groups`, the first of its peers.
    fn current_row(&self) -> PyFrameCurrentRow {
        PyFrameCurrentRow {
            inner: self.rust().current_row(),
        }
    }

    /// `offset FOLLOWING`, which needs a following end.
    fn following(&self, offset: &Bound<'_, PyAny>) -> PyResult<PyFrameFollowing> {
        Ok(PyFrameFollowing {
            inner: self.rust().following(coerce(offset)?.inner),
        })
    }

    // [spec:pgorm:req:python.truth-testing+5]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "window frames cannot be tested as Python booleans",
        ))
    }
}

/// The rows an `EXCLUDE` clause removes, relative to the current row's peers.
#[pyclass(name = "FrameExclusion", module = "pgorm", eq, from_py_object)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PyFrameExclusion {
    CurrentRow,
    Group,
    Ties,
    NoOthers,
}

#[pymethods]
impl PyFrameExclusion {
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "window frames cannot be tested as Python booleans",
        ))
    }
}

impl PyFrameExclusion {
    fn rust(&self) -> FrameExclusion {
        match self {
            Self::CurrentRow => FrameExclusion::CurrentRow,
            Self::Group => FrameExclusion::Group,
            Self::Ties => FrameExclusion::Ties,
            Self::NoOthers => FrameExclusion::NoOthers,
        }
    }
}

/// A whole frame: a mode, a start, an optional end and an optional
/// `EXCLUDE` clause.
#[pyclass(name = "Frame", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyFrame {
    inner: FrameClause,
}

#[pymethods]
impl PyFrame {
    fn exclude(&self, exclusion: PyFrameExclusion) -> Self {
        Self {
            inner: self.inner.clone().exclude(exclusion.rust()),
        }
    }

    fn __repr__(&self) -> &'static str {
        "Frame(...)"
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "window frames cannot be tested as Python booleans",
        ))
    }
}

/// A start before the current row. Every end may follow it, and it stands
/// alone as a whole frame.
#[pyclass(name = "FramePrecedingStart", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyFramePreceding {
    inner: FrameStart<FramePreceding>,
}

#[pymethods]
impl PyFramePreceding {
    fn and_preceding(&self, offset: &Bound<'_, PyAny>) -> PyResult<PyFrame> {
        Ok(PyFrame {
            inner: self.inner.clone().and_preceding(coerce(offset)?.inner),
        })
    }
    fn and_current_row(&self) -> PyFrame {
        PyFrame {
            inner: self.inner.clone().and_current_row(),
        }
    }
    fn and_following(&self, offset: &Bound<'_, PyAny>) -> PyResult<PyFrame> {
        Ok(PyFrame {
            inner: self.inner.clone().and_following(coerce(offset)?.inner),
        })
    }
    fn and_unbounded_following(&self) -> PyFrame {
        PyFrame {
            inner: self.inner.clone().and_unbounded_following(),
        }
    }
    fn exclude(&self, exclusion: PyFrameExclusion) -> PyFrame {
        PyFrame {
            inner: self.inner.clone().exclude(exclusion.rust()),
        }
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "window frames cannot be tested as Python booleans",
        ))
    }
}

/// A start at the current row. Any end but a preceding one may follow it,
/// and it stands alone as a whole frame.
#[pyclass(
    name = "FrameCurrentRowStart",
    module = "pgorm",
    frozen,
    from_py_object
)]
#[derive(Clone, Debug)]
pub struct PyFrameCurrentRow {
    inner: FrameStart<FrameCurrentRow>,
}

#[pymethods]
impl PyFrameCurrentRow {
    fn and_current_row(&self) -> PyFrame {
        PyFrame {
            inner: self.inner.clone().and_current_row(),
        }
    }
    fn and_following(&self, offset: &Bound<'_, PyAny>) -> PyResult<PyFrame> {
        Ok(PyFrame {
            inner: self.inner.clone().and_following(coerce(offset)?.inner),
        })
    }
    fn and_unbounded_following(&self) -> PyFrame {
        PyFrame {
            inner: self.inner.clone().and_unbounded_following(),
        }
    }
    fn exclude(&self, exclusion: PyFrameExclusion) -> PyFrame {
        PyFrame {
            inner: self.inner.clone().exclude(exclusion.rust()),
        }
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "window frames cannot be tested as Python booleans",
        ))
    }
}

/// A start after the current row. Only a following end may follow it, and
/// it cannot stand alone: PostgreSQL reads a lone start as running to the
/// current row, which lies behind it.
#[pyclass(name = "FrameFollowingStart", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyFrameFollowing {
    inner: FrameStart<FrameFollowing>,
}

#[pymethods]
impl PyFrameFollowing {
    fn and_following(&self, offset: &Bound<'_, PyAny>) -> PyResult<PyFrame> {
        Ok(PyFrame {
            inner: self.inner.clone().and_following(coerce(offset)?.inner),
        })
    }
    fn and_unbounded_following(&self) -> PyFrame {
        PyFrame {
            inner: self.inner.clone().and_unbounded_following(),
        }
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "window frames cannot be tested as Python booleans",
        ))
    }
}

// [spec:pgorm:req:bindings.windows]
/// An `OVER` window: PARTITION BY, ORDER BY and a frame, owned by Rust's
/// `WindowStatement`.
#[pyclass(name = "Window", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug, Default)]
pub struct PyWindow {
    pub(super) inner: WindowStatement,
}

#[pymethods]
impl PyWindow {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    #[pyo3(signature = (*expressions))]
    fn partition_by(&self, expressions: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let mut next = self.clone();
        for expression in expressions.iter() {
            next.inner
                .add_partition_by(require_expr(&expression)?.inner);
        }
        Ok(next)
    }

    #[pyo3(signature = (*orderings))]
    fn order_by(&self, orderings: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let mut next = self.clone();
        common::order(&mut next.inner, orderings)?;
        Ok(next)
    }

    /// Set the frame, replacing any already set: a `Frame`, or a preceding or
    /// current-row start standing alone.
    fn frame(&self, frame: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner.frame(frame_clause(frame)?);
        Ok(next)
    }

    fn __repr__(&self) -> &'static str {
        "Window(...)"
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "windows cannot be tested as Python booleans",
        ))
    }
}

/// The frame a window's `frame` takes: a whole frame, or a preceding or
/// current-row start standing alone.
fn frame_clause(frame: &Bound<'_, PyAny>) -> PyResult<FrameClause> {
    if let Ok(frame) = frame.extract::<PyRef<'_, PyFrame>>() {
        Ok(frame.inner.clone())
    } else if let Ok(start) = frame.extract::<PyRef<'_, PyFramePreceding>>() {
        Ok(start.inner.clone().into())
    } else if let Ok(start) = frame.extract::<PyRef<'_, PyFrameCurrentRow>>() {
        Ok(start.inner.clone().into())
    } else {
        Err(pyo3::exceptions::PyTypeError::new_err(
            "a window's frame requires a Frame, or a preceding or current-row start; \
             a following start needs an end",
        ))
    }
}

// [spec:pgorm:req:bindings.windows]
/// A window built on an entry of a SELECT's WINDOW clause, owned by Rust's
/// `ExtendedWindow`: it copies the base's PARTITION BY and any ORDER BY, and
/// adds an ORDER BY and a frame. Declared with `Select.window` it builds on
/// an earlier entry; written inline after `over` it builds on any. It has no
/// `partition_by`, because PostgreSQL refuses one there.
#[pyclass(name = "ExtendedWindow", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyExtendedWindow {
    inner: ExtendedWindow,
}

#[pymethods]
impl PyExtendedWindow {
    #[new]
    fn new(base: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: ExtendedWindow::new(Identifier::new(base)?.name()),
        })
    }

    #[pyo3(signature = (*orderings))]
    fn order_by(&self, orderings: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let mut next = self.clone();
        common::order(&mut next.inner, orderings)?;
        Ok(next)
    }

    /// Set the frame, replacing any already set, as `Window.frame` does.
    fn frame(&self, frame: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut next = self.clone();
        next.inner.frame(frame_clause(frame)?);
        Ok(next)
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "windows cannot be tested as Python booleans",
        ))
    }

    fn __repr__(&self) -> &'static str {
        "ExtendedWindow(...)"
    }
}

// [spec:pgorm:req:bindings.windows]
/// What `Select.window` declares a name as: a `Window`, or an
/// `ExtendedWindow` built on an earlier entry. Anything else is a
/// `TypeError`, as an argument of the wrong class is.
pub(super) fn window_definition(window: &Bound<'_, PyAny>) -> PyResult<WindowDefinition> {
    if let Ok(window) = window.extract::<PyRef<'_, PyWindow>>() {
        Ok(window.inner.clone().into())
    } else if let Ok(window) = window.extract::<PyRef<'_, PyExtendedWindow>>() {
        Ok(window.inner.clone().into())
    } else {
        Err(pyo3::exceptions::PyTypeError::new_err(
            "a named window is a Window or an ExtendedWindow",
        ))
    }
}

/// What `OVER` may follow: a function call or one of SQL/JSON's two
/// aggregates, as Rust's sealed `WindowFunction` has it.
#[derive(Clone, Debug)]
pub(crate) enum WindowCall {
    Function(FunctionCall),
    ArrayAgg(JsonArrayAgg),
    ObjectAgg(JsonObjectAgg),
}

impl WindowCall {
    /// The call an expression is, refused unless the grammar puts it before
    /// `OVER`: a column, arithmetic, a cast or another SQL/JSON function is
    /// not.
    pub(crate) fn of(expression: &SimpleExpr) -> PyResult<Self> {
        match expression {
            SimpleExpr::FunctionCall(call) => Ok(Self::Function(call.clone())),
            SimpleExpr::SqlJson(json) => match json.as_ref() {
                SqlJson::ArrayAgg(aggregate) => Ok(Self::ArrayAgg(aggregate.clone())),
                SqlJson::ObjectAgg(aggregate) => Ok(Self::ObjectAgg(aggregate.clone())),
                _ => Err(not_windowed()),
            },
            _ => Err(not_windowed()),
        }
    }

    /// This call over a `Window` or an `ExtendedWindow` written inline, or
    /// over the window a statement names.
    pub(crate) fn over(self, window: &Bound<'_, PyAny>) -> PyResult<PyWindowed> {
        let over = if let Ok(window) = window.extract::<PyRef<'_, PyWindow>>() {
            Over::Inline(window.inner.clone().into())
        } else if let Ok(window) = window.extract::<PyRef<'_, PyExtendedWindow>>() {
            Over::Inline(window.inner.clone().into())
        } else {
            Over::Named(Identifier::new(window)?.name())
        };
        Ok(PyWindowed {
            call: self,
            over,
            alias: None,
        })
    }
}

fn not_windowed() -> PyErr {
    ConstructionError::new_err(
        "OVER follows only a function call or JSON_ARRAYAGG / JSON_OBJECTAGG",
    )
}

/// The window a call runs over: one written inline, which may build on an
/// entry of the statement's WINDOW clause, or the name of the one the
/// statement declares with `Select.window`.
#[derive(Clone, Debug)]
enum Over {
    Inline(WindowDefinition),
    Named(Name),
}

// [spec:pgorm:req:bindings.windows]
/// A function call under `OVER`, a SELECT item built by `over`.
#[pyclass(name = "Windowed", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyWindowed {
    call: WindowCall,
    over: Over,
    alias: Option<Name>,
}

impl PyWindowed {
    /// Add this item to a SELECT list through the Rust method its window and
    /// alias call for.
    pub(super) fn project(&self, query: &mut SelectStatement) {
        match &self.call {
            WindowCall::Function(call) => self.project_call(query, call.clone()),
            WindowCall::ArrayAgg(call) => self.project_call(query, call.clone()),
            WindowCall::ObjectAgg(call) => self.project_call(query, call.clone()),
        }
    }

    fn project_call<F: WindowFunction>(&self, query: &mut SelectStatement, call: F) {
        match (&self.over, &self.alias) {
            (Over::Inline(window), None) => query.expr_window(call, window.clone()),
            (Over::Inline(window), Some(alias)) => {
                query.expr_window_as(call, window.clone(), alias.clone())
            }
            (Over::Named(name), None) => query.expr_window_name(call, name.clone()),
            (Over::Named(name), Some(alias)) => {
                query.expr_window_name_as(call, name.clone(), alias.clone())
            }
        };
    }
}

#[pymethods]
impl PyWindowed {
    fn as_(&self, alias: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            alias: Some(Identifier::new(alias)?.name()),
            ..self.clone()
        })
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL projections cannot be tested as Python booleans",
        ))
    }

    fn __repr__(&self) -> &'static str {
        "Windowed(...)"
    }
}

// [spec:pgorm:req:bindings.windows]
/// A function PostgreSQL computes only over a window, built by
/// `window_function`. Its one method is `over`, so it cannot stand in a query
/// without its window (`42809`).
#[pyclass(name = "WindowFunction", module = "pgorm", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyWindowFunction {
    inner: FunctionCall,
}

#[pymethods]
impl PyWindowFunction {
    fn over(&self, window: &Bound<'_, PyAny>) -> PyResult<PyWindowed> {
        let call = WindowCall::Function(self.inner.clone());
        call.over(window)
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL expressions cannot be tested as Python booleans",
        ))
    }

    fn __repr__(&self) -> &'static str {
        "WindowFunction(...)"
    }
}

/// PostgreSQL's general-purpose window functions with the argument counts
/// each takes, lowered into `Func::named` with the arguments bound or written
/// as the caller built them.
#[pyfunction]
#[pyo3(signature = (name, *arguments))]
pub(crate) fn window_function(
    name: &str,
    arguments: &Bound<'_, PyTuple>,
) -> PyResult<PyWindowFunction> {
    let accepted = match name {
        "row_number" | "rank" | "dense_rank" | "percent_rank" | "cume_dist" => Some(0..=0),
        "ntile" | "first_value" | "last_value" => Some(1..=1),
        "nth_value" => Some(2..=2),
        "lag" | "lead" => Some(1..=3),
        _ => None,
    };
    if !accepted.is_some_and(|counts| counts.contains(&arguments.len())) {
        return Err(crate::UnsupportedCapabilityError::new_err(
            "unsupported window function or argument count",
        ));
    }
    let arguments = arguments
        .iter()
        .map(|argument| coerce(&argument).map(|argument| argument.inner))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(PyWindowFunction {
        inner: Func::named(Name::runtime(name)).args(arguments),
    })
}
