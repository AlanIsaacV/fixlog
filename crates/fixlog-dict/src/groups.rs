//! Repeating-group hints.
//!
//! FIX repeating groups are triggered by a `NumInGroup` counter tag (e.g.
//! `268 NoMDEntries`, `146 NoRelatedSym`, `73 NoOrders`) followed by N
//! blocks of member tags. The exact membership depends on the enclosing
//! message, so a perfectly general resolver would need full message-schema
//! awareness.
//!
//! This module exposes a **pragmatic hint**: for the handful of counter
//! tags that dominate real-world logs, it returns the union of member tags
//! that may appear inside the group across the common FIX dictionaries.
//! The resolver uses the hint to render a visual hierarchy (indented
//! fields under the counter) in the detail panel — it does not rely on
//! the hint being exhaustive, and it stops grouping at the first tag
//! outside the set so a stray non-member cleanly closes the group.
//!
//! Adding more counters is a one-line change; auto-deriving the full set
//! from `dictionaries/*.xml` at build time is tracked as P5-T10 follow-up
//! work.

/// Return the set of tags that may appear as members of the repeating
/// group opened by `counter_tag`. `None` for tags that are not group
/// counters (or not yet covered).
pub fn group_members(counter_tag: u32) -> Option<&'static [u32]> {
    match counter_tag {
        // 268 NoMDEntries — covers MarketDataSnapshotFullRefresh (`W`)
        // and MarketDataIncrementalRefresh (`X`). Members include the
        // MDEntry* block plus the Instrument sub-component that some
        // venues embed per entry on incremental refresh.
        268 => Some(&[
            55,   // Symbol
            48,   // SecurityID
            22,   // SecurityIDSource
            207,  // SecurityExchange
            461,  // CFICode
            269,  // MDEntryType
            270,  // MDEntryPx
            271,  // MDEntrySize
            272,  // MDEntryDate
            273,  // MDEntryTime
            274,  // TickDirection
            275,  // MDMkt
            276,  // QuoteCondition
            277,  // TradeCondition
            278,  // MDEntryID
            279,  // MDUpdateAction
            280,  // MDEntryRefID
            282,  // MDEntryOriginator
            283,  // LocationID
            284,  // DeskID
            285,  // DeleteReason
            286,  // OpenCloseSettlFlag
            287,  // SellerDays
            288,  // MDEntryBuyer
            289,  // MDEntrySeller
            290,  // MDEntryPositionNo
            291,  // FinancialStatus
            292,  // CorporateAction
            293,  // DefBidSize
            294,  // DefOfferSize
            336,  // TradingSessionID
            432,  // ExpireDate
            625,  // TradingSessionSubID
            828,  // TrdType
            1023, // MDPriceLevel
        ]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mdentries_counter_returns_member_set() {
        let members = group_members(268).expect("268 must be registered");
        // Core MDEntry* members.
        assert!(members.contains(&269));
        assert!(members.contains(&270));
        assert!(members.contains(&271));
        assert!(members.contains(&1023));
    }

    #[test]
    fn non_counter_tags_return_none() {
        assert!(group_members(35).is_none()); // MsgType
        assert!(group_members(52).is_none()); // SendingTime
        assert!(group_members(0).is_none());
    }
}
