"""Borrows and brands: the contracts whose whole content is a compile refusal.

A transaction exists only inside the closure `transaction` runs, and its
`DatabaseTransaction` borrows the connection exclusively; `Binder` brands the
expressions it mints with a lifetime the pipeline invents per call. Neither
contract can be observed at runtime: a program that violates one never exists
as a binary, and a program that respects one is indistinguishable from a
program that had no such contract. The evidence is the rejection itself.
"""

from .compile_case import CompileCase, rejects
from .compile_entities import entity_module

OBLIGATION = "rust-ownership"

CONN_PRELUDE = (
    "    use pgorm::{Connection, DatabaseConnection, DatabasePool, "
    "DatabaseTransaction, Error, TransactionError, Transact};"
)

PIPELINE_ENTITIES = "\n\n".join(
    [
        "    pub mod left {\n"
        + "\n".join(
            "    " + line if line else line
            for line in entity_module(
                table="brand_left", columns=(("label", "String", ""),)
            ).splitlines()
        )
        + "\n    }",
        "    pub mod right {\n"
        + "\n".join(
            "    " + line if line else line
            for line in entity_module(
                table="brand_right", columns=(("weight", "i64", ""),)
            ).splitlines()
        )
        + "\n    }",
    ]
)


def _transaction(identity, body, *, verdict, phase, expects=None, note=""):
    return CompileCase(
        id=identity,
        obligation=OBLIGATION,
        verdict=verdict,
        phase=phase,
        source=CONN_PRELUDE + "\n\n" + body,
        expects=expects,
        note=note,
    )


def transaction_accept_cases():
    """The uses the borrow permits, so a rejection cannot be read as blanket."""
    commit = (
        "    pub async fn run(conn: &mut DatabaseConnection)\n"
        "        -> Result<(), TransactionError<Error>> {\n"
        '        conn.transaction(async |tx| tx.batch_execute("SELECT 1").await)\n'
        "            .await\n    }"
    )
    sequential = (
        "    pub async fn run(conn: &mut DatabaseConnection)\n"
        "        -> Result<(), TransactionError<Error>> {\n"
        '        conn.transaction(async |tx| tx.batch_execute("SELECT 1").await)\n'
        "            .await?;\n"
        '        conn.transaction(async |tx| tx.batch_execute("SELECT 2").await)\n'
        "            .await\n    }"
    )
    savepoint = (
        "    pub async fn run(conn: &mut DatabaseConnection)\n"
        "        -> Result<(), TransactionError<TransactionError<Error>>> {\n"
        "        conn.transaction(async |tx| {\n"
        '            tx.transaction(async |inner| inner.batch_execute("SELECT 1").await)\n'
        "                .await\n"
        "        })\n"
        "        .await\n    }"
    )
    reborrow = (
        "    pub async fn run(conn: &mut DatabaseConnection) -> Result<(), Error> {\n"
        '        let ended = conn.transaction(async |tx| tx.batch_execute("SELECT 1").await);\n'
        "        ended.await.map_err(|error| Error::Custom(error.to_string()))?;\n"
        '        conn.batch_execute("SELECT 2").await\n    }'
    )
    spawned = (
        "    fn spawnable<F: std::future::Future + Send + 'static>(future: F) -> F {\n"
        "        future\n    }\n\n"
        "    pub fn run(pool: DatabasePool) {\n"
        "        let _ = spawnable(async move {\n"
        "            let mut conn = pool.get().await?;\n"
        "            conn.transaction(async |tx: &mut DatabaseTransaction<'_>| {\n"
        '                tx.transaction(async |inner| inner.batch_execute("SELECT 1").await)\n'
        "                    .await\n"
        "            })\n"
        "            .await\n"
        "            .map_err(|error| Error::Custom(error.to_string()))\n"
        "        });\n    }"
    )
    return [
        _transaction(
            "txn-commit",
            commit,
            verdict="accept",
            phase="borrowck",
            note="one transaction, ended by what its closure returns",
        ),
        _transaction(
            "txn-sequential",
            sequential,
            verdict="accept",
            phase="borrowck",
            note="the borrow ends when the closure's transaction does",
        ),
        _transaction(
            "txn-savepoint",
            savepoint,
            verdict="accept",
            phase="borrowck",
            note="a nested savepoint borrows the transaction, not the connection",
        ),
        _transaction(
            "txn-after-commit",
            reborrow,
            verdict="accept",
            phase="borrowck",
            note="the connection is usable again once the transaction is gone",
        ),
        _transaction(
            "txn-spawned",
            spawned,
            verdict="accept",
            phase="borrowck",
            note="a transaction with a savepoint inside is a Send future",
        ),
    ]


