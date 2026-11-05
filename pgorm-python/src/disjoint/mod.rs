//! Disjoint models and registered `#[derive(Disjoint)]` enums, over pgorm's
//! [`DisjointShape`].
//!
//! Either is one native `_Disjoint`: a shape, from models Python declares or
//! from the registered enum, and a codec reading each row's records. Every
//! statement is the shape's — the read and its narrowing, the writes, the
//! cursor, the pages and the schema — so a model's statements are the ones
//! the Rust declaration of the same tables sends; the codec only reads the
//! rows back, a model's as `Record`s Python views by field and a
//! registration's as its entities' own models.
//!
//! What the shape refuses as it builds a statement (`Error::Type`,
//! `Error::PrimaryKeyNotSet`) is refused before anything is sent, so it is a
//! `ConstructionError`; what a row read back cannot be (a row the
//! constraints do not admit, a model that does not decode, no row) is a
//! `DecodeError`.

mod described;
mod evolve;
mod registered;
mod rows;

use std::{fmt, num::NonZeroU64, sync::Arc};

use pgorm::pgorm_query::{IntoCondition, SelectStatement, Value};
use pgorm::{DisjointShape, DisjointTable, DisjointValues, Error};
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyList, PyTuple},
};
use pyo3_async_runtimes::tokio::future_into_py;

use crate::{
    entities::{PyActiveModel, backend::EntityBackend},
    errors::{ConstructionError, DecodeError, InternalError},
    execution::Executor,
    expressions::Compiled,
    statements::{PySelect, condition},
    transactions::work::{Output, Work},
    values::PyValue,
};

pub(crate) use described::table_of;
pub(crate) use registered::{Registered, disjoint_entities, disjoint_entity};

/// How a disjoint read's rows become records: as `Record`s a model views by
/// field, or as the registered entities' own models.
pub(crate) enum Codec {
    Models,
    Entities {
        base: Arc<dyn EntityBackend>,
        kinds: Vec<Option<Arc<dyn EntityBackend>>>,
    },
}

/// One disjoint model or registration: its shape, its codec, and its
/// description.
pub(crate) struct Core {
    pub(crate) shape: DisjointShape,
    pub(crate) codec: Codec,
    pub(crate) description: serde_json::Value,
    /// The registered enum's type, which no second registration may name.
    pub(crate) rust: Option<std::any::TypeId>,
}

/// A disjoint model or a registered disjoint enum, as Python's `Disjoint`
/// and `DisjointEntity` hold it.
// [spec:pgorm:req:bindings.disjoint+1]
#[pyclass(name = "_Disjoint", module = "pgorm._native", frozen)]
pub struct NativeDisjoint {
    pub(crate) core: Arc<Core>,
}

impl fmt::Debug for NativeDisjoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeDisjoint")
            .field("description", &self.core.description)
            .finish()
    }
}

/// `query` as a statement a terminal sends, held to PostgreSQL's parameter
/// limit.
fn compiled(query: &SelectStatement) -> PyResult<Compiled> {
    let (sql, values) = query.build();
    if values.0.len() > 65535 {
        return Err(ConstructionError::new_err(
            "statement exceeds PostgreSQL's 65535-parameter limit",
        ));
    }
    Ok(Compiled { sql, values })
}

/// Which terminal a fetch is.
#[derive(Clone, Copy)]
enum Fetch {
    All,
    One,
    Write,
    Count,
}

impl NativeDisjoint {
    /// The stream decoder that reads each row as a fetch does.
    pub(crate) fn row_decode(&self) -> crate::results::RowDecode {
        let core = self.core.clone();
        Arc::new(move |row| {
            let item = core.decode(row)?;
            Python::attach(|py| item.into_python(py))
        })
    }

