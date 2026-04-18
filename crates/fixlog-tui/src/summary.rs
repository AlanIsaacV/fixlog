//! Per-MsgType summarizer producing display-ready strings for the list column
//! layout.
//!
//! The table is hardcoded here (not in `fixlog-dict`) so the parser stays
//! dict-agnostic — the dictionary resolves *labels* for us; the shape of the
//! summary (which tags to pull, which prefix to use) is a TUI concern.
//!
//! Unknown MsgTypes fall through to a generic "Side Qty Symbol [@ Price]"
//! summary built from well-known order/exec tags.
//!
//! Badges are `Cow<'static, str>` so dictionary labels — returned as
//! `&'static str` from the generated `fixlog-dict` tables — can be yielded
//! without a heap copy; fallbacks borrow-as-owned.

use std::borrow::Cow;

use fixlog_core::dict::{DictChain, chain_enum_value_label};
use fixlog_core::parser::RawMessage;

/// ClOrdID (11).
pub const TAG_CL_ORD_ID: u32 = 11;
/// MsgType (35).
pub const TAG_MSG_TYPE: u32 = 35;
/// OrderQty (38).
pub const TAG_ORDER_QTY: u32 = 38;
/// OrdStatus (39).
pub const TAG_ORD_STATUS: u32 = 39;
/// Price (44).
pub const TAG_PRICE: u32 = 44;
/// Side (54).
pub const TAG_SIDE: u32 = 54;
/// Symbol (55).
pub const TAG_SYMBOL: u32 = 55;
/// Text (58) — used for Reject / BusinessMessageReject detail.
pub const TAG_TEXT: u32 = 58;
/// ExecType (150).
pub const TAG_EXEC_TYPE: u32 = 150;

/// Max display-width of the truncated `Text(58)` value in reject summaries.
/// Matches the usual terminal column width without crowding the other
/// columns.
const DETAIL_MAX_TEXT: usize = 60;

/// Display-ready projection of a FIX message for the list view.
///
/// All strings are already formatted (enum labels resolved, thousands
/// separators applied, price preserved verbatim) so the renderer can drop
/// them straight into `Span::raw`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MessageSummary {
    /// Short tags rendered next to the MsgType column. Order matters: the
    /// first badge is the most important (e.g. `ExecType` before
    /// `OrdStatus` for ExecutionReport).
    pub badges: Vec<Cow<'static, str>>,
    /// Tag 11. Populated from the same source for every MsgType; centralised
    /// here so callers don't re-read.
    pub client_order_id: Option<String>,
    /// Free-form detail column. `None` for noise messages (Heartbeat, Logon,
    /// Logout, TestRequest, ResendRequest).
    pub detail: Option<String>,
}

/// Produce a [`MessageSummary`] for `msg` using `chain` to resolve enum
/// labels (Side, ExecType, OrdStatus).
pub fn summarize(msg: &RawMessage<'_>, chain: DictChain) -> MessageSummary {
    let msg_type = lookup_tag(msg, TAG_MSG_TYPE);
    let client_order_id = lookup_tag_string(msg, TAG_CL_ORD_ID);

    let (badges, detail) = match msg_type {
        Some(b"D") => (Vec::new(), order_detail(msg, chain, true)),
        Some(b"8") => (exec_badges(msg, chain), order_detail(msg, chain, false)),
        Some(b"F") => (Vec::new(), order_detail(msg, chain, false)),
        Some(b"G") => (Vec::new(), order_detail(msg, chain, true)),
        Some(b"3") | Some(b"j") => (Vec::new(), reject_text(msg)),
        // Session-layer / noise: no detail.
        Some(b"A") | Some(b"5") | Some(b"0") | Some(b"1") | Some(b"2") => (Vec::new(), None),
        // Unknown MsgType — keep the generic fallback so market data etc.
        // still show Side/Qty/Symbol when available.
        _ => (Vec::new(), order_detail(msg, chain, true)),
    };

    MessageSummary {
        badges,
        client_order_id,
        detail,
    }
}

/// ExecutionReport badges: `ExecType` then `OrdStatus`, each resolved to its
/// dictionary label when known.
fn exec_badges(msg: &RawMessage<'_>, chain: DictChain) -> Vec<Cow<'static, str>> {
    let mut out = Vec::new();
    if let Some(v) = lookup_tag(msg, TAG_EXEC_TYPE) {
        out.push(enum_label(chain, TAG_EXEC_TYPE, v));
    }
    if let Some(v) = lookup_tag(msg, TAG_ORD_STATUS) {
        out.push(enum_label(chain, TAG_ORD_STATUS, v));
    }
    out
}

fn enum_label(chain: DictChain, tag: u32, value: &[u8]) -> Cow<'static, str> {
    match chain_enum_value_label(chain, tag, value) {
        Some(label) => Cow::Borrowed(label),
        None => Cow::Owned(String::from_utf8_lossy(value).into_owned()),
    }
}