def transaction_reject_cases():
    """Each way the borrow can be broken, and the code that names the break."""
    variants = (
        (
            "txn-no-commit",
            (
                "    pub async fn run(conn: &mut DatabaseConnection)\n"
                "        -> Result<(), TransactionError<Error>> {\n"
                "        conn.transaction(async |tx| {\n"
                "            tx.commit().await?;\n"
                "            Ok::<_, Error>(())\n"
                "        })\n"
                "        .await\n    }"
            ),
            rejects("E0599"),
            "what the closure returns is the only commit: the handle has none",
            "typeck",
        ),
        (
            "txn-conn-while-open",
            (
                "    pub async fn run(conn: &mut DatabaseConnection)\n"
                "        -> Result<(), TransactionError<Error>> {\n"
                "        conn.transaction(async |tx| {\n"
                '            conn.batch_execute("SELECT 1").await?;\n'
                '            tx.batch_execute("SELECT 2").await\n'
                "        })\n"
                "        .await\n    }"
            ),
            rejects("E0502"),
            "the open transaction holds the connection exclusively",
            "borrowck",
        ),
        (
            "txn-begin-twice",
            (
                "    pub async fn run(conn: &mut DatabaseConnection)\n"
                "        -> Result<(), TransactionError<TransactionError<Error>>> {\n"
                "        conn.transaction(async |first| {\n"
                '            first.batch_execute("SELECT 1").await.map_err(TransactionError::Connection)?;\n'
                '            conn.transaction(async |second| second.batch_execute("SELECT 2").await)\n'
                "                .await\n"
                "        })\n"
                "        .await\n    }"
            ),
            rejects("E0501", "E0500"),
            "two live transactions on one connection is two exclusive borrows",
            "borrowck",
        ),
        (
            "txn-escapes-closure",
            (
                "    pub async fn run(conn: &mut DatabaseConnection) {\n"
                "        let mut held: Option<&DatabaseTransaction<'_>> = None;\n"
                "        let _ = conn\n"
                "            .transaction(async |tx| {\n"
                "                held = Some(&*tx);\n"
                "                Ok::<_, Error>(())\n"
                "            })\n"
                "            .await;\n"
                "        drop(held);\n    }"
            ),
            # The handle's lifetime is the closure's higher-ranked one, so the
            # escape surfaces as a region error rustc assigns no code to.
            rejects(message="lifetime may not live long enough"),
            "the handle cannot outlive the closure it was given to",
            "borrowck",
        ),
    )
    return [
        _transaction(
            identity,
            body,
            verdict="reject",
            phase=phase,
            expects=expectation,
            note=note,
        )
        for identity, body, expectation, note, phase in variants
    ]


def _pipeline(identity, body, *, verdict, phase, expects=None, note=""):
    return CompileCase(
        id=identity,
        obligation=OBLIGATION,
        verdict=verdict,
        phase=phase,
        source=PIPELINE_ENTITIES + "\n\n" + body,
        expects=expects,
        note=note,
    )


def binder_accept_cases():
    """Two pipelines, each binding its own values, which is the legal shape."""
    body = (
        "    use pgorm::pipeline::{ExprOps, Pipeline};\n\n"
        "    pub fn build() {\n"
        "        let _left = Pipeline::from(left::Entity)\n"
        "            .filter_with(|binder| left::Column::Id.eq(binder.bind(1_i32)));\n"
        "        let _right = Pipeline::from(right::Entity)\n"
        "            .filter_with(|binder| right::Column::Id.eq(binder.bind(2_i32)));\n"
        "    }"
    )
    nested = (
        "    use pgorm::pipeline::{ExprOps, Pipeline};\n\n"
        "    pub fn build() {\n"
        "        let _outer = Pipeline::from(left::Entity).filter_with(|binder| {\n"
        "            let first = binder.bind(1_i32);\n"
        "            let second = binder.bind(2_i32);\n"
        "            left::Column::Id.eq(first).and(left::Column::Id.ne(second))\n"
        "        });\n"
        "    }"
    )
    return [
        _pipeline(
            "binder-separate-pipelines",
            body,
            verdict="accept",
            phase="borrowck",
            note="each pipeline binds through its own binder",
        ),
        _pipeline(
            "binder-two-values",
            nested,
            verdict="accept",
            phase="borrowck",
            note="one brand, several values, composed inside the closure",
        ),
    ]


def binder_reject_cases():
    """Mixing brands across pipelines, at the escape and at the use."""
    escape = (
        "    use pgorm::pipeline::{Expr, ExprOps, Pipeline};\n\n"
        "    pub fn build() {\n"
        "        let mut smuggled: Option<Expr<'static>> = None;\n"
        "        let _left = Pipeline::from(left::Entity).filter_with(|binder| {\n"
        "            let bound = binder.bind(1_i32);\n"
        "            smuggled = Some(bound.clone());\n"
        "            left::Column::Id.eq(bound)\n"
        "        });\n"
        "        let _right = Pipeline::from(right::Entity)\n"
        "            .filter_with(move |_| right::Column::Id.eq(smuggled.clone().unwrap()));\n"
        "    }"
    )
    nested = (
        "    use pgorm::pipeline::{ExprOps, Pipeline};\n\n"
        "    pub fn build() {\n"
        "        let _outer = Pipeline::from(left::Entity).filter_with(|outer| {\n"
        "            let _inner = Pipeline::from(right::Entity)\n"
        "                .filter_with(|_| right::Column::Id.eq(outer.bind(1_i32)));\n"
        "            left::Column::Id.eq(outer.bind(2_i32))\n"
        "        });\n"
        "    }"
    )
    return [
        _pipeline(
            "binder-brand-escapes",
            escape,
            verdict="reject",
            phase="borrowck",
            expects=rejects("E0521"),
            note="a bound expression cannot leave the closure that minted it",
        ),
        _pipeline(
            "binder-brand-crossed",
            nested,
            verdict="reject",
            phase="borrowck",
            # The brand is invariant and introduced by a higher-ranked bound, so
            # the mismatch surfaces as a region error rustc assigns no code to.
            # An expectation that insisted on a code could not express this
            # rejection at all, and the surface would go uncovered.
            expects=rejects(message="lifetime may not live long enough"),
            note="one pipeline's binder cannot satisfy another's closure",
        ),
    ]


# [spec:pgorm:req:generative.compile-suite]
def cases():
    return (
        transaction_accept_cases()
        + transaction_reject_cases()
        + binder_accept_cases()
        + binder_reject_cases()
    )


__all__ = ["OBLIGATION", "cases"]
