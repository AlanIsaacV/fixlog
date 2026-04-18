//! Visual theme. For Phase 3 this is a hardcoded default; persistent theme
//! config via `~/.config/fixlog/config.toml` is scoped to Phase 5.

use ratatui::style::Color;

/// Colour to apply to a row based on its `MsgType` (tag 35). `None` means
/// "no explicit colour" — the list renders with the terminal default.
///
/// Defaults follow the plan in `docs/PHASE3_PLAN.md` P3-T11:
/// - `D` NewOrderSingle → green
/// - `8` ExecutionReport → blue
/// - `3` / `j` Reject / BusinessMessageReject → red
/// - `0` Heartbeat → gray
/// - anything else → `None`
pub fn color_for_msg_type(msg_type: &[u8]) -> Option<Color> {
    match msg_type {
        b"D" => Some(Color::Green),
        b"8" => Some(Color::Blue),
        b"3" | b"j" => Some(Color::Red),
        b"0" => Some(Color::DarkGray),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_map_covers_phase3_plan() {
        assert_eq!(color_for_msg_type(b"D"), Some(Color::Green));
        assert_eq!(color_for_msg_type(b"8"), Some(Color::Blue));
        assert_eq!(color_for_msg_type(b"3"), Some(Color::Red));
        assert_eq!(color_for_msg_type(b"j"), Some(Color::Red));
        assert_eq!(color_for_msg_type(b"0"), Some(Color::DarkGray));
    }

    #[test]
    fn unknown_msg_type_has_no_color() {
        assert_eq!(color_for_msg_type(b"A"), None);
        assert_eq!(color_for_msg_type(b"ZZ"), None);
        assert_eq!(color_for_msg_type(b""), None);
    }
}
