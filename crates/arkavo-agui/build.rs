fn main() {
    println!("cargo:rerun-if-changed=static/toolbar.js");
    println!("cargo:rerun-if-changed=static/shell.html");
    println!("cargo:rerun-if-changed=static/dashboard.html");
    println!("cargo:rerun-if-changed=static/index-agui.html");
}
