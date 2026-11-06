//! A registered entity's insert, the statement and the terminal a binding
//! chose at run time, run through pgorm's own [`Insert`] and [`TryInsert`].
//!
//! A binding holds an entity's insert as data until a terminal runs it: its
//! ActiveModels, whether it is Rust's `Insert<A, OneRow>` or
//! `Insert<A, ManyRows>`, whether `on_conflict_do_nothing` or
//! `on_empty_do_nothing` made it a `TryInsert`, its conflict clause, and the
//! terminal. [`insert`] builds that statement for the entity and runs that
//! terminal, so both bindings answer as Rust's insert does because Rust's
//! insert answers.

use pgorm::{
    ActiveModelTrait, ConnectionTrait, EntityTrait, Error, Insert, IntoActiveModel, TryInsert,
    TryInsertResult, Upserted,
    pgorm_query::{IntoValueTuple, OnConflict, ValueTuple},
};

/// An insert of ActiveModels `A` as a binding holds it, ready for its
/// terminal.
#[derive(Clone, Debug)]
pub struct Insertion<A> {
    /// The ActiveModels, in the order the insert writes them.
    pub models: Vec<A>,
    /// `Insert::one` of the one model rather than `Insert::many` of them all.
    pub one: bool,
    /// The statement and the terminal that runs it.
    pub write: InsertWrite,
}

/// Which statement an insert is, its conflict clause, and the terminal that
/// runs it.
#[derive(Clone, Debug)]
pub enum InsertWrite {
    /// An `Insert`, with `Insert::on_conflict`'s clause if it has one.
    Insert(Option<Box<OnConflict>>, InsertTerminal),
    /// A `TryInsert`, which answers [`TryInsertResult`], with the clause
    /// `on_conflict_do_nothing` writes or one `on_conflict` gave, if any.
    Try(Option<InsertConflict>, TryInsertTerminal),
}

/// A try-insert's conflict clause.
#[derive(Clone, Debug)]
pub enum InsertConflict {
    /// `Insert::on_conflict_do_nothing`'s: `DO NOTHING` on the primary key's
    /// columns, or on any key for an entity with none.
    PrimaryKey,
    /// A clause `on_conflict` gave, which stands in place of the key's `DO
    /// NOTHING` when it came after `on_conflict_do_nothing`.
    Clause(Box<OnConflict>),
}

/// An `Insert`'s terminals, each the row count's own: `exec`,
/// `exec_returning_pk(s)`, `exec_returning_model(s)` and
/// `exec_returning_upsert(s)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertTerminal {
    /// `exec`.
    Count,
    /// `exec_returning_pk` or `exec_returning_pks`.
    Keys,
    /// `exec_returning_model` or `exec_returning_models`.
    Models,
    /// `exec_returning_upsert` or `exec_returning_upserts`.
    Upserts,
}

/// A `TryInsert`'s terminals, which are an `Insert`'s but the upserts': Rust's
/// `TryInsert` has no upsert terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TryInsertTerminal {
    /// `exec`.
    Count,
    /// `exec_returning_pk` or `exec_returning_pks`.
    Keys,
    /// `exec_returning_model` or `exec_returning_models`.
    Models,
}

/// What an insert's terminal gave: the rows written, each written row's key
/// (its values in the primary key's column order) or model `M`, or what an
/// upsert did with each row it wrote, `U`.
#[derive(Debug)]
pub enum Inserted<M, U = Upserted<M>> {
    /// The rows written.
    Count(u64),
    /// Each written row's key.
    Keys(Vec<ValueTuple>),
    /// Each written row.
    Models(Vec<M>),
    /// What the upsert did with each row it wrote.
    Upserts(Vec<U>),
}

impl<A> Insertion<A> {
    /// The same insert of the ActiveModels `model` reads each of these as:
    /// a binding's handles as the entity's own ActiveModels.
    pub fn try_map<B, F>(self, model: impl FnMut(&A) -> Result<B, F>) -> Result<Insertion<B>, F> {
        Ok(Insertion {
            models: self.models.iter().map(model).collect::<Result<_, _>>()?,
            one: self.one,
            write: self.write,
        })
    }
}

/// A `TryInsert`'s answer, what it wrote read by `read`.
fn tried<T, M, U>(
    result: TryInsertResult<T>,
    read: impl FnOnce(T) -> Inserted<M, U>,
) -> TryInsertResult<Inserted<M, U>> {
    match result {
        TryInsertResult::Inserted(written) => TryInsertResult::Inserted(read(written)),
        TryInsertResult::Conflicted => TryInsertResult::Conflicted,
        TryInsertResult::Empty => TryInsertResult::Empty,
    }
}

