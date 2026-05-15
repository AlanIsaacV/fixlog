//! Streaming aggregator for consolidated order summaries.
//!
//! Given any number of `impl Read` sources (file, mmap slice, gz decoder,
//! stdin) it accumulates a single per-order summary across them:
//!
//! - root `ClOrdID` and the full family after cancel/replace chains (tags
//!   `11` linked through `41` = `OrigClOrdID`),
//! - cumulative quantity (max of tag `14`),
//! - notional matched (`Σ LastQty × LastPx`, tags `32` × `31`) over
//!   ExecutionReports of type Trade/PartialFill (`150 ∈ {F, 1}`), with
//!   `ExecID` (tag `17`) deduplication so a resend of the same fill is
//!   only counted once,
//! - implicit `AvgPx` (`notional / cum_qty`) — never read from tag `6`,
//! - last seen `OrdStatus` (tag `39`),
//! - `Symbol` (tag `55`) and `Side` (tag `54`) captured from the first
//!   message that exposes them.
//!
//! Streaming is implemented by buffering chunks (1 MiB) and parsing
//! complete messages via `parse_one_with_format`; the trailing partial
//! message survives across `read()` calls.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::time::SystemTime;

use fixlog_core::format::LogFormat;
use fixlog_core::index::secondary::{TAG_CL_ORD_ID, TAG_ORDER_ID, TAG_ORIG_CL_ORD_ID};
use fixlog_core::parser::{ParseError, TAG_MSG_TYPE, TAG_SENDING_TIME};
use fixlog_core::{RawMessage, parse_one_with_format};
use smallvec::SmallVec;

use crate::AnalysisError;
use crate::orders::{TAG_CUM_QTY, TAG_EXEC_TYPE, TAG_LAST_PX, TAG_ORD_STATUS};
use crate::util::{find_tag, parse_sending_time};

/// Tag `17` — ExecID. Unique per ExecutionReport; used as the dedup key
/// for fills.
pub const TAG_EXEC_ID: u32 = 17;
/// Tag `31` — LastPx. (Re-exported from `orders` for module-local clarity.)
pub const TAG_LAST_PX_FILL: u32 = TAG_LAST_PX;
/// Tag `32` — LastQty. Quantity of the last fill in this ExecutionReport.
pub const TAG_LAST_QTY: u32 = 32;
/// Tag `38` — OrderQty. Original order quantity (set on the NewOrderSingle
/// and updated by Cancel/Replace requests).
pub const TAG_ORDER_QTY: u32 = 38;
/// Tag `54` — Side (1=Buy, 2=Sell, …).
pub const TAG_SIDE: u32 = 54;
/// Tag `55` — Symbol.
pub const TAG_SYMBOL: u32 = 55;

const BEGIN_STRING_MARKER: &[u8] = b"8=FIX";
const READ_CHUNK: usize = 1 << 20; // 1 MiB
const MAX_BUFFER: usize = 8 << 20; // 8 MiB watchdog
const FILL_EXEC_TYPES: [&[u8]; 2] = [b"F", b"1"];

/// One row of the consolidated summary.
#[derive(Clone, Debug)]
pub struct OrderConsolidated {
    pub root_clordid: Vec<u8>,
    pub family: SmallVec<[Vec<u8>; 2]>,
    pub symbol: Option<Vec<u8>>,
    pub side: Option<u8>,
    pub order_qty: Option<u64>,
    pub cum_qty: u64,
    pub notional: f64,
    pub avg_px: f64,
    pub fills: u32,
    pub final_ord_status: Option<SmallVec<[u8; 2]>>,
    pub first_seen: Option<SystemTime>,
    pub last_seen: Option<SystemTime>,
}

/// Per-source statistics returned by [`ConsolidatedBuilder::push_source`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SourceStats {
    pub messages: u64,
    pub fills_seen: u64,
    pub fills_deduped: u64,
}

/// Builder that consumes messages from one or more streams and produces
/// consolidated rows. See module docs for the aggregation rules.
pub struct ConsolidatedBuilder {
    /// Acc keyed by canonical (root) ClOrdID. Non-root family members
    /// resolve to a root through `alias`.
    accs: HashMap<Vec<u8>, OrderAcc>,
    /// Alias map: `child_clordid → parent_clordid`. Walked to fixed point
    /// in [`Self::resolve_root`]. Path compression keeps subsequent
    /// lookups O(1) amortized.
    alias: HashMap<Vec<u8>, Vec<u8>>,
    /// OrderID → root ClOrdID, so an ExecutionReport that only carries
    /// `37` (after a re-cycle of `11`) still attaches to the right family.
    order_id_to_root: HashMap<Vec<u8>, Vec<u8>>,
}

