//! `COPY … TO STDOUT`, its format chosen by name at construction.

use pgorm::pgorm_query::{
    CopyBinary, CopyCsv, CopySource, CopyText, CopyToStatement, IntoCopySource, Values,
};
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyDict, PyTuple},
};

use super::{Format, character, names, table_name, text};
use crate::{
    errors::ConstructionError,
    expressions::Compiled,
    identifiers::Identifier,
    statements::{PyDelete, PyInsert, PyMerge, PyPendingMerge, PySelect, PyTable, PyUpdate},
};

/// A COPY TO in one of the three formats.
#[derive(Clone, Debug)]
pub(crate) enum CopyTo {
    Text(CopyToStatement<CopyText>),
    Csv(CopyToStatement<CopyCsv>),
    Binary(CopyToStatement<CopyBinary>),
}

impl CopyTo {
    fn new<S: IntoCopySource>(source: S, format: Format) -> Self {
        match format {
            Format::Text => Self::Text(CopyToStatement::new(source, CopyText)),
            Format::Csv => Self::Csv(CopyToStatement::new(source, CopyCsv)),
            Format::Binary => Self::Binary(CopyToStatement::new(source, CopyBinary)),
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

/// A write as a COPY TO source, refused as Rust refuses one with no RETURNING
/// list.
fn write_source<W>(write: W) -> PyResult<CopySource>
where
    CopySource: TryFrom<W, Error = pgorm::pgorm_query::error::Error>,
{
    CopySource::try_from(write).map_err(|error| ConstructionError::new_err(error.to_string()))
}

// [spec:pgorm:req:bindings.copy]
/// `COPY { table | (query) } TO STDOUT` in a format chosen at construction.
#[pyclass(name = "CopyTo", module = "pgorm", frozen, generic, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyCopyTo {
    pub(crate) inner: CopyTo,
}

#[pymethods]
impl PyCopyTo {
    /// A table, or a statement whose rows are copied; a write is a source
    /// only with a RETURNING list, refused here without one as Rust's
    /// conversion into a `CopySource` refuses it.
    #[new]
    fn new(source: &Bound<'_, PyAny>, format: &Bound<'_, PyAny>) -> PyResult<Self> {
        let format = super::format(format)?;
        let inner = if let Ok(table) = source.extract::<PyRef<'_, PyTable>>() {
            CopyTo::new(table_name(&table)?, format)
        } else if let Ok(query) = source.extract::<PyRef<'_, PySelect>>() {
            query.inspect()?;
            CopyTo::new(query.inner.clone(), format)
        } else if let Ok(query) = source.extract::<PyRef<'_, PyInsert>>() {
            query.inspect()?;
            CopyTo::new(write_source(query.inner.clone())?, format)
        } else if let Ok(query) = source.extract::<PyRef<'_, PyUpdate>>() {
            query.inspect()?;
            CopyTo::new(write_source(query.inner.clone())?, format)
        } else if let Ok(query) = source.extract::<PyRef<'_, PyDelete>>() {
            query.inspect()?;
            CopyTo::new(write_source(query.inner.clone())?, format)
        } else if let Ok(query) = source.extract::<PyRef<'_, PyMerge>>() {
            query.inspect()?;
            CopyTo::new(write_source(query.inner.clone())?, format)
        } else if source.extract::<PyRef<'_, PyPendingMerge>>().is_ok() {
            return Err(ConstructionError::new_err(
                "a MERGE needs a WHEN arm before it can be copied",
            ));
        } else {
            return Err(PyTypeError::new_err(
                "COPY TO reads a Table or a Select, Insert, Update, Delete or Merge statement",
            ));
        };
        Ok(Self { inner })
    }

    #[getter]
    fn format(&self) -> &'static str {
        self.inner.format()
    }

    fn delimiter(&self, delimiter: &Bound<'_, PyAny>) -> PyResult<Self> {
        let delimiter = character(delimiter, "DELIMITER")?;
        let mut next = self.clone();
        delimited!(CopyTo, &mut next.inner, "DELIMITER", |s| s
            .delimiter(delimiter))?;
        Ok(next)
    }

    fn null_string(&self, null: &Bound<'_, PyAny>) -> PyResult<Self> {
        let null = text(null, "NULL")?;
        let mut next = self.clone();
        delimited!(CopyTo, &mut next.inner, "NULL", |s| s.null_string(null))?;
        Ok(next)
    }

    fn header(&self) -> PyResult<Self> {
        let mut next = self.clone();
        delimited!(CopyTo, &mut next.inner, "HEADER", |s| s.header())?;
        Ok(next)
    }

    fn encoding(&self, encoding: &Bound<'_, PyAny>) -> PyResult<Self> {
        let encoding = text(encoding, "ENCODING")?;
        let mut next = self.clone();
        delimited!(CopyTo, &mut next.inner, "ENCODING", |s| s
            .encoding(encoding))?;
        Ok(next)
    }

    fn quote(&self, quote: &Bound<'_, PyAny>) -> PyResult<Self> {
        let quote = character(quote, "QUOTE")?;
        let mut next = self.clone();
        csv_only!(CopyTo, &mut next.inner, "QUOTE", |s| s.quote(quote))?;
        Ok(next)
    }

    fn escape(&self, escape: &Bound<'_, PyAny>) -> PyResult<Self> {
        let escape = character(escape, "ESCAPE")?;
        let mut next = self.clone();
        csv_only!(CopyTo, &mut next.inner, "ESCAPE", |s| s.escape(escape))?;
        Ok(next)
    }

    /// An empty list forces nothing, as in Rust.
    #[pyo3(signature = (*columns))]
    fn force_quote(&self, columns: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let columns: Vec<_> = names(columns)?.iter().map(Identifier::name).collect();
        let mut next = self.clone();
        csv_only!(CopyTo, &mut next.inner, "FORCE_QUOTE", |s| s
            .force_quote(columns))?;
        Ok(next)
    }

    fn force_quote_all(&self) -> PyResult<Self> {
        let mut next = self.clone();
        csv_only!(CopyTo, &mut next.inner, "FORCE_QUOTE", |s| s
            .force_quote_all())?;
        Ok(next)
    }

    #[pyo3(signature = (*_args, **_kwargs))]
    fn columns(
        &self,
        _args: &Bound<'_, PyTuple>,
        _kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        Err(ConstructionError::new_err(
            "COPY TO takes no column list; copy a Select of the columns instead",
        ))
    }

    #[pyo3(signature = (*_args, **_kwargs))]
    fn r#where(
        &self,
        _args: &Bound<'_, PyTuple>,
        _kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        Err(ConstructionError::new_err(
            "COPY TO takes no WHERE; copy a Select with the condition instead",
        ))
    }

    #[pyo3(signature = (*_args, **_kwargs))]
    fn freeze(
        &self,
        _args: &Bound<'_, PyTuple>,
        _kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        Err(ConstructionError::new_err(
            "FREEZE is accepted only by COPY FROM",
        ))
    }

    #[pyo3(signature = (*_args, **_kwargs))]
    fn default_string(
        &self,
        _args: &Bound<'_, PyTuple>,
        _kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        Err(ConstructionError::new_err(
            "DEFAULT is accepted only by COPY FROM",
        ))
    }

    #[pyo3(signature = (*_args, **_kwargs))]
    fn header_match(
        &self,
        _args: &Bound<'_, PyTuple>,
        _kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        Err(ConstructionError::new_err(
            "HEADER match is accepted only by COPY FROM",
        ))
    }

    #[pyo3(signature = (*_args, **_kwargs))]
    fn on_error_ignore(
        &self,
        _args: &Bound<'_, PyTuple>,
        _kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        Err(ConstructionError::new_err(
            "ON_ERROR is accepted only by COPY FROM",
        ))
    }

    #[pyo3(signature = (*_args, **_kwargs))]
    fn force_not_null(
        &self,
        _args: &Bound<'_, PyTuple>,
        _kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        Err(ConstructionError::new_err(
            "FORCE_NOT_NULL is accepted only by COPY FROM",
        ))
    }

    #[pyo3(signature = (*_args, **_kwargs))]
    fn force_not_null_all(
        &self,
        _args: &Bound<'_, PyTuple>,
        _kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        Err(ConstructionError::new_err(
            "FORCE_NOT_NULL is accepted only by COPY FROM",
        ))
    }

    #[pyo3(signature = (*_args, **_kwargs))]
    fn force_null(
        &self,
        _args: &Bound<'_, PyTuple>,
        _kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        Err(ConstructionError::new_err(
            "FORCE_NULL is accepted only by COPY FROM",
        ))
    }

    #[pyo3(signature = (*_args, **_kwargs))]
    fn force_null_all(
        &self,
        _args: &Bound<'_, PyTuple>,
        _kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        Err(ConstructionError::new_err(
            "FORCE_NULL is accepted only by COPY FROM",
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
        format!("CopyTo(format={:?})", self.inner.format())
    }
}
