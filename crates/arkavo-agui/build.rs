fn main() {
    println!("cargo:rerun-if-changed=static/index.html");
    println!("cargo:rerun-if-changed=static/toolbar.js");
    println!("cargo:rerun-if-changed=static/styles/base.css");
    println!("cargo:rerun-if-changed=static/styles/panels.css");
    println!("cargo:rerun-if-changed=static/styles/dashboard.css");
    println!("cargo:rerun-if-changed=static/js/state.js");
    println!("cargo:rerun-if-changed=static/js/websocket.js");
    println!("cargo:rerun-if-changed=static/js/chat.js");
    println!("cargo:rerun-if-changed=static/js/app.js");
    println!("cargo:rerun-if-changed=static/js/panels/agents.js");
    println!("cargo:rerun-if-changed=static/js/panels/tasks.js");
    println!("cargo:rerun-if-changed=static/js/panels/telemetry.js");
    println!("cargo:rerun-if-changed=static/js/panels/budget.js");
    println!("cargo:rerun-if-changed=static/js/panels/health.js");
    println!("cargo:rerun-if-changed=static/js/panels/router.js");
}
