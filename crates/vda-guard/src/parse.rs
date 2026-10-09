//! Bounded parsing and token-aware handling of unsupported MySQL syntax.
use crate::Dialect;
use sqlparser::{
    ast::{Expr, Ident, LimitClause, Query, Statement, Value, VisitMut, VisitorMut},
    dialect::{MySqlDialect, PostgreSqlDialect},
    parser::Parser,
    tokenizer::{Token, Tokenizer, Whitespace},
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum ParseFailure {
    #[error("{0}")]
    Parser(String),
    #[error("{0}")]
    FileWrite(String),
    #[error("Executable MySQL/MariaDB comments are not allowed")]
    ExecutableComment,
    #[error("Optimizer hints may override gateway execution limits")]
    OptimizerHint,
}

pub(crate) struct Parsed {
    pub statements: Vec<Statement>,
    pub lock_syntax: Vec<&'static str>,
}

pub(crate) fn parse(sql: &str, dialect: Dialect) -> Result<Parsed, ParseFailure> {
    let pg = PostgreSqlDialect {};
    let my = MySqlDialect {};
    let parser_dialect: &dyn sqlparser::dialect::Dialect = match dialect {
        Dialect::Postgres => &pg,
        Dialect::MySql => &my,
    };
    let tokens = if dialect == Dialect::MySql {
        crate::mysql::tokenize(sql)
    } else {
        Tokenizer::new(parser_dialect, sql).tokenize()
    }
    .map_err(|e| ParseFailure::Parser(e.to_string()))?;
    check_comments(&tokens, dialect)?;
    let mut tokens: Vec<_> = tokens
        .into_iter()
        .filter(|t| !matches!(t, Token::Whitespace(_)))
        .collect();
    bound_complexity(&tokens)?;
    if dialect == Dialect::MySql {
        if tokens.iter().any(|t| matches!(t, Token::Assignment))
            || tokens.windows(2).any(|w| {
                keyword(w.first(), "INTO")
                    && matches!(w.get(1), Some(Token::Word(w)) if w.value.starts_with('@'))
            })
        {
            return Err(ParseFailure::Parser(
                "Session variable assignment is not allowed".into(),
            ));
        }
        let alternate = crate::mysql::Lexer {
            backslash_escape: false,
        };
        if let Ok(alternate) = Tokenizer::new(&alternate, sql).tokenize() {
            let boundaries = |tokens: &[Token]| {
                tokens
                    .iter()
                    .filter(|t| matches!(t, Token::SemiColon))
                    .count()
            };
            if boundaries(&alternate) != boundaries(&tokens) {
                return Err(ParseFailure::Parser(
                    "Statement boundaries depend on NO_BACKSLASH_ESCAPES".into(),
                ));
            }
        }
    }
    if dialect == Dialect::MySql
        && tokens.windows(2).any(|w| {
            keyword(w.first(), "INTO")
                && (keyword(w.get(1), "OUTFILE") || keyword(w.get(1), "DUMPFILE"))
        })
    {
        let message = Parser::new(parser_dialect)
            .with_recursion_limit(32)
            .with_tokens(tokens)
            .parse_statements()
            .err()
            .map_or_else(
                || "Unsupported file-writing SELECT destination".to_owned(),
                |e| e.to_string(),
            );
        return Err(ParseFailure::FileWrite(message));
    }
    let lock_syntax = normalize_locks(&mut tokens, dialect);
    // sqlparser discards LIMIT ALL, making it indistinguishable from OFFSET.
    // Preserve it as an expression through parsing, then restore its spelling.
    let mut marker = "340282366920938463463374607431768211455".to_owned();
    while tokens
        .iter()
        .any(|t| matches!(t, Token::Number(n, _) if n == &marker))
    {
        marker.push('0');
    }
    let mut position = 0;
    while position + 1 < tokens.len() {
        if keyword(tokens.get(position), "LIMIT") && keyword(tokens.get(position + 1), "ALL") {
            if let Some(token) = tokens.get_mut(position + 1) {
                *token = Token::Number(marker.clone(), false);
            }
        }
        position += 1;
    }
    let mut statements = Parser::new(parser_dialect)
        .with_recursion_limit(32)
        .with_tokens(tokens)
        .parse_statements()
        .map_err(|e| ParseFailure::Parser(e.to_string()))?;
    let _ = statements.visit(&mut RestoreAll { marker });
    Ok(Parsed {
        statements,
        lock_syntax,
    })
}

fn keyword(token: Option<&Token>, value: &str) -> bool {
    matches!(token, Some(Token::Word(w)) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case(value))
}

fn check_comments(tokens: &[Token], dialect: Dialect) -> Result<(), ParseFailure> {
    if dialect != Dialect::MySql {
        return Ok(());
    }
    for token in tokens {
        match token {
            Token::Whitespace(Whitespace::MultiLineComment(comment))
                if comment.starts_with('!') || comment.starts_with("M!") =>
            {
                return Err(ParseFailure::ExecutableComment);
            }
            Token::Whitespace(Whitespace::MultiLineComment(comment))
                if comment.starts_with('+') =>
            {
                return Err(ParseFailure::OptimizerHint);
            }
            Token::Whitespace(Whitespace::SingleLineComment { prefix, comment })
                if prefix == "--"
                    && comment
                        .chars()
                        .next()
                        .is_some_and(|c| !c.is_whitespace() && !c.is_control()) =>
            {
                return Err(ParseFailure::Parser(
                    "MySQL requires whitespace after -- comments; use an unambiguous comment."
                        .into(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

// The parser limits recursive descent, but left-associative ASTs can still get
// arbitrarily deep. Bound those chains before constructing, walking, or dropping them.
fn bound_complexity(tokens: &[Token]) -> Result<(), ParseFailure> {
    // Words that build left-deep infix or postfix chains (`x IS NULL IS NULL ...`)
    // count like punctuation operators; other keywords and identifiers do not.
    const CHAIN_WORDS: [&str; 20] = [
        "AND", "OR", "NOT", "WHEN", "IS", "ISNULL", "NOTNULL", "IN", "LIKE", "ILIKE", "RLIKE",
        "REGEXP", "SIMILAR", "COLLATE", "AT", "XOR", "DIV", "MOD", "OVERLAPS", "ESCAPE",
    ];
    let mut chains = vec![0_usize];
    // Running sum of `chains`, so each token costs O(1) instead of O(depth).
    let mut total = 0_usize;
    let mut sets = 0_usize;
    for token in tokens {
        match token {
            Token::SemiColon => {
                chains.clear();
                chains.push(0);
                total = 0;
                sets = 0;
            }
            Token::Comma => {
                if let Some(chain) = chains.last_mut() {
                    total = total.saturating_sub(*chain);
                    *chain = 0;
                }
            }
            Token::LParen | Token::LBracket => {
                if let Some(chain) = chains.last_mut() {
                    *chain += 1;
                    total += 1;
                }
                chains.push(0);
            }
            Token::RParen | Token::RBracket => {
                if chains.len() > 1 {
                    total = total.saturating_sub(chains.pop().unwrap_or(0));
                }
            }
            Token::Word(w)
                if ["UNION", "INTERSECT", "EXCEPT"]
                    .iter()
                    .any(|k| w.value.eq_ignore_ascii_case(k)) =>
            {
                sets += 1;
            }
            Token::Word(w) if CHAIN_WORDS.iter().any(|k| w.value.eq_ignore_ascii_case(k)) => {
                if let Some(chain) = chains.last_mut() {
                    *chain += 1;
                    total += 1;
                }
            }
            Token::Word(_)
            | Token::Number(_, _)
            | Token::SingleQuotedString(_)
            | Token::DoubleQuotedString(_) => {}
            _ => {
                if let Some(chain) = chains.last_mut() {
                    *chain += 1;
                    total += 1;
                }
            }
        }
        if chains.len() > 64 || total > 128 || sets > 64 {
            return Err(ParseFailure::Parser("SQL expression is too complex to analyze safely. Split long expression or set-operation chains.".into()));
        }
    }
    Ok(())
}

fn normalize_locks(tokens: &mut Vec<Token>, dialect: Dialect) -> Vec<&'static str> {
    let original = std::mem::take(tokens);
    let mut normalized = Vec::with_capacity(original.len());
    let mut syntax = vec![];
    let mut position = 0;
    while position < original.len() {
        let (consumed, spelling, kind) = if keyword(original.get(position), "FOR") {
            if keyword(original.get(position + 1), "NO")
                && keyword(original.get(position + 2), "KEY")
                && keyword(original.get(position + 3), "UPDATE")
            {
                (4, "FOR NO KEY UPDATE", "UPDATE")
            } else if keyword(original.get(position + 1), "KEY")
                && keyword(original.get(position + 2), "SHARE")
            {
                (3, "FOR KEY SHARE", "SHARE")
            } else if keyword(original.get(position + 1), "UPDATE") {
                (2, "FOR UPDATE", "UPDATE")
            } else if keyword(original.get(position + 1), "SHARE") {
                (2, "FOR SHARE", "SHARE")
            } else {
                if let Some(token) = original.get(position) {
                    normalized.push(token.clone());
                }
                position += 1;
                continue;
            }
        } else if dialect == Dialect::MySql
            && keyword(original.get(position), "LOCK")
            && keyword(original.get(position + 1), "IN")
            && keyword(original.get(position + 2), "SHARE")
            && keyword(original.get(position + 3), "MODE")
        {
            (4, "LOCK IN SHARE MODE", "SHARE")
        } else {
            if let Some(token) = original.get(position) {
                normalized.push(token.clone());
            }
            position += 1;
            continue;
        };
        syntax.push(spelling);
        normalized.extend([Token::make_keyword("FOR"), Token::make_keyword(kind)]);
        position += consumed;
    }
    *tokens = normalized;
    syntax
}

struct RestoreAll {
    marker: String,
}
impl VisitorMut for RestoreAll {
    type Break = ();
    fn pre_visit_query(&mut self, query: &mut Query) -> std::ops::ControlFlow<Self::Break> {
        if let Some(LimitClause::LimitOffset {
            limit: Some(expr), ..
        }) = &mut query.limit_clause
        {
            if matches!(expr, Expr::Value(v) if matches!(&v.value, Value::Number(n, _) if n == &self.marker))
            {
                *expr = Expr::Identifier(Ident::new("ALL"));
            }
        }
        std::ops::ControlFlow::Continue(())
    }
}
