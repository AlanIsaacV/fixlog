//! `fixlog orders consolidate <inputs...>` — multi-source consolidated
//! order summary with optional `.gz` decompression.

use std::io::{BufWriter, Read, Write};

use anyhow::{Context, Result, anyhow};
use fixlog_analysis::orders_consolidated::{ConsolidatedBuilder, OrderConsolidated};
use fixlog_core::dict::chain_enum_value_label;
use fixlog_core::{CHAIN_FIX44, LogFormat};

use crate::io::{InputSource, open_source};
use crate::{ConsolidateSort, ConsolidatedFormat};

const SNIFF_WINDOW: usize = 64 * 1024;

pub fn run(
    inputs: &[InputSource],
    format: ConsolidatedFormat,
    sort: ConsolidateSort,
) -> Result<()> {
    if inputs.is_empty() {
        return Err(anyhow!("at least one input is required"));
    }

    let (log_format, primary) = sniff_and_prepare_first(&inputs[0])?;

    let mut builder = ConsolidatedBuilder::new();
    builder
        .push_source(primary, &log_format)
        .with_context(|| format!("reading {}", label(&inputs[0])))?;
    for src in &inputs[1..] {
        let reader = open_source(src).with_context(|| format!("opening {}", label(src)))?;
        builder
            .push_source(reader, &log_format)
            .with_context(|| format!("reading {}", label(src)))?;
    }

    let mut rows = builder.finish();
    sort_rows(&mut rows, sort);

    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    match format {
        ConsolidatedFormat::Pretty => write_pretty(&mut out, &rows)?,
        ConsolidatedFormat::Csv => write_csv(&mut out, &rows)?,
        ConsolidatedFormat::Json => write_json(&mut out, &rows)?,
    }
    out.flush()?;

    if rows.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn label(src: &InputSource) -> String {
    match src {
        InputSource::File(p) => p.display().to_string(),
        InputSource::Stdin => "<stdin>".to_string(),
    }
}

/// Read the first `SNIFF_WINDOW` bytes of the primary input, sniff the
/// log format, and return a `Read` that re-emits those bytes followed by
/// the remainder. Works uniformly for files, `.gz` archives, and stdin.
fn sniff_and_prepare_first(src: &InputSource) -> Result<(LogFormat, Box<dyn Read>)> {
    let mut reader = open_source(src).with_context(|| format!("opening {}", label(src)))?;
    let mut prefix = Vec::with_capacity(SNIFF_WINDOW);
    (&mut reader)
        .take(SNIFF_WINDOW as u64)
        .read_to_end(&mut prefix)
        .with_context(|| format!("sniffing {}", label(src)))?;
    if prefix.is_empty() {
        return Err(anyhow!("{} is empty", label(src)));
    }
    let log_format = fixlog_core::sniff(&prefix)
        .with_context(|| format!("could not infer log format from {}", label(src)))?;
    let chained = std::io::Cursor::new(prefix).chain(reader);
    Ok((log_format, Box::new(chained)))
}

fn sort_rows(rows: &mut [OrderConsolidated], sort: ConsolidateSort) {
    rows.sort_by(|a, b| {
        let primary = match sort {
            ConsolidateSort::Notional => b
                .notional
                .partial_cmp(&a.notional)
                .unwrap_or(std::cmp::Ordering::Equal),
            ConsolidateSort::CumQty => b.cum_qty.cmp(&a.cum_qty),
            ConsolidateSort::Fills => b.fills.cmp(&a.fills),
            ConsolidateSort::Recent => b.last_seen.cmp(&a.last_seen),
        };
        primary.then_with(|| a.root_clordid.cmp(&b.root_clordid))
    });
}

fn status_label(bytes: &[u8]) -> String {
    chain_enum_value_label(CHAIN_FIX44, 39, bytes)
        .map(|s| s.to_string())
        .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
}

fn side_label(b: u8) -> &'static str {
    match b {
        b'1' => "BUY",
        b'2' => "SELL",
        b'3' => "BUYM",
        b'4' => "SELLS",
        b'5' => "SELLSE",
        b'6' => "SELLSX",
        b'7' => "UNDISC",
        b'8' => "CROSS",
        b'9' => "CROSSS",
        _ => "?",
    }
}

fn family_display(row: &OrderConsolidated) -> String {
    if row.family.len() <= 1 {
        return lossy(&row.root_clordid);
    }
    let mut fam: Vec<String> = row.family.iter().map(|c| lossy(c)).collect();
    fam.sort();
    fam.join("→")
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn fmt_int(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(b as char);
    }
    out
}

fn fmt_money(n: f64) -> String {
    let int_part = n.trunc() as i64;
    let frac = (n.fract().abs() * 100.0).round() as u64;
    format!("{}.{:02}", fmt_int(int_part.unsigned_abs()), frac)
}

fn fmt_price(n: f64) -> String {
    if n == 0.0 {
        "-".to_string()
    } else {
        format!("{n:.4}")
    }
}

