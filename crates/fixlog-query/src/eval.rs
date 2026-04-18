//! Evaluator: does a parsed [`Expr`] match a given `RawMessage`?
//!
//! Design goals, in priority order:
//! 1. **No allocations**: every predicate look-up is a linear scan over
//!    `msg.tags`, short-circuiting on the first hit. No hashmaps, no clones.
//! 2. **Short-circuit AND/OR**: obviously-required for correctness and speed.
//! 3. **Match any occurrence**: if a tag appears multiple times (repeating groups),
//!    `Eq`/`Re` match if any occurrence satisfies the predicate. `Ne` holds if the
//!    tag is absent or none of its occurrences equal the value. This matches `grep`
//!    semantics — "show me messages that mention AAPL anywhere" is the common intent.

use crate::ast::{Expr, Op, Predicate};
use fixlog_parser::RawMessage;

/// Evaluate `expr` against `msg`.
pub fn matches(expr: &Expr, msg: &RawMessage<'_>) -> bool {
    match expr {
        Expr::Pred(pred) => eval_predicate(pred, msg),
        Expr::Not(inner) => !matches(inner, msg),
        Expr::And(a, b) => matches(a, msg) && matches(b, msg),
        Expr::Or(a, b) => matches(a, msg) || matches(b, msg),
    }
}

fn eval_predicate(pred: &Predicate, msg: &RawMessage<'_>) -> bool {
    match &pred.op {
        Op::Eq => msg
            .tags
            .iter()
            .any(|(t, v)| *t == pred.tag && *v == pred.value.as_slice()),
        Op::Ne => !msg
            .tags
            .iter()
            .any(|(t, v)| *t == pred.tag && *v == pred.value.as_slice()),
        Op::Re(rx) => msg
            .tags
            .iter()
            .any(|(t, v)| *t == pred.tag && rx.is_match(v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use fixlog_parser::{parse_one, validator};

    fn logon_with_msgtype(msg_type: &str, extra: &[u8]) -> Vec<u8> {
        let mut body = format!("35={msg_type}\x0149=S\x0156=T\x01").into_bytes();
        body.extend_from_slice(extra);
        let prefix = format!("8=FIX.4.4\x019={}\x01", body.len());
        let mut msg: Vec<u8> = prefix.into_bytes();
        msg.extend_from_slice(&body);
        let cs = validator::compute_checksum(&msg);
        msg.extend_from_slice(format!("10={cs:03}\x01").as_bytes());
        msg
    }

    fn parse_msg(bytes: &[u8]) -> RawMessage<'_> {
        parse_one(bytes).expect("valid fixture").0
    }

    #[test]
    fn eq_matches_and_misses() {
        let bytes = logon_with_msgtype("D", b"55=AAPL\x01");
        let msg = parse_msg(&bytes);
        assert!(parse("35=D").unwrap().matches(&msg));
        assert!(!parse("35=A").unwrap().matches(&msg));
        assert!(parse("55=AAPL").unwrap().matches(&msg));
    }

    #[test]
    fn ne_matches_when_tag_absent_or_different() {
        let bytes = logon_with_msgtype("D", b"");
        let msg = parse_msg(&bytes);
        // 55 is absent → treated as "not equal to AAPL".
        assert!(parse("55!=AAPL").unwrap().matches(&msg));
        // 35 is present but != "A".
        assert!(parse("35!=A").unwrap().matches(&msg));
        // 35 equals D, so the predicate is false.
        assert!(!parse("35!=D").unwrap().matches(&msg));
    }

    #[test]
    fn regex_matches_on_raw_bytes() {
        let bytes = logon_with_msgtype("D", b"55=MSFT\x01");
        let msg = parse_msg(&bytes);
        assert!(parse(r#"55~^MS"#).unwrap().matches(&msg));
        assert!(!parse(r#"55~^AAPL$"#).unwrap().matches(&msg));
    }

    #[test]
    fn and_or_not_combine_correctly() {
        let bytes = logon_with_msgtype("D", b"55=AAPL\x0154=1\x01");
        let msg = parse_msg(&bytes);
        assert!(parse("35=D AND 55=AAPL").unwrap().matches(&msg));
        assert!(!parse("35=D AND 55=MSFT").unwrap().matches(&msg));
        assert!(parse("35=A OR 55=AAPL").unwrap().matches(&msg));
        assert!(parse("NOT 35=A").unwrap().matches(&msg));
        assert!(!parse("NOT 35=D").unwrap().matches(&msg));
        assert!(parse("(35=D OR 35=8) AND NOT 54=2").unwrap().matches(&msg));
    }

    #[test]
    fn repeating_tag_matches_on_any_occurrence() {
        // 448 (PartyID) appears twice — classic repeating group case.
        let bytes = logon_with_msgtype("D", b"448=BROKER1\x01448=EXCH\x01");
        let msg = parse_msg(&bytes);
        assert!(parse("448=EXCH").unwrap().matches(&msg));
        assert!(parse("448=BROKER1").unwrap().matches(&msg));
        assert!(!parse("448=OTHER").unwrap().matches(&msg));
        // For Ne: some occurrence equals BROKER1, so Ne(BROKER1) is false
        // regardless of other occurrences. This is the intended semantic — see
        // module docs.
        assert!(!parse("448!=BROKER1").unwrap().matches(&msg));
    }
}
