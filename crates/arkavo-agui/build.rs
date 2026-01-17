fn main() {
    println!("cargo:rerun-if-changed=static/toolbar.js");
    println!("cargo:rerun-if-changed=static/index.html");
}
