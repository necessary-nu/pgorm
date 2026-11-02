#![allow(unused_imports, dead_code)]

pub mod common;
pub use common::{TestContext, features::*, setup::*};
use pgorm::entity::prelude::*;

#[derive(SqlName)]
pub enum ClassName {
    Table,
    Id,
    Title,
    Text,
}

#[derive(SqlName)]
pub enum Book {
    #[iden = "book_table"]
    Table,
    Id,
    #[iden = "turtle"]
    Title,
    #[iden = "TeXt"]
    Text,
    #[iden = "ty_pe"]
    Type,
}

#[derive(SqlName)]
struct GlyphToken;

#[derive(SqlName)]
#[iden = "weRd"]
struct Word;

// Every rendered name here is a "valid iden", so the `prepare` override is
// emitted and quotes the name itself.
#[derive(SqlName)]
pub enum AllValid {
    Table,
    Id,
}

// One variant renders a name that is *not* a valid iden, which suppresses the
// `prepare` override for the whole enum and leaves the trait default (which
// doubles any embedded double quote) to do the quoting.
#[derive(SqlName)]
pub enum SomeInvalid {
    Table,
    #[iden = "we\"ird"]
    Weird,
}

// An empty enum expands to nothing, which is the only reason this hand-written
// `SqlName` impl compiles rather than colliding with a generated one.
#[derive(SqlName)]
pub enum EmptyIden {}

impl SqlName for EmptyIden {
    fn unquoted(&self, _s: &mut dyn std::fmt::Write) {
        match *self {}
    }
}

// [spec:pgorm:sem:macros.derive.iden+3/test]    unit structs, enums, `Table`, and the `iden` override
#[test]
fn main() -> Result<(), Error> {
    assert_eq!(ClassName::Table.to_string(), "class_name");
    assert_eq!(ClassName::Id.to_string(), "id");
    assert_eq!(ClassName::Title.to_string(), "title");
    assert_eq!(ClassName::Text.to_string(), "text");

    assert_eq!(Book::Id.to_string(), "id");
    assert_eq!(Book::Table.to_string(), "book_table");
    assert_eq!(Book::Title.to_string(), "turtle");
    assert_eq!(Book::Text.to_string(), "TeXt");
    assert_eq!(Book::Type.to_string(), "ty_pe");

    assert_eq!(GlyphToken.to_string(), "glyph_token");

    assert_eq!(Word.to_string(), "weRd");
    Ok(())
}

// [spec:pgorm:sem:macros.derive.iden+3/test]    `prepare` is emitted only for statically-valid idens
#[test]
fn prepare_override_is_conditional() {
    // Emitted: the name is written between the quotes verbatim.
    let mut s = String::new();
    AllValid::Table.prepare(&mut s);
    assert_eq!(s, "\"all_valid\"");

    // Not emitted: the trait default is used, which doubles an embedded
    // double quote. `AllValid` never shows that behaviour, `SomeInvalid` does.
    let mut s = String::new();
    SomeInvalid::Weird.prepare(&mut s);
    assert_eq!(s, "\"we\"\"ird\"");

    // The suppression is per-enum, not per-variant.
    let mut s = String::new();
    SomeInvalid::Table.prepare(&mut s);
    assert_eq!(s, "\"some_invalid\"");

    let mut s = String::new();
    AllValid::Id.prepare(&mut s);
    assert_eq!(s, "\"id\"");
}

// [spec:pgorm:sem:macros.derive.iden+3/test]    an empty enum expands to nothing
#[test]
fn empty_enum_expands_to_nothing() {
    fn assert_iden<T: SqlName>() {}
    assert_iden::<EmptyIden>();
}
