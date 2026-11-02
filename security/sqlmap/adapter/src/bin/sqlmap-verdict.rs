use pgorm_sqlmap_adapter::harness::verdict;
use std::{fs, io::Write, path::PathBuf};

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [profile, artifacts] = args.as_slice() else {
        eprintln!("sqlmap-verdict <smoke|full|identifiers> <artifact directory>");
        std::process::exit(1);
    };
    let (passed, summary) = match verdict::publish(profile, &PathBuf::from(artifacts)) {
        Ok(published) => published,
        Err(error) => {
            eprintln!("sqlmap-verdict: {error}");
            std::process::exit(1);
        }
    };
    print!("{summary}");
    if let Ok(destination) = std::env::var("GITHUB_STEP_SUMMARY")
        && let Ok(mut step) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(destination)
    {
        let _ = write!(step, "{summary}");
    }
    if !passed {
        std::process::exit(1);
    }
}
