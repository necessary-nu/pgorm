//! The files pgorm-codegen generates from `schema.sql`, whose one-of
//! references are named past PostgreSQL's 63 bytes as `ck-{table}-{enum}`:
//! a table's name too long for the remark's, an enum's name of 59 bytes, the
//! most the derive takes, for the note's. `Schema::create_one_of` names each
//! `ck-{enum}`, and codegen names the enum for it whole.

pub mod issue;
pub mod note;
pub mod release;
pub mod remark_left_on_an_issue_or_a_release_in_the_project_archive;
