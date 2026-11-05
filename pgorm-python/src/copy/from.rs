//! `COPY … FROM STDIN`, its format chosen by name at construction.

use std::num::NonZeroU32;

use pgorm::pgorm_query::{
    CopyBinary, CopyCsv, CopyFromStatement, CopyIgnore, CopyLogVerbosity, CopyText, TableName,
    Values,
};
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyDict, PyInt, PyTuple},
};

use super::{Format, character, names, table_name, text};
use crate::{
    errors::ConstructionError,
    expressions::Compiled,
    identifiers::Identifier,
    statements::{PyTable, condition},
};

/// A COPY FROM in one of the three formats.
#[derive(Clone, Debug)]
pub(crate) enum CopyFrom {
    Text(CopyFromStatement<CopyText>),
    Csv(CopyFromStatement<CopyCsv>),
    Binary(CopyFromStatement<CopyBinary>),
}

impl CopyFrom {
    fn new(table: TableName, format: Format) -> Self {
        match format {
            Format::Text => Self::Text(CopyFromStatement::new(table, CopyText)),
            Format::Csv => Self::Csv(CopyFromStatement::new(table, CopyCsv)),
            Format::Binary => Self::Binary(CopyFromStatement::new(table, CopyBinary)),
        }
    }

    fn format(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::Csv(_) => "csv",
            Self::Binary(_) => "binary",
        }
    }

    pub(crate) fn sql(&self) -> String {
        match self {
            Self::Text(statement) => statement.to_string(),
            Self::Csv(statement) => statement.to_string(),
            Self::Binary(statement) => statement.to_string(),
        }
    }
}

// [spec:pgorm:req:bindings.copy]
/// `COPY table [(columns)] FROM STDIN` in a format chosen at construction.
#[pyclass(name = "CopyFrom", module = "pgorm", frozen, generic, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyCopyFrom {
    pub(crate) inner: CopyFrom,
    columns: Vec<String>,
}

