use std::path::PathBuf;

use anyhow::Result;
use armyknife::config;

fn main() -> Result<()> {
    let output: Option<PathBuf> = std::env::args().nth(1).map(PathBuf::from);
    let schema = config::generate_schema();
    let json = serde_json::to_string_pretty(&schema)?;
    match output {
        Some(path) => {
            std::fs::write(&path, format!("{json}\n"))?;
            eprintln!("Schema written to {}", path.display());
        }
        None => println!("{json}"),
    }
    Ok(())
}
