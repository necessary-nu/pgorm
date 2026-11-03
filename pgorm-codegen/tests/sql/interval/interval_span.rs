use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "interval_span")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub plain: Interval,
    pub maybe: Option<Interval>,
    #[pgorm(column_type = ColumnType::Interval(IntervalSpec::Any(Some(IntervalPrecision::P3))))]
    pub precise: Interval,
    #[pgorm(column_type = ColumnType::Interval(IntervalSpec::Fields(PgInterval::YearToMonth)))]
    pub months: Interval,
    #[pgorm(column_type = ColumnType::Interval(IntervalSpec::Fields(PgInterval::Day)))]
    pub days: Interval,
    #[pgorm(
        column_type = ColumnType::Interval(IntervalSpec::Fields(PgInterval::DayToSecond(Some(IntervalPrecision::P3)))),
        nullable
    )]
    pub clock: Option<Interval>,
    #[pgorm(column_type = ColumnType::Interval(IntervalSpec::Fields(PgInterval::HourToMinute)))]
    pub minutes: Interval,
    #[pgorm(
        column_type = ColumnType::Interval(IntervalSpec::Fields(PgInterval::Second(Some(IntervalPrecision::P0))))
    )]
    pub seconds: Interval,
    pub history: Vec<Interval>,
    #[pgorm(
        column_type = ColumnType::Array(Arc::new(ColumnType::Interval(IntervalSpec::Fields(PgInterval::YearToMonth)))),
        nullable
    )]
    pub budgets: Option<Vec<Interval>>,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