#[pymethods]
impl PyCopyFrom {
    #[new]
    fn new(table: PyRef<'_, PyTable>, format: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: CopyFrom::new(table_name(&table)?, super::format(format)?),
            columns: Vec::new(),
        })
    }

    #[getter]
    fn format(&self) -> &'static str {
        self.inner.format()
    }

    /// Add to the columns each row supplies; a column left out takes its
    /// default.
    #[pyo3(signature = (*columns))]
    fn columns(&self, columns: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let columns = names(columns)?;
        if columns.is_empty() {
            return Err(ConstructionError::new_err(
                "columns requires at least one column",
            ));
        }
        let mut next = self.clone();
        for column in &columns {
            if next.columns.contains(&column.name) {
                return Err(ConstructionError::new_err("duplicate COPY column"));
            }
            next.columns.push(column.name.clone());
        }
        let columns: Vec<_> = columns.iter().map(Identifier::name).collect();
        every!(CopyFrom, &mut next.inner, |s| s.columns(columns));
        Ok(next)
    }

    fn r#where(&self, predicate: &Bound<'_, PyAny>) -> PyResult<Self> {
        let predicate = condition(predicate)?;
        let mut next = self.clone();
        every!(CopyFrom, &mut next.inner, |s| s.cond_where(predicate));
        Ok(next)
    }

    fn freeze(&self) -> Self {
        let mut next = self.clone();
        every!(CopyFrom, &mut next.inner, |s| s.freeze());
        next
    }

    fn delimiter(&self, delimiter: &Bound<'_, PyAny>) -> PyResult<Self> {
        let delimiter = character(delimiter, "DELIMITER")?;
        let mut next = self.clone();
        delimited!(CopyFrom, &mut next.inner, "DELIMITER", |s| s
            .delimiter(delimiter))?;
        Ok(next)
    }

    fn null_string(&self, null: &Bound<'_, PyAny>) -> PyResult<Self> {
        let null = text(null, "NULL")?;
        let mut next = self.clone();
        delimited!(CopyFrom, &mut next.inner, "NULL", |s| s.null_string(null))?;
        Ok(next)
    }

    fn default_string(&self, default: &Bound<'_, PyAny>) -> PyResult<Self> {
        let default = text(default, "DEFAULT")?;
        let mut next = self.clone();
        delimited!(CopyFrom, &mut next.inner, "DEFAULT", |s| s
            .default_string(default))?;
        Ok(next)
    }

    fn header(&self) -> PyResult<Self> {
        let mut next = self.clone();
        delimited!(CopyFrom, &mut next.inner, "HEADER", |s| s.header())?;
        Ok(next)
    }

    fn header_match(&self) -> PyResult<Self> {
        let mut next = self.clone();
        delimited!(CopyFrom, &mut next.inner, "HEADER match", |s| s
            .header_match())?;
        Ok(next)
    }

    fn encoding(&self, encoding: &Bound<'_, PyAny>) -> PyResult<Self> {
        let encoding = text(encoding, "ENCODING")?;
        let mut next = self.clone();
        delimited!(CopyFrom, &mut next.inner, "ENCODING", |s| s
            .encoding(encoding))?;
        Ok(next)
    }

    #[pyo3(signature = (reject_limit=None, log_verbosity=None))]
    fn on_error_ignore(
        &self,
        reject_limit: Option<&Bound<'_, PyAny>>,
        log_verbosity: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let reject_limit = reject_limit
            .filter(|limit| !limit.is_none())
            .map(|limit| {
                if !limit.is_exact_instance_of::<PyInt>() {
                    return Err(PyTypeError::new_err("reject_limit is an int"));
                }
                limit
                    .extract::<u32>()
                    .ok()
                    .and_then(NonZeroU32::new)
                    .ok_or_else(|| {
                        ConstructionError::new_err(
                            "reject_limit requires a positive integer that fits u32",
                        )
                    })
            })
            .transpose()?;
        let log_verbosity = log_verbosity
            .filter(|verbosity| !verbosity.is_none())
            .map(|verbosity| match verbosity.extract::<String>().as_deref() {
                Ok("verbose") => Ok(CopyLogVerbosity::Verbose),
                Ok("silent") => Ok(CopyLogVerbosity::Silent),
                _ => Err(PyTypeError::new_err(
                    "log_verbosity must be \"verbose\" or \"silent\"",
                )),
            })
            .transpose()?;
        let ignore = CopyIgnore {
            reject_limit,
            log_verbosity,
        };
        let mut next = self.clone();
        delimited!(CopyFrom, &mut next.inner, "ON_ERROR", |s| s
            .on_error_ignore(ignore))?;
        Ok(next)
    }

    fn quote(&self, quote: &Bound<'_, PyAny>) -> PyResult<Self> {
        let quote = character(quote, "QUOTE")?;
        let mut next = self.clone();
        csv_only!(CopyFrom, &mut next.inner, "QUOTE", |s| s.quote(quote))?;
        Ok(next)
    }

    fn escape(&self, escape: &Bound<'_, PyAny>) -> PyResult<Self> {
        let escape = character(escape, "ESCAPE")?;
        let mut next = self.clone();
        csv_only!(CopyFrom, &mut next.inner, "ESCAPE", |s| s.escape(escape))?;
        Ok(next)
    }

    /// An empty list forces nothing, as in Rust.
    #[pyo3(signature = (*columns))]
    fn force_not_null(&self, columns: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let columns: Vec<_> = names(columns)?.iter().map(Identifier::name).collect();
        let mut next = self.clone();
        csv_only!(CopyFrom, &mut next.inner, "FORCE_NOT_NULL", |s| s
            .force_not_null(columns))?;
        Ok(next)
    }

    fn force_not_null_all(&self) -> PyResult<Self> {
        let mut next = self.clone();
        csv_only!(CopyFrom, &mut next.inner, "FORCE_NOT_NULL", |s| s
            .force_not_null_all())?;
        Ok(next)
    }

    #[pyo3(signature = (*columns))]
    fn force_null(&self, columns: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let columns: Vec<_> = names(columns)?.iter().map(Identifier::name).collect();
        let mut next = self.clone();
        csv_only!(CopyFrom, &mut next.inner, "FORCE_NULL", |s| s
            .force_null(columns))?;
        Ok(next)
    }

    fn force_null_all(&self) -> PyResult<Self> {
        let mut next = self.clone();
        csv_only!(CopyFrom, &mut next.inner, "FORCE_NULL", |s| s
            .force_null_all())?;
        Ok(next)
    }

    #[pyo3(signature = (*_args, **_kwargs))]
    fn force_quote(
        &self,
        _args: &Bound<'_, PyTuple>,
        _kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        Err(ConstructionError::new_err(
            "FORCE_QUOTE is accepted only by COPY TO",
        ))
    }

    #[pyo3(signature = (*_args, **_kwargs))]
    fn force_quote_all(
        &self,
        _args: &Bound<'_, PyTuple>,
        _kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        Err(ConstructionError::new_err(
            "FORCE_QUOTE is accepted only by COPY TO",
        ))
    }

    /// COPY takes no bind parameters: Rust renders every value inline.
    fn inspect(&self) -> Compiled {
        Compiled {
            sql: self.inner.sql(),
            values: Values(Vec::new()),
        }
    }

    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL statements cannot be tested as Python booleans",
        ))
    }
    fn __repr__(&self) -> String {
        format!("CopyFrom(format={:?})", self.inner.format())
    }
}
