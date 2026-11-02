use pgorm::entity::prelude::*;
use serde::{Deserialize, Serialize};

use super::pgorm_range_types::Floatmultirange;
use super::pgorm_range_types::Floatrange;
use super::pgorm_range_types::Slot;
use super::pgorm_range_types::SlotMultirange;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[pgorm(table_name = "ledger")]
pub struct Model {
    #[pgorm(primary_key, auto_increment = false)]
    pub id: i32,
    pub span: Interval,
    pub maybe_span: Option<Interval>,
    #[pgorm(column_type = ColumnType::Interval(IntervalSpec::Fields(PgInterval::YearToMonth)))]
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

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