fn write_pretty<W: Write>(out: &mut W, rows: &[OrderConsolidated]) -> Result<()> {
    if rows.is_empty() {
        writeln!(out, "(no orders)")?;
        return Ok(());
    }
    writeln!(
        out,
        "{:<18} {:<5} {:<8} {:>10} {:>10} {:>15} {:>10} {:<14} {:>5}",
        "ClOrdID", "Side", "Symbol", "OrderQty", "CumQty", "Notional", "AvgPx", "Status", "Fills"
    )?;
    writeln!(
        out,
        "{}",
        "-".repeat(18 + 5 + 8 + 10 + 10 + 15 + 10 + 14 + 5 + 8)
    )?;
    for r in rows {
        let side = r.side.map(side_label).unwrap_or("-");
        let symbol = r.symbol.as_deref().map(lossy).unwrap_or_else(|| "-".into());
        let order_qty = r.order_qty.map(fmt_int).unwrap_or_else(|| "-".into());
        let status = r
            .final_ord_status
            .as_deref()
            .map(status_label)
            .unwrap_or_else(|| "-".into());
        writeln!(
            out,
            "{:<18} {:<5} {:<8} {:>10} {:>10} {:>15} {:>10} {:<14} {:>5}",
            truncate(&family_display(r), 18),
            side,
            truncate(&symbol, 8),
            order_qty,
            fmt_int(r.cum_qty),
            fmt_money(r.notional),
            fmt_price(r.avg_px),
            truncate(&status, 14),
            r.fills,
        )?;
    }
    Ok(())
}

fn write_csv<W: Write>(out: &mut W, rows: &[OrderConsolidated]) -> Result<()> {
    writeln!(
        out,
        "root_clordid,family,side,symbol,order_qty,cum_qty,notional,avg_px,fills,final_ord_status"
    )?;
    for r in rows {
        let side = r.side.map(side_label).unwrap_or("");
        let symbol = r.symbol.as_deref().map(lossy).unwrap_or_default();
        let order_qty = r.order_qty.map(|q| q.to_string()).unwrap_or_default();
        let status = r
            .final_ord_status
            .as_deref()
            .map(status_label)
            .unwrap_or_default();
        let family: Vec<String> = r.family.iter().map(|c| lossy(c)).collect();
        writeln!(
            out,
            "{},{},{},{},{},{},{:.4},{:.4},{},{}",
            csv_field(&lossy(&r.root_clordid)),
            csv_field(&family.join("|")),
            side,
            csv_field(&symbol),
            order_qty,
            r.cum_qty,
            r.notional,
            r.avg_px,
            r.fills,
            csv_field(&status),
        )?;
    }
    Ok(())
}

fn write_json<W: Write>(out: &mut W, rows: &[OrderConsolidated]) -> Result<()> {
    out.write_all(b"[")?;
    for (i, r) in rows.iter().enumerate() {
        if i > 0 {
            out.write_all(b",")?;
        }
        write!(out, r#"{{"root_clordid":""#)?;
        write_json_bytes(out, &r.root_clordid)?;
        out.write_all(br#"","family":["#)?;
        for (j, c) in r.family.iter().enumerate() {
            if j > 0 {
                out.write_all(b",")?;
            }
            out.write_all(b"\"")?;
            write_json_bytes(out, c)?;
            out.write_all(b"\"")?;
        }
        out.write_all(b"]")?;
        if let Some(sym) = r.symbol.as_deref() {
            out.write_all(br#","symbol":""#)?;
            write_json_bytes(out, sym)?;
            out.write_all(b"\"")?;
        }
        if let Some(side) = r.side {
            write!(out, r#","side":"{}""#, side_label(side))?;
        }
        if let Some(oq) = r.order_qty {
            write!(out, r#","order_qty":{oq}"#)?;
        }
        write!(out, r#","cum_qty":{}"#, r.cum_qty)?;
        write!(out, r#","notional":{:.4}"#, r.notional)?;
        write!(out, r#","avg_px":{:.4}"#, r.avg_px)?;
        write!(out, r#","fills":{}"#, r.fills)?;
        if let Some(status) = r.final_ord_status.as_deref() {
            write!(out, r#","final_ord_status":"{}""#, status_label(status))?;
        }
        out.write_all(b"}")?;
    }
    out.write_all(b"]\n")?;
    Ok(())
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}

fn write_json_bytes<W: Write>(out: &mut W, bytes: &[u8]) -> std::io::Result<()> {
    let s = String::from_utf8_lossy(bytes);
    for c in s.chars() {
        match c {
            '"' => out.write_all(br#"\""#)?,
            '\\' => out.write_all(br"\\")?,
            '\n' => out.write_all(br"\n")?,
            '\r' => out.write_all(br"\r")?,
            '\t' => out.write_all(br"\t")?,
            c if (c as u32) < 0x20 => write!(out, "\\u{:04x}", c as u32)?,
            c => {
                let mut buf = [0u8; 4];
                out.write_all(c.encode_utf8(&mut buf).as_bytes())?;
            }
        }
    }
    Ok(())
}
