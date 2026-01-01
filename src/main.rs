use clap::Parser;

#[derive(Parser)]
#[command(name = "a", version, about)]
struct Cli {}

fn main() {
    Cli::parse();
}
