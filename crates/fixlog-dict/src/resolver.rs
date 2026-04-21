//! Upgrade a [`RawMessage`] into a [`ResolvedMessage`] by decorating each
//! tag with its dictionary name and, when applicable, its enum-value label.

use fixlog_parser::{RawMessage, TAG_BEGIN_STRING, TAG_MSG_TYPE};

use crate::{
    DictChain, FixVersion, chain_enum_value_label, chain_field_by_tag, chain_for,
    chain_msg_type_label, group_members,
};

/// Tag `1128` — `ApplVerID`. Carried on FIXT.1.1 Logon (and some app messages)
/// to indicate which FIX application-layer dictionary to use.
pub const TAG_APPL_VER_ID: u32 = 1128;

/// A single field pair with dictionary metadata attached.
///
/// `value` still borrows from the original buffer, so resolution is
/// allocation-free per field. `name` and `value_label` are static strings from
/// the generated dictionary.
#[derive(Debug, Clone)]
pub struct ResolvedField<'a> {
    /// Tag number as seen on the wire.
    pub tag: u32,
    /// Field name from the dictionary chain, or `None` for custom/unknown tags.
    pub name: Option<&'static str>,
    /// Raw value bytes (borrowed from the source buffer).
    pub value: &'a [u8],
    /// Human-readable label for enum-valued fields. `None` for non-enum fields
    /// or when the wire value is not listed in the dictionary.
    pub value_label: Option<&'static str>,
    /// Nesting depth inside a repeating group. `0` at top level; `1` for
    /// members of a top-level group (all fields between a `NumInGroup`
    /// counter and the first non-member tag). Deeper nesting is reserved
    /// for future use — the current resolver only flattens to a single
    /// layer.
    pub depth: u8,
}

/// A raw message after dictionary resolution.
#[derive(Debug, Clone)]
pub struct ResolvedMessage<'a> {
    /// Byte offset within the original source, carried over from [`RawMessage`].
    pub offset: u64,
    /// Resolved value of tag `35` (MsgType), e.g. `"Logon"` or `"ExecutionReport"`.
    pub msg_type_name: Option<&'static str>,
    /// Dictionary chain actually used to resolve this message, in the order
    /// tried. Useful for diagnostics.
    pub chain: &'static [FixVersion],
    /// Fields in the order they appeared in the source message.
    pub fields: Vec<ResolvedField<'a>>,
}

/// Resolve a [`RawMessage`] against an automatically-selected dictionary chain.
///
/// The chain is picked from the message's `BeginString` and `ApplVerID`.
pub fn resolve<'a>(msg: &RawMessage<'a>) -> ResolvedMessage<'a> {
    let begin_string = find_tag(msg, TAG_BEGIN_STRING).unwrap_or(b"");
    let appl_ver_id = find_tag(msg, TAG_APPL_VER_ID);
    let chain = chain_for(begin_string, appl_ver_id);
    resolve_with_chain(msg, chain)
}

/// Resolve a [`RawMessage`] against an explicit dictionary chain.
///
/// Useful when the caller already knows the session's version (e.g. it saw
/// the Logon earlier) and wants to avoid re-inferring the chain per message.
///
/// Repeating-group awareness: when a field's tag is a known group counter
/// (see [`group_members`]), the resolver flags the contiguous run of
/// member tags that follow it with `depth = 1` so the TUI can render the
/// block indented. The run ends at the first tag that's not in the
/// counter's member set.
pub fn resolve_with_chain<'a>(msg: &RawMessage<'a>, chain: DictChain) -> ResolvedMessage<'a> {
    let mut fields: Vec<ResolvedField<'a>> = Vec::with_capacity(msg.tags.len());
    let mut msg_type_name: Option<&'static str> = None;
    let mut active_group: Option<&'static [u32]> = None;

    for &(tag, value) in &msg.tags {
        let name = chain_field_by_tag(chain, tag).map(|d| d.name);
        let value_label = if tag == TAG_MSG_TYPE {
            let label = chain_msg_type_label(chain, value);
            if msg_type_name.is_none() {
                msg_type_name = label;
            }
            label
        } else {
            chain_enum_value_label(chain, tag, value)
        };

        // Depth tracking. A group counter always emits at depth 0; its
        // members follow at depth 1 until a non-member tag closes the
        // run. Encountering a *new* counter opens a fresh group and
        // replaces the active set.
        let (depth, next_active) = match (active_group, group_members(tag)) {
            (_, Some(members)) => (0, Some(members)),
            (Some(members), None) if members.contains(&tag) => (1, active_group),
            (_, None) => (0, None),
        };
        active_group = next_active;

        fields.push(ResolvedField {
            tag,
            name,
            value,
            value_label,
            depth,
        });
    }

    ResolvedMessage {
        offset: msg.offset,
        msg_type_name,
        chain,
        fields,
    }
}

