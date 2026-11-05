use std::{error, fmt, io};

// [spec:pgorm:def:codegen.entity+9]
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    StdIoError(io::Error),
    TransformError(String),
}

impl Error {
    /// Qualify a transform failure with the table it was found in, so a
    /// conversion that knows only its own column or foreign key still names
    /// the table once it reaches the transform gate.
    pub(crate) fn in_table(self, table: &str) -> Self {
        match self {
            Self::TransformError(msg) => Self::TransformError(format!("table `{table}` {msg}")),
            other => other,
        }
    }
}

/// The message itself: a `TransformError`'s text as written, and an
/// `StdIoError`'s as the I/O error displays it.
// [spec:pgorm:def:codegen.entity+9]
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::StdIoError(error) => error.fmt(f),
            Self::TransformError(message) => f.write_str(message),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::StdIoError(e) => Some(e),
            Self::TransformError(_) => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(io_err: io::Error) -> Self {
        Self::StdIoError(io_err)
    }
}
