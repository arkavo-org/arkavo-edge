pub mod commands;

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.is_empty() {
        print_usage();
        return Err("No command provided".into());
    }

    match args[0].as_str() {
        "chat" => commands::chat::execute(&args[1..]),
        "plan" => commands::plan::execute(&args[1..]),
        "apply" => commands::apply::execute(&args[1..]),
        "test" => commands::test::execute(&args[1..]),
        "vault" => commands::vault::execute(&args[1..]),
        "help" => {
            print_usage();
            Ok(())
        }
        "-h" | "--help" => {
            print_usage();
            Ok(())
        }
        "-v" | "--version" => {
            print_version();
            Ok(())
        }
        _ => {
            eprintln!("Error: Unknown command '{}'", args[0]);
            print_usage();
            Err(format!("Unknown command: {}", args[0]).into())
        }
    }
}

fn print_usage() {
    println!("Arkavo Edge - Developer-centric agentic CLI tool for AI-agent development");
    println!();
    println!("USAGE:");
    println!("    arkavo <COMMAND> [OPTIONS]");
    println!();
    println!("COMMANDS:");
    println!("    chat      Start conversational agent with repository context");
    println!("    plan      Generate a change plan before code edits");
    println!("    apply     Execute plan and commit changes");
    println!("    test      Run tests with streaming failure feedback");
    println!("    vault     Import/export notes to Edge Vault");
    println!("    help      Print this help message");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help       Print help information");
    println!("    -v, --version    Print version information");
}

fn print_version() {
    println!("arkavo {}", env!("CARGO_PKG_VERSION"));
}
