//! What a minted name holds, and how `ps`'s answer is read.

use pgorm_test_support::{creator, elapsed_secs, scoped_name, this_run};

// [spec:pgorm:req:testing.isolation+1/test]    a name fits 63 bytes, keeps the
// test's label readable and records the process that minted it
#[test]
fn a_scoped_name_fits_and_records_its_run() {
    let label = "a_label_long_enough_that_no_tail_could_ever_fit_beside_it_whole";
    let name = scoped_name(label);
    assert!(name.len() <= 63, "{name} is {} bytes", name.len());
    assert!(name.starts_with("a_label_long_enough"), "{name}");
    assert_eq!(creator(&name), Some(this_run()));

    let again = scoped_name(label);
    assert_ne!(name, again, "one label asked for twice names two objects");
    assert_eq!(creator(&again), Some(this_run()));

    let short = scoped_name("short");
    assert!(short.starts_with("short_"), "{short}");
    assert_eq!(creator("short"), None);
    assert_eq!(
        creator("crud_tests_5e2d1a7f"),
        None,
        "the checkout-hash form"
    );
}

// [spec:pgorm:req:testing.isolation+1/test]    ps's elapsed time is read in each of
// its forms, and anything else is not an answer
#[test]
fn elapsed_time_reads_each_ps_form() {
    assert_eq!(elapsed_secs("00:05"), Some(5));
    assert_eq!(elapsed_secs("01:02:03"), Some(3_723));
    assert_eq!(elapsed_secs("2-01:00:00"), Some(2 * 86_400 + 3_600));
    assert_eq!(elapsed_secs("5"), None);
    assert_eq!(elapsed_secs("1:2:3:4"), None);
    assert_eq!(elapsed_secs("ps: no such process"), None);
}