#[derive(Default)]
struct OrderAcc {
    family: SmallVec<[Vec<u8>; 2]>,
    symbol: Option<Vec<u8>>,
    side: Option<u8>,
    order_qty: Option<u64>,
    cum_qty: u64,
    notional: f64,
    fills: u32,
    seen_execids: HashSet<Vec<u8>>,
    final_ord_status: Option<SmallVec<[u8; 2]>>,
    first_seen: Option<SystemTime>,
    last_seen: Option<SystemTime>,
}

impl Default for ConsolidatedBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsolidatedBuilder {
    pub fn new() -> Self {
        Self {
            accs: HashMap::new(),
            alias: HashMap::new(),
            order_id_to_root: HashMap::new(),
        }
    }

    /// Stream messages from `src` into the builder.
    ///
    /// Reads in 1 MiB chunks, parses complete messages and keeps the
    /// trailing partial message for the next iteration. A `.gz` decoder
    /// or any other `impl Read` is acceptable as long as it eventually
    /// terminates.
    pub fn push_source<R: Read>(
        &mut self,
        mut src: R,
        format: &LogFormat,
    ) -> Result<SourceStats, AnalysisError> {
        let mut buf: Vec<u8> = Vec::with_capacity(READ_CHUNK * 2);
        let mut tmp = vec![0u8; READ_CHUNK];
        let mut stats = SourceStats::default();
        loop {
            let n = src.read(&mut tmp)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            let consumed = self.drain_complete(&buf, format, &mut stats);
            if consumed > 0 {
                buf.drain(..consumed);
            }
            if buf.len() > MAX_BUFFER {
                // No 8=FIX boundary found inside the last 8 MiB. Treat
                // everything up to the last MAX_BUFFER/2 bytes as junk so
                // the buffer cannot grow without bound on pathological
                // inputs (corrupted gz, binary noise).
                let drop = buf.len() - (MAX_BUFFER / 2);
                tracing::warn!(
                    dropped = drop,
                    "discarding bytes with no recoverable FIX boundary"
                );
                buf.drain(..drop);
            }
        }
        // EOF: drain whatever remains. We accept that the trailing
        // message may be truncated — `parse_one_with_format` will reject
        // it with `UnexpectedEof` and `drain_complete` will simply leave
        // it behind.
        let _ = self.drain_complete(&buf, format, &mut stats);
        Ok(stats)
    }

    /// Materialize the rows. Output is sorted by descending notional then
    /// by `root_clordid` for tie-break determinism.
    pub fn finish(self) -> Vec<OrderConsolidated> {
        let mut rows: Vec<OrderConsolidated> = self
            .accs
            .into_iter()
            .map(|(root, acc)| {
                let avg_px = if acc.cum_qty > 0 {
                    acc.notional / acc.cum_qty as f64
                } else {
                    0.0
                };
                OrderConsolidated {
                    root_clordid: root,
                    family: acc.family,
                    symbol: acc.symbol,
                    side: acc.side,
                    order_qty: acc.order_qty,
                    cum_qty: acc.cum_qty,
                    notional: acc.notional,
                    avg_px,
                    fills: acc.fills,
                    final_ord_status: acc.final_ord_status,
                    first_seen: acc.first_seen,
                    last_seen: acc.last_seen,
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            b.notional
                .partial_cmp(&a.notional)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.root_clordid.cmp(&b.root_clordid))
        });
        rows
    }

    fn drain_complete(&mut self, buf: &[u8], format: &LogFormat, stats: &mut SourceStats) -> usize {
        let mut cursor = 0usize;
        while cursor < buf.len() {
            let Some(rel) = memchr::memmem::find(&buf[cursor..], BEGIN_STRING_MARKER) else {
                // No further BeginString in the buffer: nothing more to
                // do; keep the trailing bytes (could contain a partial
                // marker) by reporting `cursor` as last consumed.
                break;
            };
            let start = cursor + rel;
            match parse_one_with_format(&buf[start..], format) {
                Ok((msg, consumed)) => {
                    self.handle_message(&msg, stats);
                    cursor = start + consumed;
                }
                Err(ParseError::UnexpectedEof) => {
                    // Partial message at the tail: stop and keep
                    // everything from `start` for the next read().
                    return start;
                }
                Err(err) => {
                    tracing::warn!(error = %err, "skipping malformed message");
                    // Advance past the marker so we can find the next one.
                    cursor = start + 1;
                }
            }
        }
        cursor
    }

