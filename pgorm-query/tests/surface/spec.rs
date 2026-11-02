//! The spec's prose, read for the names and rule ids it writes in backticks
//! and for the rules it cites.
//!
//! A rule is a blockquote opened by its `> [spec:pgorm:<verb>:<id>]` marker.
//! Its lines are joined with single spaces, paragraphs kept apart, so a code
//! span the text wraps across a line still reads as one span and nothing here
//! depends on where a line breaks. A citation holds no space, so no line
//! break falls inside one.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// The checkout pgorm-query is a member of, whose `docs/spec` is the spec.
pub fn checkout_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

pub fn spec_dir() -> PathBuf {
    checkout_root().join("docs/spec")
}

/// Every Markdown file under `docs/spec`, nested directories included, in
/// path order.
pub fn spec_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        let entries =
            fs::read_dir(dir).unwrap_or_else(|error| panic!("reading {}: {error}", dir.display()));
        for entry in entries {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "md") {
                files.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&spec_dir(), &mut files);
    files.sort();
    files
}

pub fn read_spec_file(file: &Path) -> String {
    fs::read_to_string(file).unwrap_or_else(|error| panic!("reading {}: {error}", file.display()))
}

/// One rule's text, a string per paragraph.
pub struct Rule {
    pub id: String,
    pub paragraphs: Vec<String>,
}

impl Rule {
    /// Read the rule `id` out of `docs/spec/<file>`.
    pub fn read(file: &str, id: &str) -> Self {
        let file = spec_dir().join(file);
        let text = read_spec_file(&file);
        let mut lines = text.lines();
        lines
            .by_ref()
            .find(|line| defined_id(line) == Some(id))
            .unwrap_or_else(|| panic!("{} defines no rule `{id}`", file.display()));
        let mut paragraphs = vec![String::new()];
        for line in lines {
            let Some(body) = line.strip_prefix('>') else {
                break;
            };
            let body = body.trim();
            let paragraph = paragraphs.last_mut().expect("never empty");
            if body.is_empty() {
                if !paragraph.is_empty() {
                    paragraphs.push(String::new());
                }
            } else {
                if !paragraph.is_empty() {
                    paragraph.push(' ');
                }
                paragraph.push_str(body);
            }
        }
        paragraphs.retain(|paragraph| !paragraph.is_empty());
        Self {
            id: id.to_owned(),
            paragraphs,
        }
    }

    pub fn text(&self) -> String {
        self.paragraphs.join("\n\n")
    }

    /// The paragraph containing `needle`, which must be exactly one: a
    /// sentinel phrase that came to appear twice would otherwise pick a
    /// paragraph silently.
    pub fn paragraph_with(&self, needle: &str) -> &str {
        let mut found = self
            .paragraphs
            .iter()
            .filter(|paragraph| paragraph.contains(needle));
        match (found.next(), found.next()) {
            (Some(paragraph), None) => paragraph,
            (None, _) => panic!("no paragraph of `{}` contains {needle:?}", self.id),
            (Some(_), Some(_)) => panic!(
                "more than one paragraph of `{}` contains {needle:?}",
                self.id
            ),
        }
    }
}

/// The citation a line is when it is a rule marker, `> [spec:pgorm:req:a.b+3]`:
/// a blockquote line holding one bracketed citation and nothing else. A line
/// of prose that opens with a citation goes on after it, so it is not one.
pub fn marker(line: &str) -> Option<Citation<'_>> {
    let marker = line.strip_prefix('>')?.trim();
    let cited = citation(marker)?;
    (marker.starts_with('[') && cited.facet.is_none()).then_some(cited)
}

/// The id a line defines when it is a rule [`marker`].
pub fn defined_id(line: &str) -> Option<&str> {
    marker(line).map(|cited| cited.id)
}

/// The verb each rule the spec defines is declared under, by id, across every
/// file under `docs/spec`: `req` for the rule `> [spec:pgorm:req:a.b+3]` opens.
pub fn declared_verbs() -> BTreeMap<String, String> {
    spec_files()
        .iter()
        .flat_map(|file| {
            read_spec_file(file)
                .lines()
                .filter_map(marker)
                .filter_map(|cited| Some((cited.id.to_owned(), cited.verb?.to_owned())))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Every rule id the spec defines, across every file under `docs/spec`.
pub fn defined_rule_ids() -> BTreeSet<String> {
    declared_verbs().into_keys().collect()
}

/// A rule as the spec's text cites one: bracketed, `[spec:pgorm:req:a.b+3]`,
/// or bare, `a.b`, either with the version and the facet (`/test`) it names.
pub struct Citation<'a> {
    /// What a bracketed citation names between the namespace and the id,
    /// `req` above; a bare one names none.
    pub verb: Option<&'a str>,
    pub id: &'a str,
    pub version: Option<&'a str>,
    pub facet: Option<&'a str>,
}

/// Read `text` as one citation, bracketed or bare.
pub fn citation(text: &str) -> Option<Citation<'_>> {
    let (verb, cited) = match text.strip_prefix("[spec:pgorm:") {
        Some(rest) => {
            let (verb, cited) = rest.strip_suffix(']')?.split_once(':')?;
            (Some(verb), cited)
        }
        None => (None, text),
    };
    let (cited, facet) = match cited.split_once('/') {
        Some((cited, facet)) => (cited, Some(facet)),
        None => (cited, None),
    };
    let (id, version) = match cited.split_once('+') {
        Some((id, version)) => (id, Some(version)),
        None => (cited, None),
    };
    let numbered = version.is_none_or(|version| {
        !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit())
    });
    (is_rule_id(id) && numbered).then_some(Citation {
        verb,
        id,
        version,
        facet,
    })
}

