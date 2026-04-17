//! Upgrade a [`RawMessage`] into a [`ResolvedMessage`] by decorating each
//! tag with its dictionary name and, when applicable, its enum-value label.

use fixlog_parser::{RawMessage, TAG_BEGIN_STRING, TAG_MSG_TYPE};

use crate::{
    DictChain, FixVersion, chain_enum_value_label, chain_field_by_tag, chain_for,
    chain_msg_type_label,
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
pub fn resolve_with_chain<'a>(msg: &RawMessage<'a>, chain: DictChain) -> ResolvedMessage<'a> {
    let mut fields: Vec<ResolvedField<'a>> = Vec::with_capacity(msg.tags.len());
    let mut msg_type_name: Option<&'static str> = None;

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
        fields.push(ResolvedField {
            tag,
            name,
            value,
            value_label,
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
}