    /// What a write gives `table`'s columns: a model's `(column, Value)`
    /// pairs, each `Set`, or a registration's `ActiveModel` of `entity`.
    fn values(
        &self,
        table: &DisjointTable,
        entity: Option<&Arc<dyn EntityBackend>>,
        given: &Bound<'_, PyAny>,
    ) -> PyResult<DisjointValues> {
        match entity {
            None => {
                let pairs: Vec<(String, PyValue)> = given.extract().map_err(|_| {
                    PyTypeError::new_err("a model's values are (column, Value) pairs")
                })?;
                table
                    .values(
                        pairs
                            .into_iter()
                            .map(|(column, value)| (column, pgorm::ActiveValue::Set(value.inner))),
                    )
                    .map_err(|error| ConstructionError::new_err(error.to_string()))
            }
            Some(entity) => {
                let active = given.extract::<PyRef<'_, PyActiveModel>>().map_err(|_| {
                    PyTypeError::new_err("a registered disjoint entity writes an ActiveModel")
                })?;
                if active.inner.info().name != entity.info().name {
                    return Err(ConstructionError::new_err(format!(
                        "the ActiveModel is `{}`'s, not `{}`'s",
                        active.inner.info().name,
                        entity.info().name
                    )));
                }
                let pairs = entity
                    .info()
                    .columns
                    .iter()
                    .map(|column| {
                        active
                            .inner
                            .get(&column.name)
                            .map(|value| (column.name.clone(), value))
                    })
                    .collect::<Result<Vec<_>, Error>>()
                    .map_err(|error| ConstructionError::new_err(error.to_string()))?;
                table
                    .values(pairs)
                    .map_err(|error| ConstructionError::new_err(error.to_string()))
            }
        }
    }

    /// The base's values and the kind row's, the kind row's `None` for a
    /// kind without a table given none, and empty for one with a table given
    /// none.
    fn written(
        &self,
        base: &Bound<'_, PyAny>,
        kind: &str,
        row: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(DisjointValues, Option<DisjointValues>)> {
        let shape = &self.core.shape;
        let (index, entry) = shape
            .kind(kind)
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        let (base_entity, kind_entity) = match &self.core.codec {
            Codec::Models => (None, None),
            Codec::Entities { base, kinds } => (Some(base), kinds[index].as_ref()),
        };
        let base = self.values(shape.base(), base_entity, base)?;
        let row = match (entry.table(), row.filter(|row| !row.is_none())) {
            (Some(table), Some(row)) => Some(self.values(table, kind_entity, row)?),
            (Some(table), None) => Some(
                table
                    .values(Vec::<(String, _)>::new())
                    .map_err(|error| ConstructionError::new_err(error.to_string()))?,
            ),
            (None, Some(_)) => {
                return Err(ConstructionError::new_err(format!(
                    "the kind `{kind}` has no table, so a write of it gives no row's values"
                )));
            }
            (None, None) => None,
        };
        Ok((base, row))
    }

    /// Values for the base's columns `columns`, in order: a model's already
    /// converted, a registration's converted through each column's declared
    /// kind.
    fn base_values(&self, columns: &[String], given: &Bound<'_, PyAny>) -> PyResult<Vec<Value>> {
        let given: Vec<Bound<'_, PyAny>> = given
            .extract()
            .map_err(|_| PyTypeError::new_err("values are given as a list"))?;
        if given.len() != columns.len() {
            return Err(ConstructionError::new_err(format!(
                "{} value(s) given for {} column(s)",
                given.len(),
                columns.len()
            )));
        }
        columns
            .iter()
            .zip(given)
            .map(|(column, value)| match &self.core.codec {
                Codec::Models => value
                    .extract::<PyRef<'_, PyValue>>()
                    .map(|value| value.inner.clone())
                    .map_err(|_| PyTypeError::new_err("a model's values are Values")),
                Codec::Entities { base, .. } => base
                    .info()
                    .column(column)?
                    .input
                    .coerce(&value)
                    .map(|value| value.inner),
            })
            .collect()
    }

    fn fetch<'py>(
        &self,
        py: Python<'py>,
        db: &Bound<'_, PyAny>,
        query: SelectStatement,
        fetch: Fetch,
    ) -> PyResult<Bound<'py, PyAny>> {
        let target = Executor::extract(py, db)?;
        let compiled = compiled(&query)?;
        let core = self.core.clone();
        future_into_py(py, async move {
            let Output::Rows(rows) = target.run(Work::Fetch(compiled)).await? else {
                return Err(InternalError::new_err("unexpected disjoint read result"));
            };
            match fetch {
                Fetch::Count => {
                    let row = rows
                        .into_iter()
                        .next()
                        .ok_or_else(|| InternalError::new_err("a count reads one row"))?;
                    let count: i64 = row
                        .try_get("num_items")
                        .map_err(|error| DecodeError::new_err(error.to_string()))?;
                    Python::attach(|py| {
                        count
                            .into_pyobject(py)
                            .map(|count| count.into_any().unbind())
                    })
                    .map_err(Into::into)
                }
                Fetch::All => {
                    let items = rows
                        .into_iter()
                        .map(|row| core.decode(row))
                        .collect::<PyResult<Vec<_>>>()?;
                    Python::attach(|py| {
                        let items = items
                            .into_iter()
                            .map(|item| item.into_python(py))
                            .collect::<PyResult<Vec<_>>>()?;
                        Ok(PyList::new(py, items)?.into_any().unbind())
                    })
                }
                Fetch::One | Fetch::Write => {
                    let row = rows
                        .into_iter()
                        .next()
                        .ok_or_else(|| DecodeError::new_err(Error::RecordNotFound.to_string()))?;
                    let item = core.decode(row)?;
                    Python::attach(|py| item.into_python(py))
                }
            }
        })
    }
}

