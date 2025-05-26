pub fn init() -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

pub fn print(message: &str) {
    println!("{}", message);
}

pub fn clear_screen() {
    print!("\x1B[2J\x1B[1;1H");
}
