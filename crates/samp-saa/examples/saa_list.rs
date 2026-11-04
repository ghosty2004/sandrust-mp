//! List (and optionally extract) the entries of a samp.saa archive.
//!
//! ```text
//! cargo run -p samp-saa --example saa_list -- <samp.saa> [--extract <dir under target/>]
//! ```
//!
//! Archive contents are third-party data: extraction is only allowed into a
//! directory below a `target/` folder (local inspection, never committed).

use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use samp_saa::SaaArchive;

fn under_target(dir: &Path) -> bool {
    let abs = if dir.is_absolute() { dir.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(dir) };
    abs.components().any(|c| matches!(c, Component::Normal(n) if n == "target"))
        && !abs.components().any(|c| matches!(c, Component::ParentDir))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (archive, extract) = match args.as_slice() {
        [a] => (PathBuf::from(a), None),
        [a, flag, out] if flag == "--extract" => (PathBuf::from(a), Some(PathBuf::from(out))),
        _ => {
            eprintln!("usage: saa_list <samp.saa> [--extract <dir under target/>]");
            return ExitCode::from(2);
        }
    };
    if let Some(out) = &extract
        && !under_target(out)
    {
        eprintln!("refusing to extract outside a target/ directory: {}", out.display());
        return ExitCode::from(2);
    }

    let saa = match SaaArchive::open(&archive) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{}: {e}", archive.display());
            return ExitCode::FAILURE;
        }
    };
    println!("{}: signature {}", archive.display(), if saa.signature_valid() { "valid" } else { "INVALID" });
    println!("slot  hash        size      offset    name");
    for e in saa.list() {
        let off = e.offset.map_or("broken".to_string(), |o| format!("{o:#09x}"));
        println!("{:>4}  {:#010x}  {:>8}  {off:>9}  {}", e.index, e.hash, e.size, e.name.unwrap_or("?"));
        if let Some(out) = &extract {
            let Some(data) = saa.read_entry(&e) else {
                eprintln!("  cannot read slot {}", e.index);
                continue;
            };
            let name = e.name.map_or_else(|| format!("{:08x}.bin", e.hash), str::to_string);
            if let Err(err) = std::fs::create_dir_all(out).and_then(|_| std::fs::write(out.join(&name), &data)) {
                eprintln!("  write {name}: {err}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Some(out) = &extract {
        println!("extracted to {}", out.display());
    }
    ExitCode::SUCCESS
}
