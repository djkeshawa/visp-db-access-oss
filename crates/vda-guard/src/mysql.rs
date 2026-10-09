//! MySQL lexing must preserve comments before sqlparser expands them, and
//! AST-rendered literals must re-escape backslashes for the server's SQL mode.
use sqlparser::{
    dialect::{Dialect as ParserDialect, MySqlDialect},
    tokenizer::{Token, Tokenizer, TokenizerError},
};
use std::fmt::Write;

#[derive(Debug)]
pub(crate) struct Lexer {
    pub backslash_escape: bool,
}
impl ParserDialect for Lexer {
    fn dialect(&self) -> std::any::TypeId {
        std::any::TypeId::of::<MySqlDialect>()
    }
    fn is_identifier_start(&self, ch: char) -> bool {
        MySqlDialect {}.is_identifier_start(ch)
    }
    fn is_identifier_part(&self, ch: char) -> bool {
        MySqlDialect {}.is_identifier_part(ch)
    }
    fn is_delimited_identifier_start(&self, ch: char) -> bool {
        ch == '`'
    }
    fn supports_string_literal_backslash_escape(&self) -> bool {
        self.backslash_escape
    }
    fn ignores_wildcard_escapes(&self) -> bool {
        true
    }
    fn supports_numeric_prefix(&self) -> bool {
        true
    }
}

pub(crate) fn tokenize(sql: &str) -> Result<Vec<Token>, TokenizerError> {
    let lexer = Lexer {
        backslash_escape: true,
    };
    let mut tokens = Tokenizer::new(&lexer, sql)
        .with_unescape(false)
        .tokenize()?;
    for token in &mut tokens {
        match token {
            Token::SingleQuotedString(value) | Token::NationalStringLiteral(value) => {
                *value = literal(value, '\'')
            }
            Token::DoubleQuotedString(value) => *value = literal(value, '"'),
            Token::Word(word) if word.quote_style == Some('`') => {
                word.value = word.value.replace("``", "`")
            }
            _ => {}
        }
    }
    Ok(tokens)
}
fn literal(raw: &str, quote: char) -> String {
    // MySQL treats unknown escapes as the escaped character: \a and \f
    // are 'a' and 'f', unlike sqlparser's default BEL/form-feed conversion.
    let mut chars = raw.chars().peekable();
    let mut value = String::with_capacity(raw.len());
    while let Some(ch) = chars.next() {
        if ch == quote && chars.peek() == Some(&quote) {
            chars.next();
            value.push(quote);
        } else if ch == '\\' {
            if let Some(next) = chars.next() {
                let decoded = match next {
                    '0' => '\0',
                    'b' => '\u{8}',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    'Z' => '\u{1a}',
                    '%' | '_' => {
                        value.push('\\');
                        next
                    }
                    other => other,
                };
                value.push(decoded);
            }
        } else {
            value.push(ch);
        }
    }
    value
}

pub(crate) fn escape_rendered(sql: &str) -> String {
    // AST Display already doubles quotes, but leaves literal backslashes bare.
    // Read that output without interpreting backslashes, then escape them.
    let lexer = Lexer {
        backslash_escape: false,
    };
    let Ok(tokens) = Tokenizer::new(&lexer, sql).with_unescape(false).tokenize() else {
        return sql.to_owned();
    };
    let mut escaped = String::with_capacity(sql.len());
    for token in tokens {
        let token = match token {
            Token::SingleQuotedString(value) if value.contains('\\') => {
                Token::SingleQuotedString(value.replace('\\', "\\\\"))
            }
            Token::DoubleQuotedString(value) if value.contains('\\') => {
                Token::DoubleQuotedString(value.replace('\\', "\\\\"))
            }
            Token::NationalStringLiteral(value) if value.contains('\\') => {
                Token::NationalStringLiteral(value.replace('\\', "\\\\"))
            }
            token => token,
        };
        // Writing to a String cannot fail.
        let _ = write!(escaped, "{token}");
    }
    escaped
}