/// Every bracketed citation in `line`, each read from its `[spec:pgorm:` to
/// the `]` that closes it. Outside prose a citation can stand flush against
/// a character [`citations`] keeps in a token (`notes.md:[spec:pgorm:req:a.b]`,
/// `[[spec:pgorm:req:a.b]]`), which would hide it there.
pub fn bracketed_citations(line: &str) -> impl Iterator<Item = Citation<'_>> {
    line.match_indices("[spec:pgorm:").filter_map(|(start, _)| {
        let rest = &line[start..];
        citation(&rest[..=rest.find(']')?])
    })
}

/// Every citation in `line`, bracketed, in backticks or bare. The line is cut
/// at every character a citation cannot hold, and a sentence's closing
/// punctuation is not part of what it ends.
pub fn citations(line: &str) -> impl Iterator<Item = Citation<'_>> {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || "[]:._-+/".contains(c)))
        .map(|token| token.trim_end_matches(['.', ':']))
        .filter_map(citation)
}

/// The text of each inline code span, in order.
pub fn code_spans(text: &str) -> impl Iterator<Item = &str> {
    text.split('`').skip(1).step_by(2)
}

/// A Rust identifier, as an item name is written.
pub fn is_identifier(span: &str) -> bool {
    let mut chars = span.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Spelled the way a type or trait is, `UpperCamelCase`: what a code span in
/// the register must mean when it is not an export. All-capitals spans are
/// SQL keywords there (`CASE`, `JSON_TABLE`), and lowercase ones are module
/// or function names, so neither is presumed to be an item.
pub fn is_item_shaped(span: &str) -> bool {
    is_identifier(span)
        && span.starts_with(|c: char| c.is_ascii_uppercase())
        && span.contains(|c: char| c.is_ascii_lowercase())
}

/// A rule id as the prose cites one, `sql.ddl.create-table`: two or more
/// dot-separated lowercase parts.
pub fn is_rule_id(span: &str) -> bool {
    let mut parts = span.split('.');
    let first_is_word = parts.next().is_some_and(|first| {
        first.starts_with(|c: char| c.is_ascii_lowercase()) && is_rule_part(first)
    });
    let mut rest = parts.peekable();
    first_is_word && rest.peek().is_some() && rest.all(is_rule_part)
}

fn is_rule_part(part: &str) -> bool {
    !part.is_empty()
        && part
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// What the register of `sql.surface` records: each added item's name, with
/// the rule ids the prose cites for it.
pub struct Register {
    pub entries: BTreeMap<String, Vec<String>>,
    /// Names after the last rule id: recorded, but with no rule.
    pub without_rule: Vec<String>,
}

impl Register {
    /// Read the register out of `text`, the span of the rule that holds it.
    ///
    /// An entry is the prose's own shape, one or more names and then the
    /// rule in parentheses — `Deferrability`, when a constraint's check runs
    /// (`sql.ddl.deferrability`) — so each name is attributed to the rule ids
    /// that next follow it. A code span is a name when it is an identifier
    /// that `is_export` accepts or that [`is_item_shaped`]; anything else is
    /// the prose's own vocabulary and is passed over.
    pub fn parse(text: &str, is_export: impl Fn(&str) -> bool) -> Self {
        let mut entries: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut pending: Vec<&str> = Vec::new();
        let mut cited: Vec<&str> = Vec::new();
        let mut attribute = |pending: &mut Vec<&str>, cited: &mut Vec<&str>| {
            for name in pending.drain(..) {
                let rules = entries.entry(name.to_owned()).or_default();
                rules.extend(cited.iter().map(|&rule| rule.to_owned()));
            }
            cited.clear();
        };
        for span in code_spans(text) {
            if is_rule_id(span) {
                cited.push(span);
                continue;
            }
            if !cited.is_empty() {
                attribute(&mut pending, &mut cited);
            }
            if is_identifier(span) && (is_export(span) || is_item_shaped(span)) {
                pending.push(span);
            }
        }
        if !cited.is_empty() {
            attribute(&mut pending, &mut cited);
        }
        Self {
            entries,
            without_rule: pending.into_iter().map(str::to_owned).collect(),
        }
    }

    pub fn cited_rules(&self) -> BTreeSet<&str> {
        self.entries
            .values()
            .flatten()
            .map(String::as_str)
            .collect()
    }
}