    fn handle_message(&mut self, msg: &RawMessage<'_>, stats: &mut SourceStats) {
        stats.messages += 1;
        let msg_type = find_tag(msg, TAG_MSG_TYPE).unwrap_or(b"");
        let clordid = find_tag(msg, TAG_CL_ORD_ID);
        let orig_clordid = find_tag(msg, TAG_ORIG_CL_ORD_ID);
        let order_id = find_tag(msg, TAG_ORDER_ID);

        // 1. Bind the new ClOrdID (if present) to a family root.
        if let Some(cid) = clordid {
            let parent_known = orig_clordid
                .map(|orig| {
                    let root = self.resolve_root(orig);
                    self.accs.contains_key(&root)
                })
                .unwrap_or(false);
            if parent_known {
                // Cancel/Replace or ER referencing a predecessor we've
                // actually seen. Union: the new ClOrdID inherits the
                // root of `orig`.
                let orig = orig_clordid.expect("parent_known implies orig");
                self.union(cid, orig);
            } else {
                // Either no OrigClOrdID, or the parent was never observed
                // as a `tag 11` in this stream (e.g. log starts mid-flow,
                // or the value is a placeholder like `NONE`/`0`). Treat
                // `cid` as its own root rather than unifying everyone
                // who shares the unknown placeholder.
                let root = self.resolve_root(cid);
                self.ensure_acc(&root);
            }
        }

        // 2. Bind OrderID to the family root, so later ER's that only
        // carry tag 37 still resolve.
        if let Some(oid) = order_id
            && let Some(cid) = clordid
        {
            let root = self.resolve_root(cid);
            self.order_id_to_root
                .entry(oid.to_vec())
                .or_insert_with(|| root.clone());
        }

        // 3. Determine which root this message belongs to.
        let root = match clordid {
            Some(cid) => Some(self.resolve_root(cid)),
            None => order_id.and_then(|oid| self.order_id_to_root.get(oid).cloned()),
        };
        let Some(root) = root else { return };

        let acc = self.accs.entry(root.clone()).or_default();
        if !acc.family.iter().any(|c| c.as_slice() == root.as_slice()) {
            acc.family.push(root.clone());
        }
        if let Some(cid) = clordid
            && cid != root.as_slice()
            && !acc.family.iter().any(|c| c.as_slice() == cid)
        {
            acc.family.push(cid.to_vec());
        }

        // 4. Capture descriptive fields the first time we see them.
        if acc.symbol.is_none()
            && let Some(sym) = find_tag(msg, TAG_SYMBOL)
            && !sym.is_empty()
        {
            acc.symbol = Some(sym.to_vec());
        }
        if acc.side.is_none()
            && let Some(side) = find_tag(msg, TAG_SIDE)
            && let Some(first) = side.first().copied()
        {
            acc.side = Some(first);
        }
        // OrderQty: prefer the most recent value seen on a D/F/G request
        // (Replace can change OrderQty). ExecutionReports also carry it
        // but we trust the request side as authoritative.
        if matches!(msg_type, b"D" | b"F" | b"G")
            && let Some(qty) = find_tag(msg, TAG_ORDER_QTY)
            && let Some(parsed) = parse_qty(qty)
        {
            acc.order_qty = Some(parsed);
        }

        // 5. Timestamps.
        let sending_time = find_tag(msg, TAG_SENDING_TIME).and_then(parse_sending_time);
        if let Some(t) = sending_time {
            acc.first_seen = Some(acc.first_seen.map_or(t, |cur| cur.min(t)));
            acc.last_seen = Some(acc.last_seen.map_or(t, |cur| cur.max(t)));
        }

        // 6. ExecutionReport-specific updates.
        if msg_type == b"8" {
            if let Some(status) = find_tag(msg, TAG_ORD_STATUS) {
                acc.final_ord_status = Some(SmallVec::from_slice(status));
            }
            if let Some(qty) = find_tag(msg, TAG_CUM_QTY)
                && let Some(parsed) = parse_qty(qty)
                && parsed > acc.cum_qty
            {
                acc.cum_qty = parsed;
            }
            let exec_type = find_tag(msg, TAG_EXEC_TYPE).unwrap_or(b"");
            if FILL_EXEC_TYPES.contains(&exec_type) {
                stats.fills_seen += 1;
                let exec_id = find_tag(msg, TAG_EXEC_ID);
                let dedup_ok = match exec_id {
                    Some(eid) if !eid.is_empty() => acc.seen_execids.insert(eid.to_vec()),
                    _ => {
                        tracing::warn!(
                            clordid = %String::from_utf8_lossy(&root),
                            "fill without ExecID — dedup failed open"
                        );
                        true
                    }
                };
                if !dedup_ok {
                    stats.fills_deduped += 1;
                    return;
                }
                if let (Some(last_qty), Some(last_px)) =
                    (find_tag(msg, TAG_LAST_QTY), find_tag(msg, TAG_LAST_PX_FILL))
                    && let (Some(q), Some(p)) = (parse_f64(last_qty), parse_f64(last_px))
                {
                    acc.notional += q * p;
                    acc.fills = acc.fills.saturating_add(1);
                }
            }
        }
    }

