//! `DDL`, the class every complete DDL statement is, over the pgorm-query
//! statement it owns. A builder that takes further clauses is a subclass
//! holding nothing of its own: its statement is the base's, so `inspect()`
//! and every terminal read each the same way.

use crate::{errors::ConstructionError, expressions::Compiled};
use pgorm::pgorm_query::{
    ColumnRenameStatement, CommentStatement, ConstraintRenameStatement, DomainAlterStatement,
    DomainCreateStatement, DomainDropStatement, IndexCreateStatement, IndexDropStatement,
    MaterializedViewCreateStatement, MaterializedViewDropStatement,
    MaterializedViewRefreshStatement, RecursiveViewCreateStatement, SchemaCreateStatement,
    SchemaDropStatement, SchemaRenameStatement, SequenceAlterStatement, SequenceCreateStatement,
    SequenceDropStatement, SequenceRenameStatement, TableAlterStatement, TableCreateAsStatement,
    TableCreateStatement, TableDropStatement, TableName, TableRenameStatement,
    TableTruncateStatement, Values, ViewCreateStatement, ViewDropStatement,
    extension::{
        AttributeRenameStatement, CompositeAlterStatement, ExtensionCreateStatement,
        ExtensionDropStatement, TypeAlterStatement, TypeCreateStatement, TypeDropStatement,
    },
};
use pyo3::{PyClass, prelude::*, pyclass_init::PyClassInitializer};
use std::fmt;

/// An `ALTER TABLE` with the table it names kept beside it, for the foreign
/// keys a later action adds, which name it.
#[derive(Clone, Debug)]
pub(crate) struct AlteredTable {
    pub(crate) table: TableName,
    pub(crate) statement: TableAlterStatement,
}

impl fmt::Display for AlteredTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.statement.fmt(f)
    }
}

/// The statements, each rendered by pgorm-query's own `Display`, which
/// writes every value it carries as an escaped literal.
macro_rules! statements {
    ($($variant:ident($kind:ty)),* $(,)?) => {
        /// Owned native DDL states; no textual reconstruction or deferred
        /// Python calls.
        #[derive(Clone, Debug)]
        pub(crate) enum Statement {
            $($variant($kind)),*
        }

        impl Statement {
            // [spec:pgorm:req:bindings.schema]
            fn sql(&self) -> String {
                match self {
                    $(Self::$variant(statement) => statement.to_string()),*
                }
            }
        }
    };
}

// The four statements over a query are boxed, the query being several times
// the size of any other statement here.
statements! {
    CreateTable(TableCreateStatement),
    AlterTable(AlteredTable),
    DropTable(TableDropStatement),
    RenameTable(TableRenameStatement),
    RenameColumn(ColumnRenameStatement),
    RenameConstraint(ConstraintRenameStatement),
    Truncate(TableTruncateStatement),
    Comment(CommentStatement),
    CreateIndex(IndexCreateStatement),
    DropIndex(IndexDropStatement),
    CreateType(TypeCreateStatement),
    AlterType(TypeAlterStatement),
    AlterComposite(CompositeAlterStatement),
    RenameAttribute(AttributeRenameStatement),
    DropType(TypeDropStatement),
    CreateDomain(DomainCreateStatement),
    AlterDomain(DomainAlterStatement),
    DropDomain(DomainDropStatement),
    CreateSequence(SequenceCreateStatement),
    AlterSequence(SequenceAlterStatement),
    DropSequence(SequenceDropStatement),
    RenameSequence(SequenceRenameStatement),
    CreateExtension(ExtensionCreateStatement),
    DropExtension(ExtensionDropStatement),
    CreateView(Box<ViewCreateStatement>),
    CreateRecursiveView(Box<RecursiveViewCreateStatement>),
    DropView(ViewDropStatement),
    CreateMaterializedView(Box<MaterializedViewCreateStatement>),
    RefreshMaterializedView(MaterializedViewRefreshStatement),
    DropMaterializedView(MaterializedViewDropStatement),
    CreateTableAs(Box<TableCreateAsStatement>),
    CreateSchema(SchemaCreateStatement),
    DropSchema(SchemaDropStatement),
    RenameSchema(SchemaRenameStatement),
}

/// A complete DDL statement. PostgreSQL takes no parameters in DDL, so it
/// inspects and runs as SQL text with no values beside it.
#[derive(Debug)]
#[pyclass(name = "DDL", module = "pgorm.schema", frozen, subclass)]
pub struct PyDDL {
    pub(crate) inner: Statement,
}

#[pymethods]
impl PyDDL {
    // [spec:pgorm:req:bindings.schema]
    pub fn inspect(&self) -> Compiled {
        Compiled {
            sql: self.inner.sql(),
            values: Values(Vec::new()),
        }
    }

    // [spec:pgorm:req:python.truth-testing+4]
    fn __bool__(&self) -> PyResult<bool> {
        Err(ConstructionError::new_err(
            "SQL statements cannot be tested as Python booleans",
        ))
    }
}

impl PyDDL {
    pub(crate) fn new(inner: Statement) -> Self {
        Self { inner }
    }
}

pub(super) fn subclass<T>(py: Python<'_>, inner: Statement, class: T) -> PyResult<Bound<'_, T>>
where
    T: PyClass<BaseType = PyDDL>,
{
    Bound::new(
        py,
        PyClassInitializer::from(PyDDL::new(inner)).add_subclass(class),
    )
}

/// The refusal of a subclass whose base holds another kind of statement,
/// which only the subclass's own factory and methods make, so it cannot
/// happen.
pub(super) fn mismatch(class: &str) -> PyErr {
    ConstructionError::new_err(format!("a {class} holds a statement of another kind"))
}

/// A DDL subclass over one `Statement` variant: `statement` reads a copy of
/// what its base holds and `object` makes one over a statement; with `with`
/// named, `with` gives a new object over a changed copy.
macro_rules! ddl_class {
    ($class:ident, $variant:ident, $kind:ty, with) => {
        $crate::schema::statement::ddl_class!($class, $variant, $kind);

        impl $class {
            fn with<'py>(
                slf: &Bound<'py, Self>,
                change: impl FnOnce(&mut $kind),
            ) -> PyResult<Bound<'py, Self>> {
                let mut statement = Self::statement(slf)?;
                change(&mut statement);
                Self::object(slf.py(), statement)
            }
        }
    };
    ($class:ident, $variant:ident, $kind:ty) => {
        impl $class {
            pub(crate) fn statement(slf: &Bound<'_, Self>) -> PyResult<$kind> {
                match &slf.as_super().get().inner {
                    $crate::schema::statement::Statement::$variant(statement) => {
                        Ok(statement.clone())
                    }
                    _ => Err($crate::schema::statement::mismatch(stringify!($variant))),
                }
            }

            pub(crate) fn object(py: Python<'_>, statement: $kind) -> PyResult<Bound<'_, Self>> {
                $crate::schema::statement::subclass(
                    py,
                    $crate::schema::statement::Statement::$variant(statement),
                    Self,
                )
            }
        }
    };
}
pub(super) use ddl_class;
