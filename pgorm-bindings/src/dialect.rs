//! How a binding spells what a refusal names.

use std::fmt;

/// The binding a refusal is worded for: an option is written by the
/// binding's naming rule (`python.naming`, `napi.naming`), and a choice
/// quoted as its language writes a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// pgorm-python: snake_case options, `'single'` quotes.
    Python,
    /// pgorm-napi: camelCase options, `"double"` quotes.
    JavaScript,
}

impl Dialect {
    /// The option `neutral`, a snake_case name, as the binding spells it.
    pub fn option(self, neutral: &str) -> String {
        match self {
            Self::Python => neutral.to_owned(),
            Self::JavaScript => {
                let mut spelled = String::with_capacity(neutral.len());
                let mut upper = false;
                for character in neutral.chars() {
                    if character == '_' {
                        upper = true;
                    } else if upper {
                        spelled.extend(character.to_uppercase());
                        upper = false;
                    } else {
                        spelled.push(character);
                    }
                }
                spelled
            }
        }
    }

    /// `option` set to the choice `value`, as a program writes it:
    /// `tls='disable'` in Python, `tls: "disable"` in JavaScript.
    pub fn setting(self, option: &str, value: &str) -> String {
        match self {
            Self::Python => format!("{}='{value}'", self.option(option)),
            Self::JavaScript => format!("{}: \"{value}\"", self.option(option)),
        }
    }

    /// How a program gives the certificates TLS trusts: Python also reads
    /// them from a file it names.
    pub(crate) fn ca(self) -> &'static str {
        match self {
            Self::Python => "ca or cafile",
            Self::JavaScript => "ca",
        }
    }
}

/// A value of the right type that the binding cannot use, raised as its
/// `ConstructionError` (`bindings.argument-errors`). A value of the wrong
/// type, or a choice outside its names, is the binding's own `TypeError`,
/// refused as it reads its language's arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub(crate) String);

impl Refusal {
    /// What the refusal says.
    pub fn message(&self) -> &str {
        &self.0
    }

    /// What the refusal says, owned.
    pub fn into_message(self) -> String {
        self.0
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for Refusal {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_are_spelled_by_each_bindings_rule() {
        assert_eq!(
            Dialect::Python.option("statement_cache_channel"),
            "statement_cache_channel"
        );
        assert_eq!(
            Dialect::JavaScript.option("statement_cache_channel"),
            "statementCacheChannel"
        );
        assert_eq!(Dialect::JavaScript.option("tls"), "tls");
        assert_eq!(Dialect::Python.setting("tls", "disable"), "tls='disable'");
        assert_eq!(
            Dialect::JavaScript.setting("queue_mode", "lifo"),
            "queueMode: \"lifo\""
        );
    }
}