/// `insert` as the `TryInsert` the binding's calls made of it: by
/// `on_conflict_do_nothing`, or by `on_empty_do_nothing` and then the clause
/// `on_conflict` gave it, the last clause given standing as it does in Rust.
fn attempt<A: ActiveModelTrait, R>(
    insert: Insert<A, R>,
    conflict: Option<InsertConflict>,
) -> TryInsert<A, R> {
    match conflict {
        Some(InsertConflict::PrimaryKey) => insert.on_conflict_do_nothing(),
        Some(InsertConflict::Clause(clause)) => insert.on_empty_do_nothing().on_conflict(*clause),
        None => insert.on_empty_do_nothing(),
    }
}

/// `insert`, with `conflict` as its clause if given.
fn clause<A: ActiveModelTrait, R>(
    mut insert: Insert<A, R>,
    conflict: Option<Box<OnConflict>>,
) -> Insert<A, R> {
    if let Some(conflict) = conflict {
        insert = insert.on_conflict(*conflict);
    }
    insert
}

/// Run `insertion`'s statement into `entity`'s table on `db` through its
/// terminal, each written row read by `model` and each upsert by `upsert`. A
/// try-insert answers as Rust's `TryInsertResult` does; an `Insert` always
/// with `Inserted`, what its terminal gave, a row its conflict clause held
/// back being Rust's error from `exec_returning_pk` and
/// `exec_returning_model` and left out of the batch terminals' answers.
// [spec:pgorm:req:bindings.entity-inserts]
// [spec:pgorm:req:bindings.entity-try-inserts]
pub async fn insert<E, C, M, U>(
    entity: E,
    db: &C,
    insertion: Insertion<E::ActiveModel>,
    model: impl Fn(E::Model) -> M,
    upsert: impl Fn(Upserted<E::Model>) -> U,
) -> Result<TryInsertResult<Inserted<M, U>>, Error>
where
    E: EntityTrait,
    E::Model: IntoActiveModel<E::ActiveModel>,
    C: ConnectionTrait,
{
    let rows = |written: Vec<E::Model>| Inserted::Models(written.into_iter().map(&model).collect());
    let Insertion { models, one, write } = insertion;
    if one {
        let active = models
            .into_iter()
            .next()
            .ok_or_else(|| Error::Type("an insert of one row needs one ActiveModel".to_owned()))?;
        let insert = Insert::one_in(entity, active);
        return Ok(match write {
            InsertWrite::Insert(conflict, terminal) => {
                let insert = clause(insert, conflict);
                TryInsertResult::Inserted(match terminal {
                    InsertTerminal::Count => Inserted::Count(insert.exec(db).await?),
                    InsertTerminal::Keys => {
                        Inserted::Keys(vec![insert.exec_returning_pk(db).await?.into_value_tuple()])
                    }
                    InsertTerminal::Models => rows(vec![insert.exec_returning_model(db).await?]),
                    InsertTerminal::Upserts => Inserted::Upserts(
                        insert
                            .exec_returning_upsert(db)
                            .await?
                            .into_iter()
                            .map(&upsert)
                            .collect(),
                    ),
                })
            }
            InsertWrite::Try(conflict, terminal) => {
                let insert = attempt(insert, conflict);
                match terminal {
                    TryInsertTerminal::Count => tried(insert.exec(db).await?, Inserted::Count),
                    TryInsertTerminal::Keys => tried(insert.exec_returning_pk(db).await?, |pk| {
                        Inserted::Keys(vec![pk.into_value_tuple()])
                    }),
                    TryInsertTerminal::Models => {
                        tried(insert.exec_returning_model(db).await?, |written| {
                            rows(vec![written])
                        })
                    }
                }
            }
        });
    }
    let insert = Insert::many_in(entity, models);
    Ok(match write {
        InsertWrite::Insert(conflict, terminal) => {
            let insert = clause(insert, conflict);
            TryInsertResult::Inserted(match terminal {
                InsertTerminal::Count => Inserted::Count(insert.exec(db).await?),
                InsertTerminal::Keys => Inserted::Keys(
                    insert
                        .exec_returning_pks(db)
                        .await?
                        .into_iter()
                        .map(IntoValueTuple::into_value_tuple)
                        .collect(),
                ),
                InsertTerminal::Models => rows(insert.exec_returning_models(db).await?),
                InsertTerminal::Upserts => Inserted::Upserts(
                    insert
                        .exec_returning_upserts(db)
                        .await?
                        .into_iter()
                        .map(&upsert)
                        .collect(),
                ),
            })
        }
        InsertWrite::Try(conflict, terminal) => {
            let insert = attempt(insert, conflict);
            match terminal {
                TryInsertTerminal::Count => tried(insert.exec(db).await?, Inserted::Count),
                TryInsertTerminal::Keys => tried(insert.exec_returning_pks(db).await?, |keys| {
                    Inserted::Keys(
                        keys.into_iter()
                            .map(IntoValueTuple::into_value_tuple)
                            .collect(),
                    )
                }),
                TryInsertTerminal::Models => tried(insert.exec_returning_models(db).await?, rows),
            }
        }
    })
}
