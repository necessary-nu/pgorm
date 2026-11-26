//! Verification for the hidden `#[pgorm_macros::test]` harness attribute.
//!
//! These cases need no database: they are about the rewrite the attribute
//! performs, not about anything it talks to.

#![allow(unused_imports, dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

static BODY_RUNS: AtomicUsize = AtomicUsize::new(0);

/// The attribute turns an `async fn` into a plain `#[test]` fn that installs a
/// DEBUG `tracing_subscriber` with a test writer and then drives the original
/// body through `crate::block_on!`.
// [spec:pgorm:sem:macros.derive.test-attr+1/test]
#[pgorm_macros::test]
async fn async_body_is_driven_by_block_on() {
    // `crate::block_on!` builds a tokio runtime and blocks on the body, so the
    // body is executing inside one.
    assert!(
        tokio::runtime::Handle::try_current().is_ok(),
        "the body should run inside the runtime block_on! created"
    );

    // A global tracing dispatcher was installed before the body started.
    assert!(tracing::dispatcher::has_been_set());

    // And the body really is async.
    BODY_RUNS.fetch_add(1, Ordering::SeqCst);
    tokio::task::yield_now().await;
    assert_eq!(BODY_RUNS.load(Ordering::SeqCst), 1);
}

/// Caller attributes are re-emitted on the generated fn verbatim — which is the
/// only reason `should_panic` has any effect here.
// [spec:pgorm:sem:macros.derive.test-attr+1/test]
#[pgorm_macros::test]
#[should_panic(expected = "the body panicked")]
async fn caller_attributes_are_passed_through() {
    panic!("the body panicked");
}

/// The return type is carried over too, so a fallible body still works.
// [spec:pgorm:sem:macros.derive.test-attr+1/test]
#[pgorm_macros::test]
async fn the_return_type_is_preserved() -> Result<(), pgorm::Error> {
    Ok(())
}
