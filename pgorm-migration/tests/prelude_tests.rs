//! The two preludes glob-imported into one module: every derive either exports
//! resolves to one macro, so a migration crate can import both and derive
//! anything.

#![allow(dead_code)]

mod both_preludes {
    use pgorm::entity::prelude::*;
    use pgorm_migration::prelude::*;

    #[derive(MigrationName)]
    pub struct Migration;

    #[derive(SqlName)]
    pub enum Post {
        Table,
        Id,
        #[iden = "headline"]
        Title,
    }

    #[derive(Clone, Copy, Debug, StaticName)]
    pub enum Shelf {
        Table,
        Top,
    }

    #[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]
    #[pgorm(enum_name = "tea", display)]
    pub enum Tea {
        #[pgorm(string_value = "everyday", display_value = "Everyday")]
        Everyday,
    }

    pub mod cake {
        use pgorm::entity::prelude::*;
        use pgorm_migration::prelude::*;

        #[derive(Clone, Debug, PartialEq, Entity)]
        #[pgorm(table_name = "cake")]
        pub struct Model {
            #[pgorm(primary_key)]
            pub id: i32,
            pub tea: super::Tea,
        }

        #[derive(Copy, Clone, Debug, Relation)]
        pub enum Relation {}
    }
}

// [spec:pgorm:req:macros.derive.names/test]    the entity and migration preludes share no macro name between two macros
#[test]
fn both_preludes_derive_without_ambiguity() {
    use pgorm::{EntityName, SqlName, StaticName};
    use pgorm_migration::MigrationName;

    let file = std::path::Path::new(file!());
    assert_eq!(
        both_preludes::Migration.name(),
        file.file_stem()
            .and_then(|stem| stem.to_str())
            .expect("a file stem")
    );
    assert_eq!(both_preludes::Post::Table.to_string(), "post");
    assert_eq!(both_preludes::Post::Title.to_string(), "headline");
    assert_eq!(both_preludes::Shelf::Top.as_str(), "top");
    assert_eq!(both_preludes::Tea::Everyday.to_string(), "Everyday");
    assert_eq!(both_preludes::cake::Entity.table_name(), "cake");
}
