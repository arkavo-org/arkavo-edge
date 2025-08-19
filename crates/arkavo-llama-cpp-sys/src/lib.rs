#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

// Only include real bindings for non-musl targets
#[cfg(not(target_env = "musl"))]
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

// For musl, create empty module (llama-cpp feature shouldn't be used with musl anyway)
#[cfg(target_env = "musl")]
pub mod dummy {
    // Empty module for musl targets
}
