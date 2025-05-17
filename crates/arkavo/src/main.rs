use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    let command_args = args.get(1..).unwrap_or_default().to_vec();

    if let Err(err) = arkavo_cli::run(&command_args) {
        eprintln!("Error: {}", err);
        process::exit(1);
    }
}