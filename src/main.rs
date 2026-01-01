use clap::Parser;

#[derive(Parser)]
#[command(name = "Fohte's armyknife", bin_name = "a", version, about)]
struct Cli {}

fn main() {
    Cli::parse();
}
