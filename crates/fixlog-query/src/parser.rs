//! Recursive-descent parser for the filter DSL.
//!
//! Grammar (see `lib.rs` for the full EBNF):
//!
//! ```text
//! expr     := and_expr ("OR" and_expr)*
//! and_expr := not_expr ("AND" not_expr)*
//! not_expr := "NOT" not_expr | atom
//! atom     := "(" expr ")" | predicate
//! ```
//!
//! Keywords are case-insensitive (`AND`/`and`/`And` all work). Tag values are either a
//! bareword (any run of non-space, non-operator bytes — no escaping needed for common
//! FIX values) or a `"..."` quoted string with `\"` and `\\` escapes.
//!
//! Error messages carry the byte offset within the input so a CLI can point a caret at
//! the problem.

use crate::ast::{Expr, Op, Predicate};

/// Parse a filter expression from `input`. Returns the root [`Expr`] or an error with
/// positional context.
pub fn parse(input: &str) -> Result<Expr, QueryError> {
    let mut p = Parser::new(input);
    let expr = p.parse_or()?;
    p.skip_ws();
    if !p.at_end() {
        return Err(QueryError::UnexpectedTrailing {
            at: p.pos,
            rest: p.rest().to_string(),
        });
    }
    Ok(expr)
}

/// Errors surfaced by [`parse`]. Positions are byte offsets into the input string.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("empty query")]
    Empty,
    #[error("expected a predicate or '(' at position {at}")]
    ExpectedAtom { at: usize },
    #[error("expected tag (digits) at position {at}")]
    ExpectedTag { at: usize },
    #[error("expected operator (=, !=, ~) at position {at}")]
    ExpectedOp { at: usize },
    #[error("expected value at position {at}")]
    ExpectedValue { at: usize },
    #[error("unterminated string literal at position {at}")]
    UnterminatedString { at: usize },
    #[error("unbalanced parenthesis at position {at}")]
    UnbalancedParen { at: usize },
    #[error("invalid regex at position {at}: {source}")]
    BadRegex {
        at: usize,
        #[source]
        source: regex::Error,
    },
    #[error("unexpected trailing input at position {at}: {rest:?}")]
    UnexpectedTrailing { at: usize, rest: String },
    #[error("tag value at position {at} is not a u32")]
    TagOverflow { at: usize },
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            src: s.as_bytes(),
            pos: 0,
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn rest(&self) -> &str {
        // Safe: grammar operates on ASCII-only byte classes, and the input was &str.
        std::str::from_utf8(&self.src[self.pos..]).unwrap_or("")
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Try to consume keyword `kw` (case-insensitive) followed by whitespace/EOF/paren.
    /// Does not advance if it doesn't match.
    fn eat_keyword(&mut self, kw: &str) -> bool {
        let bytes = kw.as_bytes();
        let end = self.pos + bytes.len();
        if end > self.src.len() {
            return false;
        }
        let candidate = &self.src[self.pos..end];
        if !candidate.eq_ignore_ascii_case(bytes) {
            return false;
        }
        // Must be followed by whitespace, EOF, or an opening paren to avoid eating a
        // prefix of a bareword like `ANDROID`.
        match self.src.get(end) {
            None => {}
            Some(&b) if b.is_ascii_whitespace() || b == b'(' => {}
            _ => return false,
        }
        self.pos = end;
        true
    }

    fn parse_or(&mut self) -> Result<Expr, QueryError> {
        let mut left = self.parse_and()?;
        loop {
            self.skip_ws();
            if !self.eat_keyword("OR") {
                break;
            }
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, QueryError> {
        let mut left = self.parse_not()?;
        loop {
            self.skip_ws();
            if !self.eat_keyword("AND") {
                break;
            }
            let right = self.parse_not()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Expr, QueryError> {
        self.skip_ws();
        if self.eat_keyword("NOT") {
            let inner = self.parse_not()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<Expr, QueryError> {
        self.skip_ws();
        match self.peek() {
            None => Err(QueryError::Empty),
            Some(b'(') => {
                let open_at = self.pos;
                self.bump();
                let inner = self.parse_or()?;
                self.skip_ws();
                match self.bump() {
                    Some(b')') => Ok(inner),
                    _ => Err(QueryError::UnbalancedParen { at: open_at }),
                }
            }
            Some(b) if b.is_ascii_digit() => Ok(Expr::Pred(self.parse_predicate()?)),
            Some(_) => Err(QueryError::ExpectedAtom { at: self.pos }),
        }
    }

    fn parse_predicate(&mut self) -> Result<Predicate, QueryError> {
        let tag_start = self.pos;
        let tag = self.parse_tag()?;
        let op_at = self.pos;
        let op_kind = self.parse_op()?;
        let value_at = self.pos;
        let value = self.parse_value()?;

        let op = match op_kind {
            OpKind::Eq => Op::Eq,
            OpKind::Ne => Op::Ne,
            OpKind::Re => {
                let pattern = std::str::from_utf8(&value).map_err(|_| QueryError::BadRegex {
                    at: value_at,
                    source: regex::Error::Syntax("non-utf8 regex".into()),
                })?;
                let rx = regex::bytes::Regex::new(pattern).map_err(|e| QueryError::BadRegex {
                    at: value_at,
                    source: e,
                })?;
                Op::Re(std::sync::Arc::new(rx))
            }
        };
        // Silence unused warnings for the position locals in release builds — they're
        // only used above for diagnostics.
        let _ = (tag_start, op_at);

        Ok(Predicate { tag, op, value })
    }

    fn parse_tag(&mut self) -> Result<u32, QueryError> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(QueryError::ExpectedTag { at: start });
        }
        let digits = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
        digits
            .parse::<u32>()
            .map_err(|_| QueryError::TagOverflow { at: start })
    }

    fn parse_op(&mut self) -> Result<OpKind, QueryError> {
        let at = self.pos;
        match self.peek() {
            Some(b'=') => {
                self.bump();
                Ok(OpKind::Eq)
            }
            Some(b'!') if self.src.get(self.pos + 1) == Some(&b'=') => {
                self.pos += 2;
                Ok(OpKind::Ne)
            }
            Some(b'~') => {
                self.bump();
                Ok(OpKind::Re)
            }
            _ => Err(QueryError::ExpectedOp { at }),
        }
    }

    fn parse_value(&mut self) -> Result<Vec<u8>, QueryError> {
        let at = self.pos;
        match self.peek() {
            None => Err(QueryError::ExpectedValue { at }),
            Some(b'"') => self.parse_quoted(),
            Some(_) => Ok(self.parse_bareword()),
        }
    }

    fn parse_quoted(&mut self) -> Result<Vec<u8>, QueryError> {
        let open_at = self.pos;
        self.bump(); // eat opening quote
        let mut out = Vec::new();
        loop {
            match self.bump() {
                None => return Err(QueryError::UnterminatedString { at: open_at }),
                Some(b'"') => return Ok(out),
                Some(b'\\') => match self.bump() {
                    Some(esc) => out.push(esc),
                    None => return Err(QueryError::UnterminatedString { at: open_at }),
                },
                Some(b) => out.push(b),
            }
        }
    }

    fn parse_bareword(&mut self) -> Vec<u8> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() || matches!(b, b'(' | b')') {
                break;
            }
            self.pos += 1;
        }
        self.src[start..self.pos].to_vec()
    }
}

enum OpKind {
    Eq,
    Ne,
    Re,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(input: &str) -> Expr {
        parse(input).unwrap_or_else(|e| panic!("parse({input:?}) failed: {e}"))
    }

    fn err(input: &str) -> QueryError {
        parse(input).unwrap_err()
    }

    #[test]
    fn parses_single_predicate() {
        match ok("35=D") {
            Expr::Pred(p) => {
                assert_eq!(p.tag, 35);
                assert!(matches!(p.op, Op::Eq));
                assert_eq!(p.value, b"D");
            }
            other => panic!("expected Pred, got {other:?}"),
        }
    }

    #[test]
    fn parses_and_or_with_precedence() {
        // AND binds tighter than OR: `35=A OR 35=8 AND 55=AAPL` == `35=A OR (35=8 AND 55=AAPL)`
        match ok("35=A OR 35=8 AND 55=AAPL") {
            Expr::Or(_, right) => match *right {
                Expr::And(_, _) => {}
                other => panic!("right side must be And, got {other:?}"),
            },
            other => panic!("expected Or, got {other:?}"),
        }
    }

    #[test]
    fn parses_not_with_highest_precedence() {
        // `NOT 35=A AND 55=AAPL` == `(NOT 35=A) AND 55=AAPL`
        match ok("NOT 35=A AND 55=AAPL") {
            Expr::And(left, _) => match *left {
                Expr::Not(_) => {}
                other => panic!("left side must be Not, got {other:?}"),
            },
            other => panic!("expected And, got {other:?}"),
        }
    }

    #[test]
    fn parens_override_precedence() {
        match ok("(35=A OR 35=8) AND 55=AAPL") {
            Expr::And(left, _) => match *left {
                Expr::Or(_, _) => {}
                other => panic!("left side must be Or, got {other:?}"),
            },
            other => panic!("expected And, got {other:?}"),
        }
    }

    #[test]
    fn parses_regex_predicate() {
        let e = ok(r#"55~^AAP"#);
        let Expr::Pred(p) = e else {
            panic!("expected Pred");
        };
        assert!(matches!(p.op, Op::Re(_)));
        assert_eq!(p.tag, 55);
    }

    #[test]
    fn parses_quoted_value_with_spaces_and_escapes() {
        let e = ok(r#"58="hello \"world\"""#);
        let Expr::Pred(p) = e else {
            panic!("expected Pred");
        };
        assert_eq!(p.value, br#"hello "world""#);
    }

    #[test]
    fn keywords_are_case_insensitive() {
        assert!(matches!(ok("35=D and 55=AAPL"), Expr::And(_, _)));
        assert!(matches!(ok("35=D oR 35=8"), Expr::Or(_, _)));
        assert!(matches!(ok("not 35=D"), Expr::Not(_)));
    }

    #[test]
    fn rejects_missing_tag() {
        assert!(matches!(err("=D"), QueryError::ExpectedAtom { .. }));
    }

    #[test]
    fn rejects_missing_op() {
        assert!(matches!(err("35 "), QueryError::ExpectedOp { .. }));
    }

    #[test]
    fn rejects_unbalanced_paren() {
        assert!(matches!(err("(35=D"), QueryError::UnbalancedParen { .. }));
    }

    #[test]
    fn rejects_unterminated_string() {
        assert!(matches!(
            err(r#"58="hello"#),
            QueryError::UnterminatedString { .. }
        ));
    }

    #[test]
    fn rejects_bad_regex() {
        assert!(matches!(err("55~["), QueryError::BadRegex { .. }));
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(matches!(
            err("35=D junk"),
            QueryError::UnexpectedTrailing { .. }
        ));
    }
}