    fn ensure_acc(&mut self, root: &[u8]) {
        self.accs.entry(root.to_vec()).or_default();
    }

    /// Walk `alias` to fixed point, applying path compression. Returns
    /// the canonical root for `cid`.
    fn resolve_root(&mut self, cid: &[u8]) -> Vec<u8> {
        let mut cur: Vec<u8> = cid.to_vec();
        let mut path: Vec<Vec<u8>> = Vec::new();
        while let Some(parent) = self.alias.get(&cur) {
            path.push(cur.clone());
            cur = parent.clone();
        }
        // Path compression.
        for node in path {
            self.alias.insert(node, cur.clone());
        }
        cur
    }

    /// Merge `child`'s family into `parent`'s family. After this call,
    /// `resolve_root(child) == resolve_root(parent)`.
    fn union(&mut self, child: &[u8], parent: &[u8]) {
        let root_child = self.resolve_root(child);
        let root_parent = self.resolve_root(parent);
        if root_child == root_parent {
            // Already in the same family — but make sure `child` itself
            // has the alias entry so later lookups don't walk via parent.
            if child != root_child.as_slice() {
                self.alias.insert(child.to_vec(), root_child.clone());
            }
            return;
        }
        // Merge `root_child` into `root_parent`. We keep the older root
        // (lexicographic min on the family of `root_parent` is a proxy
        // for "first seen"; in practice both are first-seen-anchors and
        // either choice is fine, we use `root_parent` because the caller
        // semantics are "child points at parent").
        let child_acc = self.accs.remove(&root_child).unwrap_or_default();
        let parent_acc = self.accs.entry(root_parent.clone()).or_default();
        parent_acc.merge_from(child_acc);
        // Rewire any OrderID bindings that pointed at the absorbed root.
        for v in self.order_id_to_root.values_mut() {
            if *v == root_child {
                *v = root_parent.clone();
            }
        }
        self.alias.insert(root_child, root_parent.clone());
        if child != root_parent.as_slice() {
            self.alias.insert(child.to_vec(), root_parent);
        }
    }
}

impl OrderAcc {
    fn merge_from(&mut self, other: OrderAcc) {
        for cid in other.family {
            if !self.family.iter().any(|c| c.as_slice() == cid.as_slice()) {
                self.family.push(cid);
            }
        }
        if self.symbol.is_none() {
            self.symbol = other.symbol;
        }
        if self.side.is_none() {
            self.side = other.side;
        }
        if self.order_qty.is_none() {
            self.order_qty = other.order_qty;
        }
        if other.cum_qty > self.cum_qty {
            self.cum_qty = other.cum_qty;
        }
        self.notional += other.notional;
        self.fills = self.fills.saturating_add(other.fills);
        for eid in other.seen_execids {
            self.seen_execids.insert(eid);
        }
        if other.final_ord_status.is_some() {
            self.final_ord_status = other.final_ord_status;
        }
        self.first_seen = match (self.first_seen, other.first_seen) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (None, x) | (x, None) => x,
        };
        self.last_seen = match (self.last_seen, other.last_seen) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (None, x) | (x, None) => x,
        };
    }
}

