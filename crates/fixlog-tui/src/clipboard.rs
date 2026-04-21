//! Thin wrapper over `arboard`. Failures (no display server, permission
//! denied, SSH-without-forwarding) are surfaced as `Err(String)` for the
//! status bar rather than propagated up — the TUI must never crash just
//! because the user can't copy.

use arboard::Clipboard;

/// Copy `text` to the system clipboard. Returns a human-readable error
/// string on failure.
pub fn copy(text: &str) -> Result<(), String> {
    let mut cb = Clipboard::new().map_err(|e| format!("clipboard unavailable: {e}"))?;
    cb.set_text(text)
        .map_err(|e| format!("clipboard write failed: {e}"))
}

/// Render a message's raw bytes as clipboard-safe text: SOH becomes `|`,
/// non-printable bytes become `.`. Loosely matches what `fixlog parse
/// --format pretty` shows on screen.
pub fn raw_to_text(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            0x01 => out.push('|'),
            c if c.is_ascii_graphic() || c == b' ' => out.push(c as char),
            _ => out.push('.'),
        }
    }
    out
}

/// Pretty-print a resolved message as multi-line text suitable for pasting
/// into a bug report or a ticket. Matches the shape of `fixlog parse
/// --format pretty` in `fixlog-cli`.
pub fn pretty_text(resolved: &crate::state::ResolvedMessageOwned) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Message @ offset {} — {}\n",
        resolved.offset,
        resolved.msg_type_name.unwrap_or("?")
    ));
    for f in &resolved.fields {
        let value = raw_to_text(&f.value);
        let name = f.name.unwrap_or("?");
        let ty = f.field_type.unwrap_or("-");
        let decoded = match f.value_label {
            Some(l) => format!(" ({l})"),
            None => String::new(),
        };
        out.push_str(&format!(
            "  {:>5}  {:<20} {:<10}  {}{}\n",
            f.tag, name, ty, value, decoded
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_to_text_replaces_soh() {
        assert_eq!(raw_to_text(b"8=FIX.4.4\x01"), "8=FIX.4.4|");
    }

    #[test]
    fn raw_to_text_scrubs_controls() {
        assert_eq!(raw_to_text(b"a\x00b\x1Fc"), "a.b.c");
    }

    #[test]
    fn pretty_text_includes_header_and_fields() {
        use crate::state::{ResolvedFieldOwned, ResolvedMessageOwned};
        let msg = ResolvedMessageOwned {
            offset: 42,
            msg_type_name: Some("NewOrderSingle"),
            fields: vec![ResolvedFieldOwned {
                tag: 35,
                name: Some("MsgType"),
                field_type: Some("char"),
                value: b"D".to_vec(),
                value_label: Some("NEW_ORDER_SINGLE"),
                depth: 0,
            }],
        };
        let out = pretty_text(&msg);
        assert!(out.contains("offset 42"));
        assert!(out.contains("NewOrderSingle"));
        assert!(out.contains("MsgType"));
        assert!(out.contains("NEW_ORDER_SINGLE"));
    }
}
