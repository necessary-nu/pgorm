use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "note")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub body: String,
    pub issue_id: Option<i32>,
    pub release_id: Option<i32>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = super::issue::Entity,
        from = Column::IssueId,
        to = super::issue::Column::Id
    )]
    Issue,
    #[pgorm(
        belongs_to = super::release::Entity,
        from = Column::ReleaseId,
        to = super::release::Column::Id
    )]
    Release,
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, OneOf)]
#[pgorm(entity = Entity)]
pub enum TheReferenceANoteMakesToTheIssueOrReleaseItIsOn {
    #[pgorm(column = Column::IssueId)]
    Issue(super::issue::Entity),
    #[pgorm(column = Column::ReleaseId)]
    Release(super::release::Entity),
}
