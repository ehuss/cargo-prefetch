use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

fn main() {
    let path = match std::env::args().skip(1).next() {
        Some(arg) => PathBuf::from(arg),
        None => {
            eprintln!("Must specify path to crates.");
            std::process::exit(1)
        }
    };
    if let Err(e) = doit(&path) {
        eprintln!("error: {}", e);
        for cause in e.chain() {
            eprintln!("Caused by: {}", cause);
        }
        std::process::exit(1);
    }
}

fn doit(index: &PathBuf) -> Result<()> {
    let mut counts = HashMap::new();

    reg_index::list_all(index, None, None, |entries| {
        if let Some(pkg) = entries.into_iter().max_by(|a, b| a.vers.cmp(&b.vers)) {
            for dep in pkg.deps {
                *counts.entry(dep.name).or_insert(0) += 1;
            }
        }
    })?;

    let mut all: Vec<(u32, String)> = counts
        .into_iter()
        .map(|(name, count)| (count, name))
        .collect();
    all.sort_unstable();
    all.reverse();
    let n = 1000.min(all.len());
    println!("pub static TOP_CRATES: [&'static str; {}] = [", n);
    for (count, name) in all.into_iter().take(n) {
        println!("    \"{}\", // {}", name, count);
    }
    println!("];");
    Ok(())
}
