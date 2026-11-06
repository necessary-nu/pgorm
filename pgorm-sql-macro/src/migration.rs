//! The compile-time half of pgorm-migration's `sql_migration!`: read a
//! migration's `.sql` file, hold it to the grammar, and hand back its name and
//! the text to run.

use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Error, LitStr, Token};

/// `$crate, "path"`, as pgorm-migration's `sql_migration!` forwards it: the
/// path to the crate defining `SqlMigration`, then the caller's literal.
pub(crate) struct MigrationInput {
    krate: TokenStream,
    path: LitStr,
}

impl Parse for MigrationInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut krate = TokenStream::new();
        while !input.peek(Token![,]) {
            krate.extend([input.parse::<TokenTree>()?]);
        }
        input.parse::<Token![,]>()?;
        let path = input.parse()?;
        input.parse::<Option<Token![,]>>()?;
        Ok(Self { krate, path })
    }
}

/// A migration file as the macro read it.
#[derive(Debug)]
pub(crate) struct Script {
    /// The file stem, which is the migration's name.
    name: String,
    /// The file's text with every CRLF written as LF.
    text: String,
    /// Where the file was read from, for the expansion to include.
    path: PathBuf,
}

/// The file's name and checked text as literals, with the file itself included
/// so that editing it rebuilds the calling crate.
// [spec:pgorm:def:migration.sql-file]
// [spec:pgorm:req:migration.sql-file.compile+1]    the file is an input of the calling crate's build
pub(crate) fn expand(input: &MigrationInput) -> Result<TokenStream, Error> {
    let literal = &input.path;
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR").ok_or_else(|| {
        Error::new(
            literal.span(),
            "sql_migration! resolves its path against CARGO_MANIFEST_DIR, which is not set",
        )
    })?;
    let script = load(literal, Path::new(&manifest_dir))?;
    let tracked = script.path.to_str().ok_or_else(|| {
        Error::new(
            literal.span(),
            format!("{} is not a UTF-8 path", script.path.display()),
        )
    })?;

    let krate = &input.krate;
    let Script { name, text, .. } = &script;
    Ok(quote! {{
        const _: &[u8] = ::core::include_bytes!(#tracked);
        #krate::SqlMigration::new(#name, #text)
    }})
}

/// Read the file `literal` names under `manifest_dir`, and hold it to the
/// grammar.
// [spec:pgorm:req:migration.sql-file.compile+1]    resolved against the manifest directory, refused at compile time
// [spec:pgorm:req:migration.sql-file.checksum]    line endings are LF before anything sees the text
pub(crate) fn load(literal: &LitStr, manifest_dir: &Path) -> Result<Script, Error> {
    let relative = literal.value();
    let refuse = |message: String| Error::new(literal.span(), message);

    let name = Path::new(&relative)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| refuse(format!("`{relative}` names no file")))?
        .to_owned();
    let path = manifest_dir.join(&relative);
    let bytes =
        std::fs::read(&path).map_err(|error| refuse(format!("cannot read {relative}: {error}")))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| refuse(format!("{relative} is not UTF-8 text")))?
        .replace("\r\n", "\n");

    if let Err(refused) = pgorm_sql_check::check_sql(&text) {
        return Err(refuse(rejection(&relative, &text, &refused)));
    }

    Ok(Script { name, text, path })
}

