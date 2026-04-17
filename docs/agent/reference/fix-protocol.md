# FIX wire format — primer

Just enough to work on the parser, sniffer, dict, and resolver without re-learning the spec.

## Message layout

```
8=<BeginString><SEP>9=<BodyLength><SEP><body tags><SEP>10=<CheckSum><SEP>
```

- `SEP` is typically SOH (`0x01`); re-rendered logs use `|`, `^`, `;`.
- `BeginString` is the protocol version marker: `FIX.4.4`, `FIXT.1.1`, etc.
- `BodyLength` is the byte count **between the `<SEP>` after `9=` and the `<SEP>` before `10=`**, exclusive of both. In other words, the length of the "body tags" substring (starting at tag 35) including every separator within it but not the ones framing the length header or the checksum.
- `CheckSum` is exactly 3 ASCII digits, computed as the sum of **all** preceding bytes (from `8=` through the `<SEP>` that closes the last body field), modulo 256, zero-padded.

## Canonical tags (session / framing)

| Tag | Name | Required | Notes |
|-----|------|----------|-------|
| 8 | BeginString | Y | Always first. |
| 9 | BodyLength | Y | Always second. |
| 35 | MsgType | Y | Always first field of body. Char or string: `A`, `0`, `D`, `8`, `CO`, etc. |
| 34 | MsgSeqNum | Y | Per-session monotonic. |
| 49 | SenderCompID | Y | Session identity (initiator side). |
| 56 | TargetCompID | Y | Session identity (acceptor side). |
| 52 | SendingTime | Y | `YYYYMMDD-HH:MM:SS[.sss]` UTC. Sorts lexicographically as chronological. |
| 10 | CheckSum | Y | Always last. 3 ASCII digits. |

`fixlog-parser` exports these as `TAG_*` constants (`TAG_BEGIN_STRING`, `TAG_BODY_LENGTH`, `TAG_CHECKSUM`, `TAG_MSG_SEQ_NUM`, `TAG_MSG_TYPE`, `TAG_SENDER_COMP_ID`, `TAG_SENDING_TIME`, `TAG_TARGET_COMP_ID`).

## Common MsgTypes (FIX 4.4)

| Wire | Name |
|------|------|
| `0` | Heartbeat |
| `1` | TestRequest |
| `2` | ResendRequest |
| `3` | Reject |
| `4` | SequenceReset |
| `5` | Logout |
| `A` | Logon |
| `D` | NewOrderSingle |
| `F` | OrderCancelRequest |
| `G` | OrderCancelReplaceRequest |
| `8` | ExecutionReport |
| `9` | OrderCancelReject |
| `V` | MarketDataRequest |
| `W` | MarketDataSnapshotFullRefresh |
| `X` | MarketDataIncrementalRefresh |
| `Y` | MarketDataRequestReject |
| `j` | BusinessMessageReject |
| `x` | SecurityListRequest |
| `y` | SecurityList |

Full resolution comes from `fixlog-dict::msg_type_label` / `chain_msg_type_label`. Non-standard or vendor-extended types (e.g. `CO`) resolve to `None`.

## FIXT.1.1 vs FIX.4.4 split

`FIX.4.4` bundles session + application. `FIXT.1.1` separates them: the **session** messages (Logon, Logout, Heartbeat, ResendRequest, SequenceReset, TestRequest, Reject, BusinessMessageReject, XMLnonFIX) live in the FIXT dictionary; the **application** messages (NewOrderSingle, ExecutionReport, MarketDataRequest, etc.) live in a FIX 5.0.x dictionary.

The active application dictionary is negotiated via `ApplVerID`:

| Tag 1128 (ApplVerID) value | Dictionary |
|----------------------------|------------|
| `0` | FIX 2.7 |
| `1` | FIX 3.0 |
| `2` | FIX 4.0 |
| `3` | FIX 4.1 |
| `4` | FIX 4.2 |
| `5` | FIX 4.3 |
| `6` | FIX 4.4 |
| `7` | FIX 5.0 |
| `8` | FIX 5.0SP1 |
| `9` | FIX 5.0SP2 |

`ApplVerID` typically appears on Logon (as `DefaultApplVerID`, tag 1137) and on individual app messages (tag 1128). In `fixlog-dict`, `chain_for` consults tag 1128 first; see `crates/dict.md` for the routing table.

## Checksum algorithm

```rust
fn compute_checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u32, |acc, &b| acc + b as u32) as u8
}
```

The sum is over **all bytes up to and including the `<SEP>` that closes the last body field**, i.e. `msg[..msg.len() - "10=NNN<SEP>".len()]`. The trailer itself is **not** included in the sum.

Tag 10 is 3 zero-padded ASCII digits. `validator::parse_checksum` rejects anything that isn't exactly 3 digits.

## Body-length semantics

`BodyLength` does **not** include:
- The `8=<bs><SEP>` preamble.
- The `9=<bl><SEP>` length header itself.
- The `10=<cs><SEP>` trailer.

It **does** include every separator between the first body tag (typically `35=…<SEP>`) and the last body `<SEP>` before `10=`.

Parser invariant: `body_end = body_start + body_length`. If `body_end` walks past the buffer → `ParseError::UnexpectedEof`.

## Common real-world quirks

- **Re-rendered logs**: SOH replaced with `|` for readability. Body-length and checksum reflect the *rendered* bytes, not the original wire. In practice the rendering is 1:1 byte-for-byte (SOH is also 1 byte), so BodyLength stays valid but the checksum may not if any other transformation happened.
- **Prefix wrappers**: timestamp, logback, or `<ts, session, dir>` strings before `8=FIX`. Parser scans for `8=FIX` and skips prefixes automatically.
- **Custom tags**: tag numbers >5000 are frequently vendor extensions. Resolver returns `None` for them — display as `?` in pretty output.
- **Non-ASCII in values**: rare but legal (e.g. UTF-8 free text in `Text` / tag 58). Render via `String::from_utf8_lossy`.

## Further references

- FIX 4.4 spec: https://www.onixs.biz/fix-dictionary/4.4/
- FIX 5.0SP2: https://www.onixs.biz/fix-dictionary/5.0.sp2/
- QuickFIX/J dictionaries (the ones we bundle): https://github.com/quickfix-j/quickfixj/tree/master/quickfixj-messages
