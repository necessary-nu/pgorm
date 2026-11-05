use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity, Eq)]
#[pgorm(table_name = "issue")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub title: String,
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {
    #[pgorm(has_many = super::note::Entity)]
    Note,
    #[pgorm(has_many = super::remark_left_on_an_issue_or_a_release_in_the_project_archive::Entity)]
    RemarkLeftOnAnIssueOrAReleaseInTheProjectArchive,
}
