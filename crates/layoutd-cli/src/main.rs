use std::path::{Path, PathBuf};

use clap::{Parser,Subcommand};

#[derive(Parser)]
struct Cli{
    #[command(subcommand)]

    command : Command
}

#[derive(Subcommand)]
enum Command{
    Diff{
        old : PathBuf,
        new :PathBuf,
        #[arg(long)]
        account : String
    },
    Gen{
        old : PathBuf,
        new :PathBuf,
        #[arg(long)]
        account : String
    },
    Check{
        old : PathBuf,
        new :PathBuf,
        #[arg(long)]
        account : String
    }
}

fn main() {
    let cli = Cli::parse();

   
}
