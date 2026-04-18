//! Vim-like forward/reverse search over the currently-visible messages.
//!
//! Reuses `fixlog-query` as the match engine — so `/35=D AND 54=1` finds
//! NewOrderSingle buys without introducing a separate search grammar.
//! Search does **not** filter the list; it only moves the cursor. This
//! matches vim's `/` semantics and keeps the filter/search concepts
//! orthogonal.

use fixlog_core::QueryExpr;
use fixlog_core::parser::parse_one_with_format;

use crate::state::AppState;

/// Direction for `n` / `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// Outcome of a single search step. Used by the event loop to set status
/// messages ("wrapped", "no match").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Found a match; cursor is now on `pos` (index into `visible`).
    Match { pos: usize, wrapped: bool },
    /// No row in `visible` matches the expression.
    NoMatch,
}

/// Advance the cursor to the next row in `visible` whose message matches
/// `expr` (skipping the current row). Wraps around the end of the list and
/// reports that via `Hit::Match { wrapped: true }`.
pub fn next_match(state: &mut AppState, expr: &QueryExpr, dir: Direction) -> Hit {
    let len = state.visible.len();
    if len == 0 {
        return Hit::NoMatch;
    }

    let start = state.cursor;
    for step in 1..=len {
        let i = match dir {
            Direction::Forward => (start + step) % len,
            Direction::Backward => (start + len - (step % len)) % len,
        };
        let ord = state.visible[i] as usize;
        let Some(bytes) = state.index.message_bytes(&state.mmap, ord) else {
            continue;
        };
        let Ok((msg, _)) = parse_one_with_format(bytes, &state.format) else {
            continue;
        };
        if expr.matches(&msg) {
            let wrapped = match dir {
                Direction::Forward => i <= start,
                Direction::Backward => i >= start,
            };
            state.cursor = i;
            return Hit::Match { pos: i, wrapped };
        }
    }
    Hit::NoMatch
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use fixlog_core::query::parse as parse_query;

    use crate::state::bootstrap;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/real")).join(name)
    }

    #[test]
    fn next_match_moves_cursor_to_first_match_after_start() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.cursor = 0;

        let expr = parse_query("35=D").expect("parse");
        let hit = next_match(&mut state, &expr, Direction::Forward);
        assert!(matches!(hit, Hit::Match { wrapped: false, .. }));
        // Re-parse the message at the new cursor and confirm 35=D.
        let ord = state.visible[state.cursor] as usize;
        let bytes = state.index.message_bytes(&state.mmap, ord).unwrap();
        let (msg, _) = parse_one_with_format(bytes, &state.format).unwrap();
        let mt = msg.tags.iter().find(|(t, _)| *t == 35).map(|(_, v)| *v);
        assert_eq!(mt, Some(b"D".as_ref()));
    }

    #[test]
    fn next_match_wraps_when_past_end() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        // Jump to the end; a forward search must wrap.
        state.cursor = state.visible.len() - 1;
        let expr = parse_query("35=D").expect("parse");
        let hit = next_match(&mut state, &expr, Direction::Forward);
        assert!(matches!(hit, Hit::Match { wrapped: true, .. }));
    }

    #[test]
    fn next_match_returns_no_match_for_unknown_msgtype() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        let expr = parse_query("35=ZZZZ").expect("parse");
        let hit = next_match(&mut state, &expr, Direction::Forward);
        assert_eq!(hit, Hit::NoMatch);
    }

    #[test]
    fn backward_finds_a_different_match() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        let expr = parse_query("35=D").expect("parse");

        // Forward search from start gives us the first D.
        state.cursor = 0;
        let fwd = next_match(&mut state, &expr, Direction::Forward);
        let Hit::Match { pos: first, .. } = fwd else {
            panic!("expected match");
        };

        // Backward from `first` must find another D. Whether it wraps or
        // not depends on fixture contents — we just need it to succeed and
        // land on a different row than `first`.
        let back = next_match(&mut state, &expr, Direction::Backward);
        let Hit::Match { pos, .. } = back else {
            panic!("expected backward match, got {back:?}");
        };
        assert_ne!(pos, first, "backward should move off the current match");
    }
}
