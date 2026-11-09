use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let dist_dir = manifest_dir.join("agent-watch-web/dist");
    let assets_dir = if dist_dir.is_dir() {
        println!("cargo:rerun-if-changed={}", dist_dir.display());
        dist_dir
    } else {
        println!(
            "cargo:rerun-if-changed={}",
            manifest_dir.join("agent-watch-web").display()
        );
        let empty_dir = PathBuf::from(env::var("OUT_DIR")?).join("agent-watch-web-dist");
        fs::create_dir_all(&empty_dir)?;
        empty_dir
    };

    println!(
        "cargo:rustc-env=AGENT_WATCH_WEB_ASSETS={}",
        assets_dir.display()
    );
    Ok(())
}