/// The message for a file the grammar refused: the parser's diagnostic and,
/// when it quotes a token the file holds, that token's line, reprinted with a
/// caret under it. A diagnostic quoting no token carries no position.
fn rejection(relative: &str, text: &str, refused: &pgorm_sql_check::Rejection) -> String {
    let mut rendered = format!("PostgreSQL rejected {relative}: {}", refused.message());
    if let Some(offset) = refused.offset() {
        let line = text
            .get(..offset)
            .map_or(1, |before| before.matches('\n').count() + 1);
        rendered.push_str(&format!(" (line {line})"));
        if let Some(block) = refused.caret() {
            rendered.push_str(block);
        }
    }
    rendered
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use proc_macro2::Span;
    use syn::{Error, LitStr};

    use super::{Script, load};

    /// A directory of migration files under the system's temporary directory,
    /// removed when dropped.
    struct Files(PathBuf);

    impl Files {
        fn new(case: &str, files: &[(&str, &[u8])]) -> std::io::Result<Self> {
            let dir = std::env::temp_dir()
                .join(format!("pgorm-sql-migration-{}-{case}", std::process::id()));
            std::fs::create_dir_all(&dir)?;
            for (name, bytes) in files {
                std::fs::write(dir.join(name), bytes)?;
            }
            Ok(Self(dir))
        }

        fn load(&self, relative: &str) -> Result<Script, Error> {
            load(&LitStr::new(relative, Span::call_site()), &self.0)
        }

        fn refusal(&self, relative: &str) -> String {
            self.load(relative)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default()
        }
    }

    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // [spec:pgorm:def:migration.sql-file/test]    the name is the file stem
    // [spec:pgorm:req:migration.sql-file.checksum/test]    CRLF line endings are read as LF
    #[test]
    fn crlf_line_endings_are_read_as_lf() -> Result<(), Box<dyn std::error::Error>> {
        let files = Files::new(
            "crlf",
            &[(
                "m20260101_000001_create_note.sql",
                b"CREATE TABLE note (id int);\r\nINSERT INTO note VALUES (1);\r\n",
            )],
        )?;

        let script = files.load("m20260101_000001_create_note.sql")?;
        assert_eq!(script.name, "m20260101_000001_create_note");
        assert_eq!(
            script.text,
            "CREATE TABLE note (id int);\nINSERT INTO note VALUES (1);\n"
        );
        assert_eq!(
            script.path,
            files.0.join("m20260101_000001_create_note.sql")
        );
        Ok(())
    }

    // [spec:pgorm:req:migration.sql-file.compile+1/test]    the refusal names the file and the line
    #[test]
    fn a_grammar_refusal_names_file_and_line() -> std::io::Result<()> {
        let files = Files::new(
            "grammar",
            &[(
                "m20260101_000001_broken.sql",
                b"CREATE TABLE note (id int);\nSELECT id FORM note;\n",
            )],
        )?;

        let message = files.refusal("m20260101_000001_broken.sql");
        assert!(
            message.starts_with(
                "PostgreSQL rejected m20260101_000001_broken.sql: \
                 syntax error at or near \"note\" (line 2)"
            ),
            "{message}"
        );
        assert!(message.contains("\n  SELECT id FORM note;\n"), "{message}");
        Ok(())
    }

    // [spec:pgorm:req:migration.sql-file.compile+1/test]    a refusal quoting no token names no line
    #[test]
    fn a_refusal_quoting_no_token_names_no_line() -> std::io::Result<()> {
        let files = Files::new(
            "end-of-input",
            &[(
                "m20260101_000001_cut.sql",
                b"CREATE TABLE note (id int);\nSELECT (",
            )],
        )?;

        assert_eq!(
            files.refusal("m20260101_000001_cut.sql"),
            "PostgreSQL rejected m20260101_000001_cut.sql: syntax error at end of input"
        );
        Ok(())
    }

    // [spec:pgorm:req:migration.sql-file.compile+1/test]    unusable files are refused, not run
    #[test]
    fn unreadable_files_are_refused() -> std::io::Result<()> {
        let files = Files::new("unusable", &[("latin1.sql", b"SELECT '\xe9';")])?;

        assert!(
            files
                .refusal("missing.sql")
                .starts_with("cannot read missing.sql: ")
        );
        assert_eq!(files.refusal("latin1.sql"), "latin1.sql is not UTF-8 text");
        assert_eq!(files.refusal(""), "`` names no file");
        Ok(())
    }
}
