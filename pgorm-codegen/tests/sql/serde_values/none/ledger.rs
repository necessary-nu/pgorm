use pgorm::entity::prelude::*;

use super::pgorm_range_types::{Floatmultirange, Floatrange, Slot, SlotMultirange};

#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "ledger")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub span: Interval,
    pub maybe_span: Option<Interval>,
    #[pgorm(
        column_type = ColumnType::Interval(IntervalSpec::Fields(PgInterval::YearToMonth))
    )]
    pub months: Interval,
    pub spans: Vec<Interval>,
    pub ints: Range<i32>,
    pub bigs: Option<Range<i64>>,
    pub nums: Range<Decimal>,
    pub days: Range<Date>,
    pub stamps: Range<DateTime>,
    pub instants: Option<Range<DateTimeWithTimeZone>>,
    pub int_set: Multirange<i32>,
    pub big_set: Option<Multirange<i64>>,
    pub num_set: Multirange<Decimal>,
    pub day_set: Multirange<Date>,
    pub stamp_set: Multirange<DateTime>,
    pub instant_set: Option<Multirange<DateTimeWithTimeZone>>,
    pub int_ranges: Vec<Range<i32>>,
    pub day_sets: Option<Vec<Multirange<Date>>>,
    pub float_set: Option<Floatmultirange>,
    pub floats: Floatrange,
    pub slot: Option<Slot>,
    pub slots: SlotMultirange,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}
