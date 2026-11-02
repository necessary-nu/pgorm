mod period_outside_the_key {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "period_outside_the_key")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(without_overlaps)]
        pub valid_at: Range<Date>,
    }
}

mod period_before_the_end {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "period_before_the_end")]
    pub struct Model {
        #[pgorm(primary_key, without_overlaps)]
        pub valid_at: Range<Date>,
        #[pgorm(primary_key)]
        pub id: i32,
    }
}

mod period_alone {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "period_alone")]
    pub struct Model {
        #[pgorm(primary_key, without_overlaps)]
        pub valid_at: Range<Date>,
        pub rate: i32,
    }
}

fn main() {}
