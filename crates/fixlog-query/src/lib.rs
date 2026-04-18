#![forbid(unsafe_code)]

//! Filter DSL for FIX messages.
//!
//! The DSL lets the CLI do `fixlog grep <file> --filter "35=D AND 55=AAPL"` against the
//! zero-copy `RawMessage`s produced by `fixlog-parser`. The grammar is deliberately small
//! (`=`, `!=`, `~`, `AND`, `OR`, `NOT`, parentheses) because a FIX log is flat enough that
//! a conjunction of tag/value predicates covers ~all the post-mortem questions we care
//! about.
//!
//! ```text
//! expr      := or_expr
//! or_expr   := and_expr ( "OR"  and_expr )*
//! and_expr  := not_expr ( "AND" not_expr )*
//! not_expr  := "NOT" not_expr | atom
//! atom      := predicate | "(" expr ")"
//! predicate := digit+ op value
//! op        := "=" | "!=" | "~"
//! value     := "\"" …quoted bytes… "\"" | bareword   ; bareword = non-space, non-operator
//! ```
//!
//! Precedence is standard: `NOT` > `AND` > `OR`. Regex (`~`) uses `regex::bytes` so it
//! works on raw tag values without requiring UTF-8 validity.
//!
//! Lifetime note: [`Expr`] owns its regexes and value bytes. You parse once, then
//! [`Expr::matches`] against many `RawMessage`s.

pub mod ast;
pub mod eval;
pub mod parser;

pub use ast::{Expr, Op, Predicate};
pub use parser::{QueryError, parse};