fn reject_text(msg: &RawMessage<'_>) -> Option<String> {
    let raw = lookup_tag(msg, TAG_TEXT)?;
    let text = String::from_utf8_lossy(raw).into_owned();
    if text.chars().count() > DETAIL_MAX_TEXT {
        let mut truncated: String = text.chars().take(DETAIL_MAX_TEXT).collect();
        truncated.push('…');
        Some(truncated)
    } else {
        Some(text)
    }
}

/// Compose `Side Qty Symbol [@ Price]`. `include_price` controls whether
/// the price is appended — `false` for ExecutionReport/OrderCancelRequest
/// where the order price is less relevant than the execution details.
fn order_detail(msg: &RawMessage<'_>, chain: DictChain, include_price: bool) -> Option<String> {
    let side =
        lookup_tag(msg, TAG_SIDE).map(|v| match chain_enum_value_label(chain, TAG_SIDE, v) {
            Some(label) => label.to_string(),
            None => String::from_utf8_lossy(v).into_owned(),
        });
    let qty = lookup_tag(msg, TAG_ORDER_QTY).map(format_qty);
    let symbol = lookup_tag_string(msg, TAG_SYMBOL);
    let price = if include_price {
        lookup_tag(msg, TAG_PRICE).map(|v| String::from_utf8_lossy(v).into_owned())
    } else {
        None
    };

    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = side {
        parts.push(s);
    }
    if let Some(q) = qty {
        parts.push(q);
    }
    if let Some(s) = symbol {
        parts.push(s);
    }

    let mut out = parts.join(" ");
    if let Some(p) = price {
        if out.is_empty() {
            out = format!("@ {p}");
        } else {
            out = format!("{out} @ {p}");
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// First occurrence of `tag` in `msg.tags`, as a borrowed byte slice.
pub(crate) fn lookup_tag<'a>(msg: &'a RawMessage<'_>, tag: u32) -> Option<&'a [u8]> {
    msg.tags.iter().find(|(t, _)| *t == tag).map(|(_, v)| *v)
}

/// First occurrence of `tag` materialised as an owned UTF-8 string
/// (lossy-converting non-UTF-8 bytes).
pub(crate) fn lookup_tag_string(msg: &RawMessage<'_>, tag: u32) -> Option<String> {
    lookup_tag(msg, tag).map(|v| String::from_utf8_lossy(v).into_owned())
}

/// Insert thousands separators in an ASCII integer string. Leaves the
/// decimal portion (if any) untouched. Preserves a leading sign. Falls
/// back to the raw text when bytes aren't valid UTF-8 digits.
pub(crate) fn format_qty(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(bytes).into_owned();
    let (int_part, frac) = match raw.find('.') {
        Some(i) => (&raw[..i], Some(&raw[i..])),
        None => (raw.as_str(), None),
    };
    let (sign, digits) = match int_part.strip_prefix('-') {
        Some(r) => ("-", r),
        None => int_part
            .strip_prefix('+')
            .map(|r| ("+", r))
            .unwrap_or(("", int_part)),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return raw;
    }
    let len = digits.len();
    let mut grouped = String::with_capacity(len + len / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i != 0 && (len - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let mut out =
        String::with_capacity(sign.len() + grouped.len() + frac.map(|s| s.len()).unwrap_or(0));
    out.push_str(sign);
    out.push_str(&grouped);
    if let Some(f) = frac {
        out.push_str(f);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixlog_core::dict::CHAIN_FIX44;

    fn raw(tags: &[(u32, &'static [u8])]) -> RawMessage<'static> {
        RawMessage {
            offset: 0,
            raw: b"",
            tags: tags.iter().copied().collect(),
        }
    }

    #[test]
    fn summarize_new_order_single_includes_price() {
        let msg = raw(&[
            (TAG_MSG_TYPE, b"D"),
            (TAG_CL_ORD_ID, b"ORD-1"),
            (TAG_SIDE, b"1"),
            (TAG_ORDER_QTY, b"10000"),
            (TAG_SYMBOL, b"SPY"),
            (TAG_PRICE, b"150.25"),
        ]);
        let s = summarize(&msg, CHAIN_FIX44);
        assert_eq!(s.client_order_id.as_deref(), Some("ORD-1"));
        assert!(s.badges.is_empty());
        assert_eq!(s.detail.as_deref(), Some("BUY 10,000 SPY @ 150.25"));
    }

    #[test]
    fn summarize_execution_report_has_badges_and_no_price() {
        let msg = raw(&[
            (TAG_MSG_TYPE, b"8"),
            (TAG_CL_ORD_ID, b"ORD-1"),
            (TAG_EXEC_TYPE, b"F"),  // TRADE
            (TAG_ORD_STATUS, b"2"), // FILLED
            (TAG_SIDE, b"1"),
            (TAG_ORDER_QTY, b"500"),
            (TAG_SYMBOL, b"AAPL"),
            (TAG_PRICE, b"210"),
        ]);
        let s = summarize(&msg, CHAIN_FIX44);
        assert_eq!(s.client_order_id.as_deref(), Some("ORD-1"));
        assert_eq!(s.badges.len(), 2);
        assert_eq!(s.detail.as_deref(), Some("BUY 500 AAPL"));
    }

    #[test]
    fn summarize_cancel_request_no_price() {
        let msg = raw(&[
            (TAG_MSG_TYPE, b"F"),
            (TAG_CL_ORD_ID, b"CXL-1"),
            (TAG_SIDE, b"2"),
            (TAG_ORDER_QTY, b"100"),
            (TAG_SYMBOL, b"MSFT"),
            (TAG_PRICE, b"300"), // present but ignored for F
        ]);
        let s = summarize(&msg, CHAIN_FIX44);
        assert_eq!(s.detail.as_deref(), Some("SELL 100 MSFT"));
    }

    #[test]
    fn summarize_cancel_replace_keeps_price() {
        let msg = raw(&[
            (TAG_MSG_TYPE, b"G"),
            (TAG_SIDE, b"1"),
            (TAG_ORDER_QTY, b"200"),
            (TAG_SYMBOL, b"TSLA"),
            (TAG_PRICE, b"250.5"),
        ]);
        let s = summarize(&msg, CHAIN_FIX44);
        assert_eq!(s.detail.as_deref(), Some("BUY 200 TSLA @ 250.5"));
    }

    #[test]
    fn summarize_reject_truncates_text() {
        let long = "E".repeat(120);
        let leaked: &'static str = Box::leak(long.into_boxed_str());
        let msg = raw(&[(TAG_MSG_TYPE, b"3"), (TAG_TEXT, leaked.as_bytes())]);
        let s = summarize(&msg, CHAIN_FIX44);
        let detail = s.detail.expect("reject detail");
        assert_eq!(detail.chars().count(), DETAIL_MAX_TEXT + 1);
        assert!(detail.ends_with('…'));
    }

    #[test]
    fn summarize_business_reject_uses_text() {
        let msg = raw(&[(TAG_MSG_TYPE, b"j"), (TAG_TEXT, b"unsupported MsgType")]);
        let s = summarize(&msg, CHAIN_FIX44);
        assert_eq!(s.detail.as_deref(), Some("unsupported MsgType"));
    }

    #[test]
    fn summarize_session_noise_has_no_detail() {
        for mt in [b"A", b"5", b"0", b"1", b"2"] {
            let msg = raw(&[(TAG_MSG_TYPE, mt.as_slice())]);
            let s = summarize(&msg, CHAIN_FIX44);
            assert!(
                s.detail.is_none(),
                "MsgType {:?} should have empty detail, got {:?}",
                String::from_utf8_lossy(mt),
                s.detail
            );
            assert!(s.badges.is_empty());
        }
    }

    #[test]
    fn summarize_unknown_msgtype_falls_back_to_order_detail() {
        // W (MarketDataSnapshot) isn't in the hardcoded table — should still
        // pick up whatever order-shaped tags happen to be present.
        let msg = raw(&[
            (TAG_MSG_TYPE, b"W"),
            (TAG_SYMBOL, b"EURUSD"),
            (TAG_PRICE, b"1.0815"),
        ]);
        let s = summarize(&msg, CHAIN_FIX44);
        assert_eq!(s.detail.as_deref(), Some("EURUSD @ 1.0815"));
    }

    #[test]
    fn format_qty_inserts_thousands_separators() {
        assert_eq!(format_qty(b"10000"), "10,000");
        assert_eq!(format_qty(b"1000000"), "1,000,000");
        assert_eq!(format_qty(b"100"), "100");
        assert_eq!(format_qty(b"1"), "1");
        assert_eq!(format_qty(b"-12345"), "-12,345");
        assert_eq!(format_qty(b"1234.56"), "1,234.56");
        assert_eq!(format_qty(b"not-a-number"), "not-a-number");
    }
}
