use super::*;
use crate::postgres::oracle::{assert_eq, assert_eq_unparsed};

// [spec:pgorm:def:sql.token+1/test]    the cursor is the tokenizer's own: it
// advances only by iteration, and iteration ends rather than indexing past the
// input
#[test]
fn the_cursor_is_advanced_only_by_iteration() {
    let mut tokenizer = Tokenizer::new("a b");
    assert_eq!(tokenizer.next(), Some(Token::Unquoted("a".to_owned())));
    assert_eq!(tokenizer.next(), Some(Token::Space(" ".to_owned())));
    assert_eq!(tokenizer.next(), Some(Token::Unquoted("b".to_owned())));

    // Exhausted, and asking again keeps answering `None` instead of reading a
    // character that is not there.
    assert_eq!(tokenizer.next(), None);
    assert_eq!(tokenizer.next(), None);
    assert_eq!(Tokenizer::new("").next(), None);

    // The character vector and the cursor are private, so no caller can put the
    // cursor anywhere `next()` would index unchecked.
    //
    // ```compile_fail,E0616
    // let tokenizer = pgorm_query::Tokenizer::new("a");
    // let _ = tokenizer.p;
    // ```
}

// [spec:pgorm:def:sql.token+1/test]    the four classes, each carrying its exact source text
#[test]
fn token_classes_carry_their_source_text_verbatim() {
    let input = r#"SELECT 'a' , "b""#;
    let tokens: Vec<Token> = Tokenizer::new(input).iter().collect();

    assert_eq!(
        tokens,
        vec![
            Token::Unquoted("SELECT".to_owned()),
            Token::Space(" ".to_owned()),
            Token::Quoted("'a'".to_owned()),
            Token::Space(" ".to_owned()),
            Token::Punctuation(",".to_owned()),
            Token::Space(" ".to_owned()),
            Token::Quoted(r#""b""#.to_owned()),
        ]
    );

    // `as_str` and `Display` both hand back the carried text unmodified: quoted
    // tokens keep their delimiters, spaces keep their exact run.
    let strs: Vec<&str> = tokens.iter().map(|t| t.as_str()).collect();
    assert_eq!(strs, vec!["SELECT", " ", "'a'", " ", ",", " ", r#""b""#]);
    for token in &tokens {
        assert_eq!(token.as_str(), token.to_string());
    }
    assert_eq!(strs.concat(), input);

    assert!(tokens[0].is_unquoted());
    assert!(tokens[1].is_space());
    assert!(tokens[2].is_quoted());
    assert!(tokens[4].is_punctuation());
}

// [spec:pgorm:sem:sql.token.limits+3/test]    a line comment is one space
// token, ending at the newline it includes
#[test]
fn line_comments_lex_as_one_space_token() {
    let tokens: Vec<Token> = Tokenizer::new("1 -- two").iter().collect();

    assert_eq!(
        tokens,
        vec![
            Token::Unquoted("1".to_owned()),
            Token::Space(" ".to_owned()),
            Token::Space("-- two".to_owned()),
        ]
    );
}

// [spec:pgorm:sem:sql.token.limits+3/test]    a block comment is one space
// token, and a quote character inside a comment stays inside it
#[test]
fn block_comments_lex_as_one_space_token() {
    let tokens: Vec<Token> = Tokenizer::new("/* a */").iter().collect();
    assert_eq!(tokens, vec![Token::Space("/* a */".to_owned())]);

    let tokens: Vec<Token> = Tokenizer::new("-- it's fine\nSELECT").iter().collect();
    assert_eq!(
        tokens,
        vec![
            Token::Space("-- it's fine\n".to_owned()),
            Token::Unquoted("SELECT".to_owned()),
        ]
    );
}

// [spec:pgorm:sem:sql.token.limits+3/test]    a dollar-quoted body — bare or
// tagged — is one quoted token
#[test]
fn dollar_quoting_lexes_as_one_quoted_token() {
    let tokens: Vec<Token> = Tokenizer::new("$$body$$").iter().collect();
    assert_eq!(tokens, vec![Token::Quoted("$$body$$".to_owned())]);

    let tokens: Vec<Token> = Tokenizer::new("$tag$body$tag$").iter().collect();
    assert_eq!(tokens, vec![Token::Quoted("$tag$body$tag$".to_owned())]);
}

// [spec:pgorm:sem:sql.token.limits+3/test]    an E-string is one quoted token,
// its backslash escapes honoured
#[test]
fn e_strings_lex_as_one_quoted_token() {
    let tokens: Vec<Token> = Tokenizer::new(r#"E'a\nb'"#).iter().collect();
    assert_eq!(tokens, vec![Token::Quoted(r#"E'a\nb'"#.to_owned())]);
}

// [spec:pgorm:req:sql.token.quoted+3/test]    the delimiter set is PostgreSQL's
// own: backtick and bracket are punctuation, so text after one is still read
#[test]
fn only_postgresql_delimiters_open_a_string() {
    let tokens: Vec<Token> = Tokenizer::new("`a` [b] \"c\" 'd'").iter().collect();
    assert_eq_unparsed!(
        tokens
            .iter()
            .filter(|t| t.is_quoted())
            .map(|t| t.as_str())
            .collect::<Vec<_>>(),
        vec!["\"c\"", "'d'"],
        "only the double- and single-quoted forms are strings"
    );
    assert!(
        tokens
            .iter()
            .any(|t| t.is_punctuation() && t.as_str() == "`")
    );
    assert!(
        tokens
            .iter()
            .any(|t| t.is_punctuation() && t.as_str() == "[")
    );

    // The defect this closes: a backtick used to swallow the rest of the
    // statement as one opaque body, hiding the placeholder inside it.
    let tokens: Vec<Token> = Tokenizer::new("SELECT `x` WHERE a = $1").iter().collect();
    assert!(
        tokens
            .iter()
            .any(|t| t.is_punctuation() && t.as_str() == "$"),
        "the placeholder marker is still visible: {tokens:?}"
    );
    // Not valid PostgreSQL — which is the point — so it is compared as text
    // rather than put to the render oracle.
    assert_eq_unparsed!(
        tokens.iter().map(Token::as_str).collect::<String>(),
        "SELECT `x` WHERE a = $1"
    );
}
