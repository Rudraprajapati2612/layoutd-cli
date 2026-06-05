use clap::{Parser,Subcommand};

#[derive(Parser)]
struct Cli{
    #[command(subcommand)]

    command : Command
}

#[derive(Subcommand)]
enum Command{
    Diff,
    Gen,
    Check
}
fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Diff => println!("not implemented"),
        Command::Gen => println!("not implemented"),
        Command::Check => println!("not implemented"),
    }
}
