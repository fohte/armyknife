use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "Fohte's armyknife", bin_name = "a", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Update to the latest version
    Update,
}
