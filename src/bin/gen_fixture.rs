//! Generate synthetic CSV fixtures for Anvil, including a ZIP that
//! contains the CSV alongside realistic archive noise.
//!
//! Usage:
//!   gen_fixture <output-dir> [rows]

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;

const HEADERS: &[&str] = &[
    "id",
    "timestamp",
    "user_id",
    "email",
    "country",
    "currency",
    "amount",
    "status",
    "channel",
    "device",
    "signup_date",
    "last_login",
    "risk_score",
    "is_verified",
    "note",
    "reference",
    "group",
    "score",
    "delta",
    "payload",
];

const COUNTRIES: &[&str] = &["US", "DE", "GB", "FR", "JP", "BR", "IN", "CA", "AU", "NL"];
const CURRENCIES: &[&str] = &["USD", "EUR", "GBP", "JPY", "BRL", "INR", "CAD"];
const STATUSES: &[&str] = &["pending", "settled", "failed", "refunded", "review"];
const CHANNELS: &[&str] = &["web", "ios", "android", "api", "pos"];
const DEVICES: &[&str] = &["desktop", "phone", "tablet", "kiosk"];

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("fixtures"));
    let rows: u64 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);

    if let Err(err) = run(&out_dir, rows) {
        eprintln!("gen_fixture failed: {err}");
        std::process::exit(1);
    }
}

fn run(out_dir: &Path, rows: u64) -> std::io::Result<()> {
    std::fs::create_dir_all(out_dir)?;
    let csv_path = out_dir.join("large.csv");

    let started = std::time::Instant::now();
    write_csv(&csv_path, rows)?;
    let csv_len = std::fs::metadata(&csv_path)?.len();
    eprintln!(
        "wrote {} ({} rows, {:.1} MiB) in {:.2}s",
        csv_path.display(),
        rows,
        csv_len as f64 / (1024.0 * 1024.0),
        started.elapsed().as_secs_f64()
    );

    let zip_path = out_dir.join("archive.zip");
    write_zip(&zip_path, &csv_path)?;
    let zip_len = std::fs::metadata(&zip_path)?.len();
    eprintln!(
        "wrote {} ({:.1} MiB compressed)",
        zip_path.display(),
        zip_len as f64 / (1024.0 * 1024.0)
    );

    // A small multi-file archive so the picker has two real choices.
    let multi_path = out_dir.join("multi.zip");
    write_multi_zip(&multi_path)?;
    eprintln!("wrote {}", multi_path.display());

    // A deliberately malformed file for exercising the diagnostics UI.
    let malformed_path = out_dir.join("malformed.csv");
    std::fs::write(
        &malformed_path,
        b"id,name,value\n1,alice,10\n2,\"unterminated,20\n3,bob,30\n4,bob,extra,40\n",
    )?;
    eprintln!("wrote {}", malformed_path.display());

    Ok(())
}

fn write_csv(path: &Path, rows: u64) -> std::io::Result<()> {
    let file = File::create(path)?;
    let mut w = BufWriter::with_capacity(1 << 20, file);
    writeln!(w, "{}", HEADERS.join(","))?;

    let mut state: u64 = 0x9E3779B97F4A7C15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    for i in 0..rows {
        let r1 = next();
        let r2 = next();
        let r3 = next();

        let user_id = r1 % 250_000;
        let email = if i % 97 == 0 {
            String::new()
        } else if i % 211 == 0 {
            format!("broken-{i}@")
        } else {
            format!("user{user_id}@example{}", r2 % 5)
        };
        let country = COUNTRIES[(r1 % COUNTRIES.len() as u64) as usize];
        let currency = CURRENCIES[(r2 % CURRENCIES.len() as u64) as usize];
        let amount = (r3 % 1_000_000) as f64 / 100.0;
        let status = STATUSES[(r1 % STATUSES.len() as u64) as usize];
        let channel = CHANNELS[(r2 % CHANNELS.len() as u64) as usize];
        let device = DEVICES[(r3 % DEVICES.len() as u64) as usize];
        let risk = (r1 % 1000) as f64 / 10.0;
        let verified = if r2 % 3 == 0 { "false" } else { "true" };
        // Inner quotes are doubled so the emitted CSV is well-formed.
        let note = if i % 500 == 0 {
            "contains, comma and \"\"quotes\"\""
        } else {
            ""
        };
        let reference = format!("{:08x}-{:04x}", r1 & 0xffff_ffff, r2 & 0xffff);
        let group = format!("G{:02}", r3 % 40);
        let score = (r1 % 100) as i64;
        let delta = (r2 as i64 % 200) - 100;
        let payload = format!("{{\"\"k\"\":{},\"\"v\"\":\"\"{}\"\"}}", r3 % 7, country);

        writeln!(
            w,
            "{i},2026-01-{:02}T{:02}:{:02}:{:02}Z,{user_id},{email},{country},{currency},{amount:.2},{status},{channel},{device},2024-{:02}-{:02},2026-{:02}-{:02},{risk:.1},{verified},\"{note}\",{reference},{group},{score},{delta},\"{payload}\"",
            1 + (r1 % 28),
            r1 % 24,
            r2 % 60,
            r3 % 60,
            1 + (r2 % 12),
            1 + (r3 % 28),
            1 + (r1 % 12),
            1 + (r2 % 28),
        )?;
    }
    w.flush()?;
    Ok(())
}

fn write_zip(zip_path: &Path, csv_path: &Path) -> std::io::Result<()> {
    let file = File::create(zip_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // The real data file.
    zip.start_file("data/large.csv", options)?;
    let mut input = File::open(csv_path)?;
    std::io::copy(&mut input, &mut zip)?;

    // Archive noise the importer should ignore.
    zip.add_directory("__MACOSX/", options)?;
    zip.start_file("__MACOSX/._large.csv", options)?;
    zip.write_all(b"\x00\x05\x16\x07mac metadata placeholder")?;
    zip.start_file(".DS_Store", options)?;
    zip.write_all(b"\x00\x00\x00\x01Bud1")?;
    zip.start_file("README.txt", options)?;
    zip.write_all(b"This archive was generated by gen_fixture for Anvil testing.\n")?;

    zip.finish()?;
    Ok(())
}

fn write_multi_zip(zip_path: &Path) -> std::io::Result<()> {
    let file = File::create(zip_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // A smaller second dataset so the picker has two real choices.
    zip.start_file("exports/orders.csv", options)?;
    zip.write_all(b"order_id,customer,amount\n1,alice,10.00\n2,bob,20.50\n")?;

    zip.start_file("exports/customers.csv", options)?;
    zip.write_all(b"customer_id,name,country\n1,alice,US\n2,bob,DE\n")?;

    zip.add_directory("__MACOSX/", options)?;
    zip.start_file("__MACOSX/._orders.csv", options)?;
    zip.write_all(b"\x00\x05\x16\x07")?;

    zip.finish()?;
    Ok(())
}