fn find_tag<'a>(msg: &RawMessage<'a>, tag: u32) -> Option<&'a [u8]> {
    msg.tags.iter().find(|(t, _)| *t == tag).map(|(_, v)| *v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixlog_parser::parse_one;

    fn build_with_trailer(body: &[u8], begin_string: &[u8]) -> Vec<u8> {
        let mut head = Vec::new();
        head.extend_from_slice(b"8=");
        head.extend_from_slice(begin_string);
        head.push(0x01);
        let bl_prefix = format!("9={}\x01", body.len());
        head.extend_from_slice(bl_prefix.as_bytes());
        head.extend_from_slice(body);
        let sum: u32 = head.iter().map(|&b| b as u32).sum();
        let cs = (sum % 256) as u8;
        head.extend_from_slice(format!("10={cs:03}\x01").as_bytes());
        head
    }

    fn build(bytes: &[u8]) -> RawMessage<'_> {
        parse_one(bytes).expect("fixture must parse").0
    }

    #[test]
    fn resolves_fix44_logon() {
        let body = b"35=A\x0149=S\x0156=T\x0134=1\x0152=20260101-00:00:00\x0198=0\x01108=30\x01";
        let raw = build_with_trailer(body, b"FIX.4.4");
        let msg = build(&raw);
        let res = resolve(&msg);

        assert_eq!(res.msg_type_name, Some("Logon"));
        assert_eq!(res.chain, crate::CHAIN_FIX44);
        let field_98 = res.fields.iter().find(|f| f.tag == 98).unwrap();
        assert_eq!(field_98.name, Some("EncryptMethod"));
        assert_eq!(field_98.value_label, Some("NONE_OTHER"));
    }

    #[test]
    fn resolves_fixt11_logon_with_appl_ver_id_9() {
        // FIXT.1.1 Logon with ApplVerID=9 (FIX 5.0SP2). Tag 1137 DefaultApplVerID
        // lives in FIX50SP2 and resolves because the chain falls back there.
        let body =
            b"35=A\x0149=S\x0156=T\x0134=1\x0152=20260101-00:00:00\x0198=0\x01108=30\x011128=9\x011137=9\x01";
        let raw = build_with_trailer(body, b"FIXT.1.1");
        let msg = build(&raw);
        let res = resolve(&msg);

        assert_eq!(res.msg_type_name, Some("Logon"));
        assert_eq!(res.chain, crate::CHAIN_FIXT11_FIX50SP2);
        assert_eq!(
            res.fields.iter().find(|f| f.tag == 1128).unwrap().name,
            Some("ApplVerID")
        );
        assert_eq!(
            res.fields.iter().find(|f| f.tag == 1137).unwrap().name,
            Some("DefaultApplVerID")
        );
    }

    #[test]
    fn unknown_tag_has_no_name() {
        let body = b"35=D\x0199999=hello\x01";
        let raw = build_with_trailer(body, b"FIX.4.4");
        let msg = build(&raw);
        let res = resolve(&msg);
        let custom = res.fields.iter().find(|f| f.tag == 99999).unwrap();
        assert_eq!(custom.name, None);
        assert_eq!(custom.value_label, None);
    }

    #[test]
    fn side_enum_label_resolves() {
        let body = b"35=D\x0154=1\x01";
        let raw = build_with_trailer(body, b"FIX.4.4");
        let msg = build(&raw);
        let res = resolve(&msg);
        let side = res.fields.iter().find(|f| f.tag == 54).unwrap();
        assert_eq!(side.name, Some("Side"));
        assert_eq!(side.value_label, Some("BUY"));
    }

    /// MarketDataSnapshot-shaped body with `268=3` and three blocks of
    /// `{269, 270, 271}`. Verifies `268` stays at depth 0, all three
    /// repetitions' members land at depth 1, and a post-group tag (`10`
    /// is added by `build_with_trailer`) drops back to depth 0.
    #[test]
    fn mdentries_group_members_get_depth_one() {
        let body = b"35=W\x0155=AAPL\x01268=3\x01\
269=0\x01270=100.0\x01271=10\x01\
269=1\x01270=100.5\x01271=20\x01\
269=2\x01270=101.0\x01271=30\x01";
        let raw = build_with_trailer(body, b"FIX.4.4");
        let msg = build(&raw);
        let res = resolve(&msg);

        // 55 (Symbol) comes before the counter — depth 0.
        assert_eq!(res.fields.iter().find(|f| f.tag == 55).unwrap().depth, 0);
        // Counter itself — depth 0.
        assert_eq!(res.fields.iter().find(|f| f.tag == 268).unwrap().depth, 0);
        // All three repetitions of 269/270/271 — depth 1.
        for tag in [269u32, 270, 271] {
            let members: Vec<_> = res.fields.iter().filter(|f| f.tag == tag).collect();
            assert_eq!(members.len(), 3, "expected 3 occurrences of tag {tag}");
            for f in members {
                assert_eq!(f.depth, 1, "tag {tag} expected depth 1, got {}", f.depth);
            }
        }
        // Trailer (10 CheckSum) closes the group — depth 0 again.
        assert_eq!(res.fields.iter().find(|f| f.tag == 10).unwrap().depth, 0);
    }
}
