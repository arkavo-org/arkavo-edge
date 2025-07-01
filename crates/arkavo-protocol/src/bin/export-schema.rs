use arkavo_protocol::{generate_openrpc_schema, openrpc_to_json};

fn main() {
    match openrpc_to_json() {
        Ok(json) => {
            println!("{}", json);
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Failed to generate OpenRPC schema: {}", e);
            std::process::exit(1);
        }
    }
}
