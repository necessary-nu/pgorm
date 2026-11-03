//! What `#[derive(OneOf)]` declares: `<Enum>Key` and `<Enum>Model`, the
//! targets and their columns in declaration order, a row's reference read as
//! its key and refused where it sets two columns or none the enum allows,
//! and a key set on an `ActiveModel`, its variant's column set and every
//! other one null.

use pgorm::entity::prelude::*;
use pgorm::tests_cfg::{
    comment::{self, Commentable, CommentableKey, CommentableModel},
    photo, post,
};
use pgorm::{OneOfTarget, TargetVisitor};

/// A comment on a post or a photo, never on neither: `Commentable` without
/// its unit variant.
#[derive(Clone, Copy, Debug, OneOf)]
#[pgorm(entity = comment::Entity)]
pub enum Attached {
    #[pgorm(column = comment::Column::PostId)]
    Post(post::Entity),
    #[pgorm(column = comment::Column::PhotoId)]
    Photo(photo::Entity),
}

fn comment(post_id: Option<i32>, photo_id: Option<i64>) -> comment::Model {
    comment::Model {
        id: 9,
        body: "nice".to_owned(),
        post_id,
        photo_id,
    }
}

/// The target tables a visit reaches, each with its column, in order.
struct Targets(Vec<(String, String)>);

impl<O: OneOf> TargetVisitor<O> for Targets {
    fn target<T: OneOfTarget<O>>(&mut self)
    where
        O::Entity: Related<T>,
    {
        let table = T::default().table_name().to_owned();
        self.0.push((table, T::COLUMN.as_str().to_owned()));
    }
}

// [spec:pgorm:def:disjoint.one-of+1/test]    each variant naming a target reaches its entity and the column holding its key, in declaration order; the enum is named in snake_case, and only an enum with a unit variant has one
#[test]
fn targets_in_declaration_order() {
    let mut targets = Targets(Vec::new());
    Commentable::visit_targets(&mut targets);
    let pair = |table: &str, column: &str| (table.to_owned(), column.to_owned());
    assert_eq!(
        targets.0,
        [pair("post", "post_id"), pair("photo", "photo_id")]
    );
    assert_eq!(
        Commentable::columns()
            .iter()
            .map(|column| column.as_str())
            .collect::<Vec<_>>(),
        ["post_id", "photo_id"]
    );
    assert_eq!(Commentable::name(), "commentable");
    assert_eq!(Attached::name(), "attached");
    assert_eq!(
        Commentable::none(),
        Some((CommentableKey::Detached, CommentableModel::Detached))
    );
    assert!(Attached::none().is_none());
}

// [spec:pgorm:def:disjoint.one-of+1/test]    `<Enum>Key` holds each target's key and `<Enum>Model` its `Model`, the unit variant holding nothing
#[test]
fn generated_enums_hold_keys_and_models() {
    let post = post::Model {
        id: 3,
        title: "hello".to_owned(),
    };
    let photo = photo::Model {
        id: 4,
        url: "a.png".to_owned(),
    };
    assert_eq!(
        <post::Entity as OneOfTarget<Commentable>>::model(post.clone()),
        CommentableModel::Post(post)
    );
    assert_eq!(
        <photo::Entity as OneOfTarget<Commentable>>::model(photo.clone()),
        CommentableModel::Photo(photo)
    );
    let key: CommentableKey = <photo::Entity as OneOfTarget<Commentable>>::key(4_i64);
    assert_eq!(key, CommentableKey::Photo(4));
    assert_eq!(
        <photo::Entity as OneOfTarget<Commentable>>::of_key(&key),
        Some(&4)
    );
    assert_eq!(
        <post::Entity as OneOfTarget<Commentable>>::of_key(&key),
        None
    );
    assert_eq!(
        <post::Entity as OneOfTarget<Commentable>>::of_key(&CommentableKey::Detached),
        None
    );
}

// [spec:pgorm:def:disjoint.one-of+1/test]    a row reads as the variant whose column is set, and as the unit variant where none is
#[test]
fn a_row_reads_as_its_key() -> Result<(), Error> {
    assert_eq!(
        Commentable::key(&comment(Some(3), None))?,
        CommentableKey::Post(3)
    );
    assert_eq!(
        Commentable::key(&comment(None, Some(4)))?,
        CommentableKey::Photo(4)
    );
    assert_eq!(
        Commentable::key(&comment(None, None))?,
        CommentableKey::Detached
    );
    assert_eq!(
        Attached::key(&comment(None, Some(4)))?,
        AttachedKey::Photo(4)
    );
    Ok(())
}

// [spec:pgorm:def:disjoint.one-of+1/test]    a row with two columns set, or none where the enum has no unit variant, is an error naming the table, the key and the columns, never read as one of them
#[test]
fn a_row_against_its_declaration_is_refused() {
    let both = Commentable::key(&comment(Some(3), Some(4)));
    let Err(Error::Query(message)) = both else {
        panic!("two columns set read as {both:?}");
    };
    assert_eq!(
        message.to_string(),
        "the `comment` row whose key is (`id` = 9) references more than one target, setting \
         `post_id`, `photo_id`, where at most one of `post_id`, `photo_id` is set"
    );
    let neither = Attached::key(&comment(None, None));
    let Err(Error::Query(message)) = neither else {
        panic!("no column set read as {neither:?}");
    };
    assert_eq!(
        message.to_string(),
        "the `comment` row whose key is (`id` = 9) references no target, setting none of \
         `post_id`, `photo_id`, where exactly one is set"
    );
}

// [spec:pgorm:def:disjoint.one-of+1/test]    setting a key sets its variant's column and every other one to null, the unit variant every one
#[test]
fn setting_a_key_nulls_the_other_columns() -> Result<(), Error> {
    let mut active = comment(Some(3), None).into_active();
    Commentable::set(&mut active, CommentableKey::Photo(4))?;
    assert_eq!(active.post_id, ActiveValue::Set(None));
    assert_eq!(active.photo_id, ActiveValue::Set(Some(4)));
    assert_eq!(active.body, ActiveValue::Unchanged("nice".to_owned()));

    Commentable::set(&mut active, CommentableKey::Post(5))?;
    assert_eq!(active.post_id, ActiveValue::Set(Some(5)));
    assert_eq!(active.photo_id, ActiveValue::Set(None));

    Commentable::set(&mut active, CommentableKey::Detached)?;
    assert_eq!(active.post_id, ActiveValue::Set(None));
    assert_eq!(active.photo_id, ActiveValue::Set(None));

    let mut fresh = <comment::ActiveModel as Default>::default();
    Attached::set(&mut fresh, AttachedKey::Post(6))?;
    assert_eq!(fresh.post_id, ActiveValue::Set(Some(6)));
    assert_eq!(fresh.photo_id, ActiveValue::Set(None));
    Ok(())
}
