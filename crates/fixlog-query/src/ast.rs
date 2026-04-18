//! AST types for parsed filter expressions.
//!
//! The evaluator lives in [`crate::eval`]. Keeping the AST separate makes it trivial to
//! serialize or transform later (e.g. push-down to a secondary index).

use std::sync::Arc;

use fixlog_parser::RawMessage;

/// Comparison operator inside a [`Predicate`].
///
/// `Op::Re` holds its `Regex` behind an [`Arc`] so the whole AST (`Op` →
/// [`Predicate`] → [`Expr`]) can derive `Clone` cheaply. Cloning a regex
/// otherwise requires re-compilation (≈ 100 µs for typical patterns); with
/// `Arc` it's an atomic refcount bump.
#[derive(Debug, Clone)]
pub enum Op {
    /// `tag = value` — byte-exact equality.
    Eq,
    /// `tag != value` — logical inverse of `Eq`. Missing tag counts as "not equal".
    Ne,
    /// `tag ~ regex` — regex match on the raw tag value bytes.
    Re(Arc<regex::bytes::Regex>),
}

/// A single `tag <op> value` atom.
#[derive(Debug, Clone)]
pub struct Predicate {
    pub tag: u32,
    pub op: Op,
    /// Byte-exact value for `Eq`/`Ne`. Ignored for `Re` (the regex carries the pattern).
    pub value: Vec<u8>,
}

/// Parsed filter expression.
#[derive(Debug, Clone)]
pub enum Expr {
    Pred(Predicate),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

impl Expr {
    /// Evaluate this expression against a single message.
    ///
    /// Zero allocations on the hot path: we walk the tag list linearly and short-circuit
    /// boolean operators. Regexes are pre-compiled at parse time.
    #[inline]
    pub fn matches(&self, msg: &RawMessage<'_>) -> bool {
        crate::eval::matches(self, msg)
    }

    /// If this expression is a pure AND-of-`Eq`-predicates, return the
    /// list of `(tag, value)` pairs. Otherwise return `None`.
    ///
    /// This is the shape the secondary index can intersect in O(output)
    /// instead of a full scan. Any `Or`, `Not`, `Ne`, or `Re` node
    /// disqualifies the expression — partial pushdowns are out of scope
    /// (P4-T14 decision log).
    pub fn hot_equalities(&self) -> Option<Vec<(u32, &[u8])>> {
        let mut out = Vec::new();
        if self.collect_hot_equalities(&mut out) {
            Some(out)
        } else {
            None
        }
    }

    fn collect_hot_equalities<'a>(&'a self, out: &mut Vec<(u32, &'a [u8])>) -> bool {
        match self {
            Expr::Pred(p) => match p.op {
                Op::Eq => {
                    out.push((p.tag, p.value.as_slice()));
                    true
                }
                Op::Ne | Op::Re(_) => false,
            },
            Expr::And(a, b) => a.collect_hot_equalities(out) && b.collect_hot_equalities(out),
            Expr::Not(_) | Expr::Or(_, _) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::parse;

    #[test]
    fn hot_equalities_single_eq() {
        let e = parse("35=D").unwrap();
        let hot = e.hot_equalities().unwrap();
        assert_eq!(hot.len(), 1);
        assert_eq!(hot[0].0, 35);
        assert_eq!(hot[0].1, b"D");
    }

    #[test]
    fn hot_equalities_and_chain() {
        let e = parse("35=D AND 49=A AND 56=B").unwrap();
        let hot = e.hot_equalities().unwrap();
        assert_eq!(hot.len(), 3);
        assert_eq!(hot[0].0, 35);
        assert_eq!(hot[1].0, 49);
        assert_eq!(hot[2].0, 56);
    }

    #[test]
    fn hot_equalities_rejects_or() {
        let e = parse("35=D OR 35=8").unwrap();
        assert!(e.hot_equalities().is_none());
    }

    #[test]
    fn hot_equalities_rejects_not() {
        let e = parse("NOT 35=D").unwrap();
        assert!(e.hot_equalities().is_none());
    }

    #[test]
    fn hot_equalities_rejects_ne() {
        let e = parse("35!=D").unwrap();
        assert!(e.hot_equalities().is_none());
    }

    #[test]
    fn hot_equalities_rejects_regex() {
        let e = parse("55~^MS").unwrap();
        assert!(e.hot_equalities().is_none());
    }

    #[test]
    fn expr_is_clone_even_when_it_contains_a_regex() {
        // Clone is cheap for all variants — the regex sits behind an `Arc`,
        // so cloning is a refcount bump rather than a re-compile. This is
        // what `FilterSnapshot` and `search_last` rely on to stop
        // re-parsing on every toggle / `n` / `N`.
        fn assert_clone<T: Clone>(_: &T) {}
        let e = parse("(55~^MS AND 35=D) OR NOT 35=0").unwrap();
        assert_clone(&e);
        let _ = e.clone();
    }
}