#[pymethods]
impl NativeDisjoint {
    /// The model or registration as data.
    // [spec:pgorm:req:bindings.disjoint+1]
    fn describe<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        py.import("json")?
            .call_method1("loads", (self.core.description.to_string(),))
    }

    /// The kinds' values, in order.
    fn kinds(&self) -> Vec<String> {
        self.core
            .shape
            .kinds()
            .iter()
            .map(|kind| kind.value().to_owned())
            .collect()
    }

    /// The SQL names of the base's key columns, in key order: what a
    /// registration's key names.
    fn key_columns(&self) -> Vec<String> {
        self.core
            .shape
            .base()
            .key()
            .map(|column| column.name().to_string())
            .collect()
    }

    /// The names a read gives the columns of the decoded source `source`,
    /// the base being 0, in its table's column order.
    fn result_names(&self, source: usize) -> PyResult<Vec<String>> {
        self.core
            .shape
            .result_names(source)
            .ok_or_else(|| ConstructionError::new_err("no such decoded source"))
    }

    /// Every base row with its kind's row: the shape's read.
    // [spec:pgorm:req:bindings.disjoint+1]
    fn find(&self) -> PySelect {
        PySelect {
            inner: self.core.shape.find(),
        }
    }

    /// `select` narrowed to the base row of the key `key` gives in key
    /// order, as the shape narrows it.
    fn narrow_key(
        &self,
        select: PyRef<'_, PySelect>,
        key: &Bound<'_, PyAny>,
    ) -> PyResult<PySelect> {
        let columns: Vec<String> = self
            .core
            .shape
            .base()
            .key()
            .map(|column| column.name().to_string())
            .collect();
        let key = self.base_values(&columns, key)?;
        let mut next = select.clone();
        next.inner.cond_where(
            self.core
                .shape
                .key_is(key)
                .map_err(|error| ConstructionError::new_err(error.to_string()))?,
        );
        Ok(next)
    }

    /// `select` narrowed to base rows of the kind `kind`.
    fn narrow_kind(&self, select: PyRef<'_, PySelect>, kind: &str) -> PyResult<PySelect> {
        let mut next = select.clone();
        next.inner.and_where(
            self.core
                .shape
                .kind_is(kind)
                .map_err(|error| ConstructionError::new_err(error.to_string()))?,
        );
        Ok(next)
    }

    /// `select` narrowed to base rows of one of `kinds`.
    fn narrow_kinds(&self, select: PyRef<'_, PySelect>, kinds: Vec<String>) -> PyResult<PySelect> {
        let mut next = select.clone();
        next.inner.and_where(
            self.core
                .shape
                .kind_in(&kinds)
                .map_err(|error| ConstructionError::new_err(error.to_string()))?,
        );
        Ok(next)
    }

    /// `select` narrowed to base rows of the kind `kind` whose row passes
    /// `predicate`, the kind part of the condition.
    fn narrow_filter_kind(
        &self,
        select: PyRef<'_, PySelect>,
        kind: &str,
        predicate: &Bound<'_, PyAny>,
    ) -> PyResult<PySelect> {
        let condition = condition(predicate)?.into_condition();
        let mut next = select.clone();
        next.inner.cond_where(
            self.core
                .shape
                .filter_kind(kind, condition)
                .map_err(|error| ConstructionError::new_err(error.to_string()))?,
        );
        Ok(next)
    }

    /// Page `page` of `select` at `size` rows a page, as pgorm's paginator
    /// reads it.
    fn page(&self, select: PyRef<'_, PySelect>, size: u64, page: u64) -> PyResult<PySelect> {
        let size = NonZeroU64::new(size)
            .ok_or_else(|| ConstructionError::new_err("a page holds a row"))?;
        Ok(PySelect {
            inner: pgorm::page_statement(&select.inner, size, page)
                .map_err(|error| ConstructionError::new_err(error.to_string()))?,
        })
    }

    /// The statement pgorm's paginator counts `select`'s rows with.
    fn counted(&self, select: PyRef<'_, PySelect>) -> PyResult<PySelect> {
        Ok(PySelect {
            inner: pgorm::count_statement(&select.inner)
                .map_err(|error| ConstructionError::new_err(error.to_string()))?,
        })
    }

    /// The statement a keyset cursor over `select` ordered by the base's
    /// `columns` reads its window with, and whether its rows come reversed:
    /// pgorm's own `Cursor`, each boundary value under its column's casts.
    #[pyo3(signature = (select, columns, *, after=None, before=None, window=None, descending=false))]
    fn cursor(
        &self,
        select: PyRef<'_, PySelect>,
        columns: Vec<String>,
        after: Option<&Bound<'_, PyAny>>,
        before: Option<&Bound<'_, PyAny>>,
        window: Option<(bool, u64)>,
        descending: bool,
    ) -> PyResult<(PySelect, bool)> {
        let mut cursor = self
            .core
            .shape
            .cursor_by(select.inner.clone(), &columns)
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        if let Some(after) = after {
            cursor.after_with(pgorm::pgorm_query::ValueTuple::from(
                self.base_values(&columns, after)?,
            ));
        }
        if let Some(before) = before {
            cursor.before_with(pgorm::pgorm_query::ValueTuple::from(
                self.base_values(&columns, before)?,
            ));
        }
        match window {
            Some((true, rows)) => cursor.last(rows),
            Some((false, rows)) => cursor.first(rows),
            None => &mut cursor,
        };
        if descending {
            cursor.desc();
        }
        let inner = cursor
            .statement()
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        Ok((PySelect { inner }, cursor.reversed()))
    }

    /// The statement inserting a value of the kind `kind`.
    // [spec:pgorm:req:bindings.disjoint+1]
    #[pyo3(signature = (base, kind, row=None))]
    fn insert(
        &self,
        base: &Bound<'_, PyAny>,
        kind: &str,
        row: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PySelect> {
        let (base, row) = self.written(base, kind, row)?;
        let inner = self
            .core
            .shape
            .insert(&base, kind, row.as_ref())
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        Ok(PySelect { inner })
    }

    /// The statement updating a value of the kind `kind`.
    // [spec:pgorm:req:bindings.disjoint+1]
    #[pyo3(signature = (base, kind, row=None))]
    fn update(
        &self,
        base: &Bound<'_, PyAny>,
        kind: &str,
        row: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PySelect> {
        let (base, row) = self.written(base, kind, row)?;
        let inner = self
            .core
            .shape
            .update(&base, kind, row.as_ref())
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        Ok(PySelect { inner })
    }

    /// The statement changing the kind of the row whose key `key` gives.
    // [spec:pgorm:req:bindings.disjoint+1]
    #[pyo3(signature = (key, kind, row=None))]
    fn change_kind(
        &self,
        key: &Bound<'_, PyAny>,
        kind: &str,
        row: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PySelect> {
        let shape = &self.core.shape;
        let columns: Vec<String> = shape
            .base()
            .key()
            .map(|column| column.name().to_string())
            .collect();
        let key = self.base_values(&columns, key)?;
        let (index, entry) = shape
            .kind(kind)
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        let row = match (entry.table(), row.filter(|row| !row.is_none())) {
            (Some(table), Some(row)) => {
                let entity = match &self.core.codec {
                    Codec::Models => None,
                    Codec::Entities { kinds, .. } => kinds[index].as_ref(),
                };
                Some(self.values(table, entity, row)?)
            }
            (Some(table), None) => Some(
                table
                    .values(Vec::<(String, _)>::new())
                    .map_err(|error| ConstructionError::new_err(error.to_string()))?,
            ),
            (None, Some(_)) => {
                return Err(ConstructionError::new_err(format!(
                    "the kind `{kind}` has no table, so a write of it gives no row's values"
                )));
            }
            (None, None) => None,
        };
        let inner = shape
            .change_kind(key, kind, row.as_ref())
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        Ok(PySelect { inner })
    }

    /// Run a read: every row (`"all"`), the first of a read narrowed to one
    /// row (`"one"`), the one row a write reads back (`"write"`), or the
    /// count of a `counted` statement (`"count"`). No row for `"one"` or
    /// `"write"` is pgorm's `RecordNotFound`, a `DecodeError`.
    // [spec:pgorm:req:bindings.disjoint+1]
    // [spec:pgorm:req:bindings.registered-pools]
    fn run<'py>(
        &self,
        py: Python<'py>,
        db: &Bound<'_, PyAny>,
        select: PyRef<'_, PySelect>,
        terminal: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut query = select.inner.clone();
        let fetch = match terminal {
            "all" => Fetch::All,
            "one" => {
                query.narrow_limit_to_one();
                Fetch::One
            }
            "write" => Fetch::Write,
            "count" => Fetch::Count,
            _ => return Err(PyTypeError::new_err("unknown disjoint terminal")),
        };
        self.fetch(py, db, query, fetch)
    }

    /// The statement `run` sends for `terminal`: a read narrowed to one row
    /// for `"one"`, as sent otherwise.
    #[pyo3(signature = (select, *, terminal="all"))]
    fn inspect(&self, select: PyRef<'_, PySelect>, terminal: &str) -> PyResult<Compiled> {
        let mut query = select.inner.clone();
        match terminal {
            "all" | "write" => {}
            "one" => {
                query.narrow_limit_to_one();
            }
            _ => return Err(PyTypeError::new_err("unknown disjoint terminal")),
        }
        compiled(&query)
    }

    /// The statements creating the kind type, the tables and their
    /// constraints, in the order they run.
    // [spec:pgorm:req:bindings.disjoint+1]
    fn statements<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let statements = self
            .core
            .shape
            .create()
            .map_err(|error| ConstructionError::new_err(error.to_string()))?;
        evolve::ddl_tuple(py, statements)
    }

    /// The kind change `change` names, of the kind `value`, as its steps:
    /// each its statements and its gap, `None` or the gap's kind and the
    /// kind table's schema and name.
    // [spec:pgorm:req:bindings.disjoint-evolve]
    #[pyo3(signature = (change, value, table=None))]
    fn change<'py>(
        &self,
        py: Python<'py>,
        change: &str,
        value: &str,
        table: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyList>> {
        evolve::change(py, &self.core.shape, change, value, table)
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeDisjoint>()?;
    module.add_function(wrap_pyfunction!(described::disjoint_model, module)?)?;
    module.add_function(wrap_pyfunction!(disjoint_entity, module)?)?;
    module.add_function(wrap_pyfunction!(disjoint_entities, module)?)?;
    Ok(())
}

/// Each disjoint operation beside the Rust API that answers it; only
/// `disjoint_entity` needs a registration.
pub(crate) const OPERATIONS: [(&str, &str); 13] = [
    ("disjoint.declare", "pgorm::DisjointShape::new"),
    ("disjoint.find", "pgorm::DisjointShape::find"),
    ("disjoint.find_by_key", "pgorm::DisjointShape::key_is"),
    (
        "disjoint.kind",
        "pgorm::DisjointShape::{kind_is, kind_in, filter_kind}",
    ),
    ("disjoint.records", "pgorm::DisjointShape::judge"),
    (
        "disjoint.paginate",
        "pgorm::{page_statement, count_statement}",
    ),
    (
        "disjoint.cursor",
        "pgorm::DisjointShape::cursor_by / pgorm::Cursor::statement",
    ),
    ("disjoint.insert", "pgorm::DisjointShape::insert"),
    ("disjoint.update", "pgorm::DisjointShape::update"),
    ("disjoint.change_kind", "pgorm::DisjointShape::change_kind"),
    ("disjoint.schema", "pgorm::DisjointShape::create"),
    (
        "disjoint.evolve",
        "pgorm::DisjointShape::{add_kind, add_kind_recreating, remove_kind, add_kind_table, \
         remove_kind_table}",
    ),
    (
        "disjoint_entity",
        "pgorm_python::entities::Registry::disjoint / pgorm::DisjointShape::of",
    ),
];
