/// The file stem of `path`, which is what the `MigrationName` derive resolves
/// `file!()` to. This is public because the derive expands in the caller's
/// crate, so it is reachable with any string: a path carrying no file stem
/// (`""`, `"."`, `"foo/"`) or a non-UTF-8 one yields `path` unchanged.
// [spec:pgorm:sem:migration.name+4]    what MigrationName resolves `file!()` to
pub fn get_file_stem(path: &str) -> &str {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_file_stem() {
        let pair = vec![
            (
                "m20220101_000001_create_table.rs",
                "m20220101_000001_create_table",
            ),
            (
                "src/m20220101_000001_create_table.rs",
                "m20220101_000001_create_table",
            ),
            (
                "migration/src/m20220101_000001_create_table.rs",
                "m20220101_000001_create_table",
            ),
            (
                "/migration/src/m20220101_000001_create_table.tmp.rs",
                "m20220101_000001_create_table.tmp",
            ),
            ("", ""),
            (".", "."),
            ("migration/src/", "src"),
            ("..", ".."),
        ];
        for (path, expect) in pair {
            assert_eq!(get_file_stem(path), expect);
        }
    }
}
