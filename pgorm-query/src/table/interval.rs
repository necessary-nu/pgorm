//! The `interval` type's field and precision vocabulary.
//!
//! PostgreSQL's `interval` is the one type in the column vocabulary whose
//! spelling is itself a small grammar — a field qualifier, optionally a
//! fractional-seconds precision, and a rule about where the precision may
//! appear. That grammar is a type detail rather than part of building a column
//! definition, so it lives beside [`ColumnType::Interval`] rather than inside
//! the builder.
//!
//! [`ColumnType::Interval`]: crate::ColumnType::Interval

/// The `[fields] [(p)]` tail of a PostgreSQL `interval` type.
///
/// PostgreSQL takes a precision only where the trailing field is `SECOND`, so
/// the precision sits on the second-bearing field spellings and on the
/// unqualified form, and `interval HOUR(3)` has no spelling here.
// [spec:pgorm:def:sql.types.column-type+11]
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum IntervalSpec {
    /// `interval`, or `interval(p)` — every field, with a fractional-seconds
    /// precision when one is given.
    Any(Option<IntervalPrecision>),
    /// `interval <fields>` — one of the field-qualified forms.
    Fields(PgInterval),
}

/// Fractional-seconds precision of an interval type.
///
/// PostgreSQL accepts 0 through 6; a wider precision has no spelling.
// [spec:pgorm:def:sql.types.column-type+11]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum IntervalPrecision {
    P0,
    P1,
    P2,
    P3,
    P4,
    P5,
    P6,
}

impl IntervalPrecision {
    /// The precision `digits` names, or `None` where PostgreSQL has none.
    pub const fn new(digits: u8) -> Option<Self> {
        match digits {
            0 => Some(Self::P0),
            1 => Some(Self::P1),
            2 => Some(Self::P2),
            3 => Some(Self::P3),
            4 => Some(Self::P4),
            5 => Some(Self::P5),
            6 => Some(Self::P6),
            _ => None,
        }
    }

    /// The digit count this precision spells.
    pub const fn digits(self) -> u8 {
        match self {
            Self::P0 => 0,
            Self::P1 => 1,
            Self::P2 => 2,
            Self::P3 => 3,
            Self::P4 => 4,
            Self::P5 => 5,
            Self::P6 => 6,
        }
    }
}

impl std::fmt::Display for IntervalPrecision {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.digits())
    }
}

/// All interval field qualifiers; the second-bearing ones carry the precision
/// PostgreSQL allows only there.
// [spec:pgorm:def:sql.types.column-type+11]
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PgInterval {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second(Option<IntervalPrecision>),
    YearToMonth,
    DayToHour,
    DayToMinute,
    DayToSecond(Option<IntervalPrecision>),
    HourToMinute,
    HourToSecond(Option<IntervalPrecision>),
    MinuteToSecond(Option<IntervalPrecision>),
}

impl PgInterval {
    /// The fields with the fractional-seconds `precision`, or `None` when the
    /// fields end before seconds, where PostgreSQL takes no precision:
    /// `HOUR TO SECOND` takes one, `HOUR TO MINUTE` only none.
    ///
    /// ```
    /// use pgorm_query::{IntervalPrecision, PgInterval};
    ///
    /// let three = IntervalPrecision::new(3);
    /// assert_eq!(PgInterval::HourToSecond(None).with_precision(three), Some(PgInterval::HourToSecond(three)));
    /// assert_eq!(PgInterval::HourToMinute.with_precision(three), None);
    /// assert_eq!(PgInterval::HourToMinute.with_precision(None), Some(PgInterval::HourToMinute));
    /// ```
    // [spec:pgorm:def:sql.types.column-type+11]
    pub fn with_precision(self, precision: Option<IntervalPrecision>) -> Option<Self> {
        Some(match self {
            Self::Second(_) => Self::Second(precision),
            Self::DayToSecond(_) => Self::DayToSecond(precision),
            Self::HourToSecond(_) => Self::HourToSecond(precision),
            Self::MinuteToSecond(_) => Self::MinuteToSecond(precision),
            fields if precision.is_none() => fields,
            _ => return None,
        })
    }
}
