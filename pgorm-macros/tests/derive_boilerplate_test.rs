//! What the derives write so an entity's module holds only what the program
//! decides: the behaviour an entity with no hooks has, `Related` for each
//! entity one relation reaches, and an `ActiveEnum`'s values and type name.

use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]
pub enum BrewKind {
    EverydayTea,
    BreakfastTea,
    #[pgorm(string_value = "Earl Grey")]
    EarlGrey,
}

#[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]
#[pgorm(
    rename_all = "camelCase",
    enum_name = "steep_time",
    schema_name = "kitchen"
)]
pub enum Steep {
    ShortSteep,
    #[pgorm(rename = "SCREAMING_SNAKE_CASE")]
    LongSteep,
}

#[derive(Clone, Debug, PartialEq, Eq, ActiveEnum)]
#[pgorm(rs_type = i32, db_type = ColumnType::Integer)]
pub enum Strength {
    Weak = 1,
    #[pgorm(num_value = 5)]
    Strong,
}

pub mod author {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "author")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
        pub brew: super::BrewKind,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(has_many = super::post::Entity)]
        Post,
    }
}

/// An author is reached from a post twice, so the post names the one it means,
/// and its tags through the junction, beside the one it leads with.
pub mod post {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "post")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub author_id: i32,
        pub editor_id: i32,
        pub lead_tag_id: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::author::Entity,
            from = Column::AuthorId,
            to = super::author::Column::Id
        )]
        Author,
        #[pgorm(
            belongs_to = super::author::Entity,
            from = Column::EditorId,
            to = super::author::Column::Id
        )]
        Editor,
        #[pgorm(
            belongs_to = super::tag::Entity,
            from = Column::LeadTagId,
            to = super::tag::Column::Id,
            related = false
        )]
        LeadTag,
        #[pgorm(has_many = super::tagging::Entity)]
        Tagging,
    }

    impl Related<super::author::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Editor.def()
        }
    }

    impl Related<super::tag::Entity> for Entity {
        fn to() -> RelationDef {
            super::tagging::Relation::Tag.def()
        }

        fn via() -> Option<RelationDef> {
            Some(super::tagging::Relation::Post.def().rev())
        }
    }
}

pub mod tag {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "tag")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

pub mod tagging {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "tagging")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub post_id: i32,
        #[pgorm(primary_key, auto_increment = false)]
        pub tag_id: i32,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::post::Entity,
            from = Column::PostId,
            to = super::post::Column::Id
        )]
        Post,
        #[pgorm(
            belongs_to = super::tag::Entity,
            from = Column::TagId,
            to = super::tag::Column::Id
        )]
        Tag,
    }
}

/// An entity whose behaviour is the program's: a new draft starts untitled.
pub mod draft {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "draft", hooks)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub title: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {
        fn new() -> Self {
            Self {
                title: set("untitled"),
                ..ActiveModelTrait::default()
            }
        }
    }
}

fn spelled(def: RelationDef) -> String {
    format!("{def:?}")
}

// [spec:pgorm:sem:macros.derive.boilerplate/test]    `Related<F>` for an entity one variant reaches is that variant's definition
#[test]
fn related_is_the_one_variant_reaching_the_entity() {
    assert_eq!(
        spelled(<author::Entity as Related<post::Entity>>::to()),
        spelled(author::Relation::Post.def())
    );
    assert!(<author::Entity as Related<post::Entity>>::via().is_none());
    assert_eq!(
        spelled(<post::Entity as Related<tagging::Entity>>::to()),
        spelled(post::Relation::Tagging.def())
    );
    assert_eq!(
        spelled(<tagging::Entity as Related<tag::Entity>>::to()),
        spelled(tagging::Relation::Tag.def())
    );
}

// [spec:pgorm:sem:macros.derive.boilerplate/test]    two variants reaching one entity get no `Related`, and `related = false` leaves a junction's to the program
#[test]
fn program_writes_related_where_the_derive_cannot_choose() {
    assert_eq!(
        spelled(<post::Entity as Related<author::Entity>>::to()),
        spelled(post::Relation::Editor.def())
    );
    assert_eq!(
        spelled(<post::Entity as Related<tag::Entity>>::to()),
        spelled(tagging::Relation::Tag.def())
    );
    assert!(<post::Entity as Related<tag::Entity>>::via().is_some());
}

// [spec:pgorm:sem:macros.derive.boilerplate/test]    the behaviour with no hooks, and the program's under `hooks`
#[test]
fn behaviour_is_derived_unless_the_model_says_hooks() {
    let author = author::ActiveModel::new();
    assert!(author.name.is_not_set());
    assert_eq!(author, <author::ActiveModel as Default>::default());

    let draft = <draft::ActiveModel as Default>::default();
    assert_eq!(draft.title, set("untitled"));
    assert!(draft.id.is_not_set());
}

// [spec:pgorm:sem:macros.derive.boilerplate/test]    an enum with neither `rs_type` nor `db_type` is a PostgreSQL enum of its variants' names in snake_case
#[test]
fn name_enum_is_a_postgres_enum_by_default() {
    assert_eq!(BrewKind::name().to_string(), "brew_kind");
    assert_eq!(
        BrewKind::values(),
        ["everyday_tea", "breakfast_tea", "Earl Grey"]
    );
    assert_eq!(BrewKind::EverydayTea.to_value(), "everyday_tea");
    assert_eq!(
        BrewKind::try_from_value(&"breakfast_tea".to_owned()),
        Ok(BrewKind::BreakfastTea)
    );
    match BrewKind::db_type().get_column_type() {
        ColumnType::Enum {
            name,
            schema,
            variants,
        } => {
            assert_eq!(name.to_string(), "brew_kind");
            assert!(schema.is_none());
            assert_eq!(
                variants
                    .iter()
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>(),
                ["everyday_tea", "breakfast_tea", "Earl Grey"]
            );
        }
        other => panic!("expected a PostgreSQL enum, got {other:?}"),
    }
    assert!(matches!(
        author::Column::Brew.def().get_column_type(),
        ColumnType::Enum { .. }
    ));
}

// [spec:pgorm:sem:macros.derive.boilerplate/test]    `rename_all`, `rename` and `enum_name` in place of the defaults, and an enum stored otherwise naming both keys and falling back to discriminants
#[test]
fn an_enum_says_what_differs_from_the_defaults() {
    assert_eq!(Steep::name().to_string(), "steep_time");
    assert_eq!(Steep::values(), ["shortSteep", "LONG_STEEP"]);
    match Steep::db_type().get_column_type() {
        ColumnType::Enum { schema, .. } => {
            assert_eq!(
                schema.as_ref().map(|schema| schema.to_string()).as_deref(),
                Some("kitchen")
            );
        }
        other => panic!("expected a PostgreSQL enum, got {other:?}"),
    }

    assert_eq!(Strength::values(), [1, 5]);
    assert_eq!(Strength::db_type(), ColumnType::Integer.def());
    assert_eq!(Strength::name().to_string(), "strength");
}
