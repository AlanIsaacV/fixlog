# `fixlog-query`

Small filter DSL that matches a parsed `RawMessage` against an expression like
`35=D AND 55=AAPL`. Lives at `crates/fixlog-query/`.

## Grammar

```
expr      := or_expr
or_expr   := and_expr ("OR"  and_expr)*
and_expr  := not_expr ("AND" not_expr)*
not_expr  := "NOT" not_expr | atom
atom      := predicate | "(" expr ")"
predicate := digit+ op value
op        := "=" | "!=" | "~"
value     := bareword | "\"" …bytes… "\""      ; bareword = non-space, non-paren
```

- Keywords are case-insensitive: `AND`/`and`, `OR`/`or`, `NOT`/`not`.
- Precedence: `NOT` > `AND` > `OR`. Parenthesize to override.
- `~` is a regex match via `regex::bytes` — compiled once at parse time, not per message.
- Quoted strings support `\"` and `\\` escapes.

## Public surface

- `pub fn parse(&str) -> Result<Expr, QueryError>` — positional errors include byte offset.
- `pub enum Expr { Pred(Predicate), Not, And, Or }`.
- `Expr::matches(&self, &RawMessage) -> bool` — zero-alloc evaluator.

## Evaluator semantics (INVARIANT)

- `Eq` / `Re`: true iff **any** occurrence of the tag matches — matches `grep` intuition
  when tags repeat (repeating groups).
- `Ne`: true iff the tag is absent *or* no occurrence equals the value. Note this is
  stricter than "some occurrence differs" on repeating tags; callers who need the latter
  should rewrite with `NOT (tag = value)` explicitly… which is the same thing. Current
  semantics match the idiomatic expectation for `grep -v`.
- Short-circuiting on `AND`/`OR` — the right side is skipped if the left is conclusive.

## REALITY (vs the original design intent)

- No dictionary integration yet. The DSL is tag-number only; `MsgType=NewOrderSingle`
  isn't supported. Adding it requires the query crate to depend on `fixlog-dict`, which
  we'll only do when it's demanded by a concrete user path (probably alongside the TUI).
- `QueryError` doesn't impl `PartialEq` because `regex::Error` doesn't. Use `matches!`
  in tests.

## Tests

- 18 unit (parser + evaluator + precedence + regex + errors).
- e2e coverage via the CLI in `fixlog-cli/tests/grep.rs` (4 tests).

## When to modify this crate

- **Add an operator**: touch the grammar in `parser.rs` + matching arm in `eval.rs` +
  docstring above.
- **Change repeating-group semantics**: `eval_predicate` in `eval.rs`. Update the
  invariant section above too.
- **Relax `Ne` semantics**: search for `eval_predicate` and the "N!=X" test case in
  `eval::tests`.
