use arkavo_test::mcp::xctest_compiler::XCTestCompiler;

fn main() {
    println!("Testing XCTest compilation...");
    
    match XCTestCompiler::new() {
        Ok(compiler) => {
            println!("Compiler created successfully");
            match compiler.get_xctest_bundle() {
                Ok(bundle) => println!("Bundle path: {}", bundle.display()),
                Err(e) => println!("Failed to compile bundle: {}", e),
            }
        }
        Err(e) => println!("Failed to create compiler: {}", e),
    }
}