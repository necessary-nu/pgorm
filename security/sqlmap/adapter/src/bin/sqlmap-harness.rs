use pgorm_sqlmap_adapter::harness::{self, Options};
use std::path::Path;

#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "sqlmap-harness [--profile smoke|full|identifiers] [--case ID] [--artifacts DIR] [--baseline-only] [--direct-regressions] [--remeasure-exemptions] [--python PATH]"
        );
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let result = match Options::parse(args.into_iter(), &root) {
        Ok(options) => harness::run(options).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(error) => {
            eprintln!("sqlmap-harness: {error}");
            std::process::exit(1);
        }
    }
}