/// Parse an ASCII quantity into `u64`. Accepts a trailing `.<digits>`
/// fractional part and truncates it (with a debug-level note); FIX
/// quantities are typically integral but exchange feeds for crypto can
/// be decimal.
fn parse_qty(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    let mut split = bytes.splitn(2, |b| *b == b'.');
    let int_part = split.next()?;
    let mut out: u64 = 0;
    for &b in int_part {
        if !b.is_ascii_digit() {
            return None;
        }
        out = out.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    if let Some(frac) = split.next()
        && !frac.is_empty()
    {
        tracing::debug!(
            value = %String::from_utf8_lossy(bytes),
            "quantity truncated to integer"
        );
    }
    Some(out)
}

/// Parse an ASCII decimal into `f64`. FIX prices/qtys are always plain
/// decimal (no exponent, no commas), so this is a minimal hand-rolled
/// parser to avoid pulling `std::str::from_utf8` + `parse::<f64>` cost.
fn parse_f64(bytes: &[u8]) -> Option<f64> {
    if bytes.is_empty() {
        return None;
    }
    let s = std::str::from_utf8(bytes).ok()?;
    s.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixlog_core::sniff;

    fn build_msg(body_fields: &str) -> Vec<u8> {
        let body_len = body_fields.len();
        let head = format!("8=FIX.4.4\x019={body_len}\x01");
        let payload: Vec<u8> = head.bytes().chain(body_fields.bytes()).collect();
        let sum: u8 = payload.iter().fold(0u8, |a, b| a.wrapping_add(*b));
        let trailer = format!("10={sum:03}\x01");
        payload.into_iter().chain(trailer.bytes()).collect()
    }

    fn synthetic_log() -> Vec<u8> {
        let tail = "49=A\x0156=B\x01";
        let t = |sec| format!("52=20260417-12:34:{sec:02}\x01");
        let mut out = Vec::new();
        // Order 1 (ABC): D, ER PendingNew, ER New, ER PartialFill, ER Fill — Filled
        out.extend(build_msg(&format!(
            "35=D\x0134=1\x01{tail}{}11=ABC\x0155=AAPL\x0154=1\x0138=100\x01",
            t(1)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=2\x01{tail}{}11=ABC\x0137=ord1\x0117=E0\x01150=A\x0139=A\x0114=0\x01",
            t(2)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=3\x01{tail}{}11=ABC\x0137=ord1\x0117=E1\x01150=F\x0139=1\x0114=60\x0131=10.5\x0132=60\x01",
            t(3)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=4\x01{tail}{}11=ABC\x0137=ord1\x0117=E2\x01150=F\x0139=2\x0114=100\x0131=11.0\x0132=40\x01",
            t(4)
        )));
        // Order 2 (DEF → GHI): D, G(41=DEF), ER Replaced(11=GHI), ER Fill(11=GHI)
        out.extend(build_msg(&format!(
            "35=D\x0134=5\x01{tail}{}11=DEF\x0155=MSFT\x0154=2\x0138=50\x01",
            t(5)
        )));
        out.extend(build_msg(&format!(
            "35=G\x0134=6\x01{tail}{}11=GHI\x0141=DEF\x01",
            t(6)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=7\x01{tail}{}11=GHI\x0137=ord2\x0117=E3\x01150=5\x0139=5\x0114=0\x01",
            t(7)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=8\x01{tail}{}11=GHI\x0137=ord2\x0117=E4\x01150=F\x0139=2\x0114=50\x0131=100\x0132=50\x01",
            t(8)
        )));
        out
    }

    #[test]
    fn dedup_repeated_execid_counts_once() {
        let mut log = synthetic_log();
        // Append a duplicate of the ABC E1 fill (resend simulation).
        let tail = "49=A\x0156=B\x01";
        let t = |sec: i32| format!("52=20260417-12:34:{sec:02}\x01");
        log.extend(build_msg(&format!(
            "35=8\x0134=99\x01{tail}{}11=ABC\x0137=ord1\x0117=E1\x01150=F\x0139=1\x0114=60\x0131=10.5\x0132=60\x01",
            t(3)
        )));
        let fmt = sniff(&log).unwrap();
        let mut b = ConsolidatedBuilder::new();
        let stats = b.push_source(log.as_slice(), &fmt).unwrap();
        assert_eq!(stats.fills_deduped, 1, "one duplicate fill must be dropped");
        let rows = b.finish();
        let abc = rows
            .iter()
            .find(|r| r.root_clordid == b"ABC")
            .expect("ABC row");
        assert_eq!(abc.fills, 2, "ABC has 2 unique fills (E1, E2)");
        // notional = 60*10.5 + 40*11.0 = 630 + 440 = 1070
        assert!((abc.notional - 1070.0).abs() < 1e-9);
    }

    #[test]
    fn replace_chain_unifies_family() {
        let log = synthetic_log();
        let fmt = sniff(&log).unwrap();
        let mut b = ConsolidatedBuilder::new();
        b.push_source(log.as_slice(), &fmt).unwrap();
        let rows = b.finish();
        let row = rows
            .iter()
            .find(|r| r.family.iter().any(|c| c == b"DEF"))
            .expect("DEF family row");
        let mut fam: Vec<&[u8]> = row.family.iter().map(|v| v.as_slice()).collect();
        fam.sort();
        assert_eq!(fam, vec![b"DEF".as_slice(), b"GHI".as_slice()]);
        assert_eq!(row.root_clordid, b"DEF", "first-seen ClOrdID is the root");
        assert_eq!(row.cum_qty, 50);
        assert!((row.notional - 5000.0).abs() < 1e-9); // 50 * 100
        assert_eq!(row.fills, 1);
    }

    #[test]
    fn notional_is_sum_lastqty_lastpx() {
        // Craft a log where AvgPx in tag 6 disagrees with the implicit
        // notional/cum_qty — we must trust the fills.
        let tail = "49=A\x0156=B\x01";
        let t = |sec| format!("52=20260417-12:34:{sec:02}\x01");
        let mut log = Vec::new();
        log.extend(build_msg(&format!(
            "35=D\x0134=1\x01{tail}{}11=ZZZ\x0155=AAPL\x01",
            t(1)
        )));
        log.extend(build_msg(&format!(
            "35=8\x0134=2\x01{tail}{}11=ZZZ\x0117=X1\x01150=F\x0139=1\x016=999.99\x0114=10\x0131=10\x0132=10\x01",
            t(2)
        )));
        let fmt = sniff(&log).unwrap();
        let mut b = ConsolidatedBuilder::new();
        b.push_source(log.as_slice(), &fmt).unwrap();
        let rows = b.finish();
        let row = &rows[0];
        assert!((row.notional - 100.0).abs() < 1e-9);
        assert!((row.avg_px - 10.0).abs() < 1e-9);
    }

    #[test]
    fn fill_without_execid_still_counts() {
        let tail = "49=A\x0156=B\x01";
        let t = |sec| format!("52=20260417-12:34:{sec:02}\x01");
        let mut log = Vec::new();
        log.extend(build_msg(&format!(
            "35=D\x0134=1\x01{tail}{}11=NOID\x0155=AAPL\x01",
            t(1)
        )));
        // Fill without tag 17 — must be counted (warning) and dedup falls
        // open: the next identical message would also be counted.
        log.extend(build_msg(&format!(
            "35=8\x0134=2\x01{tail}{}11=NOID\x01150=F\x0139=1\x0114=5\x0131=20\x0132=5\x01",
            t(2)
        )));
        let fmt = sniff(&log).unwrap();
        let mut b = ConsolidatedBuilder::new();
        let stats = b.push_source(log.as_slice(), &fmt).unwrap();
        assert_eq!(stats.fills_seen, 1);
        assert_eq!(stats.fills_deduped, 0);
        let rows = b.finish();
        assert_eq!(rows[0].fills, 1);
        assert!((rows[0].notional - 100.0).abs() < 1e-9);
    }

    #[test]
    fn multi_source_concatenated_streams() {
        let log = synthetic_log();
        let mid = log.len() / 2;
        // Find a 8=FIX boundary near `mid` so the split lands between
        // messages (the builder also handles a split mid-message, but the
        // assertion is cleaner this way).
        let split = (mid..log.len())
            .find(|i| log[*i..].starts_with(b"8=FIX"))
            .expect("must find boundary");
        let (a, b) = log.split_at(split);

        let fmt = sniff(&log).unwrap();
        let mut whole = ConsolidatedBuilder::new();
        whole.push_source(log.as_slice(), &fmt).unwrap();
        let mut split_b = ConsolidatedBuilder::new();
        split_b.push_source(a, &fmt).unwrap();
        split_b.push_source(b, &fmt).unwrap();

        let lhs = whole.finish();
        let rhs = split_b.finish();
        assert_eq!(lhs.len(), rhs.len());
        for (l, r) in lhs.iter().zip(rhs.iter()) {
            assert_eq!(l.root_clordid, r.root_clordid);
            assert_eq!(l.cum_qty, r.cum_qty);
            assert!((l.notional - r.notional).abs() < 1e-9);
            assert_eq!(l.fills, r.fills);
            assert_eq!(l.final_ord_status, r.final_ord_status);
        }
    }
}
