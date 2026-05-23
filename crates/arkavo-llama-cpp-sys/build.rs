use std::env;
use std::path::PathBuf;
use std::process::Command;

struct PatchMetadata {
    base_commit: Option<String>,
    verify_absent: Option<String>,
    verify_present: Option<String>,
}

fn parse_patch_metadata(patch_path: &std::path::Path) -> PatchMetadata {
    let content = std::fs::read_to_string(patch_path).unwrap_or_default();
    let mut meta = PatchMetadata {
        base_commit: None,
        verify_absent: None,
        verify_present: None,
    };

    for line in content.lines() {
        if let Some(val) = line.strip_prefix("# BASE_COMMIT: ") {
            meta.base_commit = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("# VERIFY_ABSENT: ") {
            meta.verify_absent = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("# VERIFY_PRESENT: ") {
            meta.verify_present = Some(val.trim().to_string());
        }
    }
    meta
}

fn get_vendor_commit(vendor_dir: &std::path::Path) -> Option<String> {
    Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(vendor_dir)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
}

fn file_contains(vendor_dir: &std::path::Path, pattern: &str) -> bool {
    Command::new("grep")
        .args(["-rq", pattern])
        .current_dir(vendor_dir)
        .status()
        .is_ok_and(|s| s.success())
}

fn apply_patches(vendor_dir: &std::path::Path, patches_dir: &std::path::Path) {
    if !patches_dir.exists() {
        return;
    }

    let current_commit = get_vendor_commit(vendor_dir);

    let mut patches: Vec<_> = std::fs::read_dir(patches_dir)
        .expect("Failed to read patches directory")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "patch"))
        .collect();

    patches.sort_by_key(|e| e.path());

    for entry in patches {
        let patch_path = entry.path();
        let patch_name = patch_path.file_name().unwrap().to_string_lossy();
        println!("cargo:rerun-if-changed={}", patch_path.display());

        let meta = parse_patch_metadata(&patch_path);

        // Check if patch might be merged upstream (VERIFY_ABSENT text is gone)
        if let Some(ref absent_text) = meta.verify_absent {
            if !file_contains(vendor_dir, absent_text) {
                println!(
                    "cargo:warning=Patch {} may be merged upstream - VERIFY_ABSENT text not found. Consider removing patch.",
                    patch_name
                );
                continue;
            }
        }

        // Fail on commit drift to ensure patches are verified against correct version
        if let (Some(ref base), Some(ref current)) = (&meta.base_commit, &current_commit) {
            if !current.starts_with(base) && !base.starts_with(current) {
                panic!(
                    "Patch {} was created for commit {}, but vendor is at {}. \
                     Update BASE_COMMIT in patch metadata or verify patch still applies.",
                    patch_name, base, current
                );
            }
        }

        // Check if patch is already applied by trying a reverse dry-run
        // -f prevents interactive prompts that default to "yes" in non-tty contexts
        let reverse_check = Command::new("patch")
            .args(["--dry-run", "-R", "-f", "-p1", "-i"])
            .arg(&patch_path)
            .current_dir(vendor_dir)
            .output()
            .expect("Failed to run patch command");

        if reverse_check.status.success() {
            // Reverse applies cleanly = patch is already applied
            eprintln!("Patch already applied: {}", patch_name);
            continue;
        }

        // Patch not yet applied, apply it
        eprintln!("Applying patch: {}", patch_name);
        let result = Command::new("patch")
            .args(["-p1", "-N", "--no-backup-if-mismatch", "-i"])
            .arg(&patch_path)
            .current_dir(vendor_dir)
            .output()
            .expect("Failed to apply patch");

        if !result.status.success() {
            eprintln!("Patch output: {}", String::from_utf8_lossy(&result.stdout));
            eprintln!("Patch stderr: {}", String::from_utf8_lossy(&result.stderr));
            panic!(
                "Failed to apply patch: {}. The upstream code may have changed. \
                 Check if the fix was merged or if the patch needs updating.",
                patch_name
            );
        }

        // Verify patch was applied correctly
        if let Some(ref present_text) = meta.verify_present {
            if !file_contains(vendor_dir, present_text) {
                panic!(
                    "Patch {} applied but VERIFY_PRESENT text not found. \
                     The patch may have applied incorrectly.",
                    patch_name
                );
            }
        }
    }
}

fn main() {
    // Skip building for musl targets - llama.cpp doesn't work well with musl
    let target = env::var("TARGET").unwrap_or_default();
    if target.contains("musl") {
        println!("cargo:warning=Skipping llama.cpp build for musl target");
        // Create dummy bindings for musl
        let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
        std::fs::write(out_path.join("bindings.rs"), "// Dummy bindings for musl\n").unwrap();
        return;
    }
    // Track only key files that indicate real source changes
    // Avoid tracking directories as CMake build artifacts trigger false rebuilds
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=ARKAVO_TARGET_DEVICE");
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/CMakeLists.txt");
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/include/llama.h");
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/tools/mtmd/mtmd.h");
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/tools/mtmd/clip.h");
    // Track git HEAD to detect submodule updates
    let git_path = std::path::Path::new("../../vendor/llama.cpp/.git");
    if git_path.exists() {
        if git_path.is_dir() {
            println!("cargo:rerun-if-changed=../../vendor/llama.cpp/.git/HEAD");
        } else {
            // It's a git-link file
            println!("cargo:rerun-if-changed=../../vendor/llama.cpp/.git");
            // Also try to track the actual HEAD in the main repository's modules directory
            let submodule_head = std::path::Path::new("../../.git/modules/vendor/llama.cpp/HEAD");
            if submodule_head.exists() {
                println!("cargo:rerun-if-changed=../../.git/modules/vendor/llama.cpp/HEAD");
            }
        }
    }

    // Apply patches to vendor/llama.cpp before building
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendor_dir = manifest_dir.join("../../vendor/llama.cpp");
    let patches_dir = manifest_dir.join("patches");
    apply_patches(&vendor_dir, &patches_dir);

    let mut config = cmake::Config::new("../../vendor/llama.cpp");

    // Determine build type based on profile
    // Debug builds use RelWithDebInfo for reasonable speed with debug symbols
    // Release builds use Release for maximum optimization
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let build_type = if profile == "release" {
        "Release"
    } else {
        "RelWithDebInfo" // Faster than MinSizeRel, includes debug info
    };

    config
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("CMAKE_BUILD_TYPE", build_type)
        // Disable Unity builds - incompatible with Objective-C (Metal backend)
        .define("CMAKE_UNITY_BUILD", "OFF");

    // Enable parallel compilation (skip -j for Windows/MSBuild)
    if !target.contains("windows") {
        if let Ok(num_jobs) = env::var("NUM_JOBS") {
            config.build_arg(format!("-j{}", num_jobs));
        } else {
            // Use number of CPUs
            let num_cpus = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            config.build_arg(format!("-j{}", num_cpus));
        }
    }
    // MSBuild handles parallelism automatically with /m flag (set by cmake internally)

    // Platform-specific GPU acceleration
    if cfg!(target_os = "macos") {
        config
            .define("GGML_METAL", "ON") // Enable Metal for GPU on macOS
            .define("GGML_METAL_EMBED_LIBRARY", "ON") // Embed Metal shaders for GPU acceleration
            .define("GGML_ACCELERATE", "ON") // use Apple Accelerate
            .define("GGML_NATIVE", "OFF") // Disable native CPU feature detection to ensure compatibility
            .define("GGML_CPU_ARM_ARCH", "armv8.2-a+fp16") // Use baseline ARM arch without i8mm
            .define("GGML_METAL_NDEBUG", "ON"); // Always disable Metal debug for faster builds
    } else if cfg!(target_os = "windows") {
        // Windows can use Vulkan or CUDA if available
        config
            .define("GGML_VULKAN", "OFF") // Could be enabled if Vulkan SDK present
            .define("GGML_CUDA", "OFF"); // Could be enabled if CUDA toolkit present
    } else {
        // Linux - use CPU optimizations by default with static linking
        config
            .define("GGML_BLAS", "OFF") // Could be enabled if OpenBLAS present
            .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON") // Required for static linking
            .define("GGML_STATIC", "ON") // Build static libraries
            .define("GGML_OPENMP", "ON"); // Enable OpenMP but link statically

        // ARM64-specific optimizations for Raspberry Pi and similar systems
        if target.contains("aarch64") || target.contains("arm64") {
            config.define("GGML_NATIVE", "OFF"); // Disable native CPU feature detection for portability

            // Device-specific CPU architecture targets
            if let Ok(device) = env::var("ARKAVO_TARGET_DEVICE") {
                if device == "uno-q" {
                    // UNO Q has Cortex-A53 (ARMv8.0) - use conservative architecture
                    config.define("GGML_CPU_ARM_ARCH", "armv8-a+fp+simd");
                    eprintln!("Building CPU-only for UNO Q (ARMv8.0/Cortex-A53, Vulkan disabled)");
                    eprintln!("See: docs/uno-q-hardware-acceleration-blockers.md");
                } else {
                    // Other devices: use ARMv8.2 baseline
                    config.define("GGML_CPU_ARM_ARCH", "armv8.2-a+fp16");
                    eprintln!("Building CPU-only for device: {}", device);
                }
            } else {
                // No device specified - use ARMv8.2 baseline for portability
                config.define("GGML_CPU_ARM_ARCH", "armv8.2-a+fp16");
                eprintln!(
                    "Building CPU-only for aarch64-linux (no ARKAVO_TARGET_DEVICE specified)"
                );
            }
        }
    }

    // Common settings for all platforms
    config
        .define("GGML_OPENCL", "OFF") // Mesa OpenCL doesn't support Adreno GPUs
        .define("GGML_ASSERTS", "OFF") // Disable asserts for performance
        .define("LLAMA_CURL", "OFF") // Disable CURL requirement (not needed for local inference)
        .define("LLAMA_HTTPLIB", "OFF") // Disable httplib (not needed without server)
        .define("LLAMA_OPENSSL", "OFF") // Disable OpenSSL (project uses rustls exclusively)
        .define("LLAMA_BUILD_MTMD", "ON") // Enable multimodal support
        .define("LLAMA_BUILD_TESTS", "OFF") // Don't build tests
        .define("LLAMA_BUILD_EXAMPLES", "OFF") // Don't build examples
        .define("LLAMA_BUILD_SERVER", "OFF") // Don't build server
        .define("LLAMA_BUILD_APP", "OFF") // Unified `llama` binary links server-impl; not needed for sys crate
        .define("LLAMA_BUILD_COMMON", "ON"); // Build common library (chat templates, Jinja, grammar)

    // Use ccache or sccache if available for faster rebuilds
    if let Ok(ccache) = which::which("ccache") {
        config
            .define("CMAKE_C_COMPILER_LAUNCHER", ccache.to_str().unwrap())
            .define("CMAKE_CXX_COMPILER_LAUNCHER", ccache.to_str().unwrap());
        eprintln!("Using ccache for faster C++ compilation");
    } else if let Ok(sccache) = which::which("sccache") {
        config
            .define("CMAKE_C_COMPILER_LAUNCHER", sccache.to_str().unwrap())
            .define("CMAKE_CXX_COMPILER_LAUNCHER", sccache.to_str().unwrap());
        eprintln!("Using sccache for faster C++ compilation");
    }

    let dst = config.build();

    let lib_dir = dst.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    // Dynamically discover and link all static libraries built by CMake
    // This handles both Unix (.a) and Windows (.lib) naming conventions
    if cfg!(target_os = "windows") {
        // Windows: look for *.lib files
        for entry in std::fs::read_dir(&lib_dir)
            .unwrap_or_else(|_| panic!("Failed to read lib directory: {}", lib_dir.display()))
        {
            let entry = entry.unwrap();
            let path = entry.path();
            if let Some(fname) = path.file_name().and_then(|f| f.to_str()) {
                if fname.ends_with(".lib") && !fname.ends_with(".dll.lib") {
                    let libname = fname.trim_end_matches(".lib");
                    println!("cargo:rustc-link-lib=static={}", libname);
                }
            }
        }
    } else {
        // Unix: look for lib*.a files
        for entry in std::fs::read_dir(&lib_dir)
            .unwrap_or_else(|_| panic!("Failed to read lib directory: {}", lib_dir.display()))
        {
            let entry = entry.unwrap();
            let path = entry.path();
            if let Some(fname) = path.file_name().and_then(|f| f.to_str()) {
                if fname.starts_with("lib") && fname.ends_with(".a") {
                    let libname = fname.trim_start_matches("lib").trim_end_matches(".a");
                    println!("cargo:rustc-link-lib=static={}", libname);
                }
            }
        }
    }

    // Link libraries from the CMake build tree that aren't installed to lib/.
    // Upstream b9292 renamed `common` → `llama-common` and split the build-info
    // OBJECT library into a static `llama-common-base` archive. `llama-common`
    // is installed (auto-picked from lib/); `llama-common-base` and the vendored
    // cpp-httplib are not, so we link them out of the CMake build tree.
    //
    // On Windows with MSBuild, CMake places artifacts in config subdirectories
    // (e.g., build/common/Release/) instead of build/common/ directly.
    let build_dir = dst.join("build");
    let common_base = build_dir.join("common");
    let common_lib = if cfg!(target_os = "windows") {
        let release_dir = common_base.join("Release");
        let relwithdebinfo_dir = common_base.join("RelWithDebInfo");
        if release_dir.exists() {
            release_dir
        } else if relwithdebinfo_dir.exists() {
            relwithdebinfo_dir
        } else {
            common_base.clone()
        }
    } else {
        common_base.clone()
    };
    if common_lib.exists() {
        println!("cargo:rustc-link-search=native={}", common_lib.display());
        println!("cargo:rustc-link-lib=static=llama-common-base");
    }
    let httplib_base = build_dir.join("vendor/cpp-httplib");
    let httplib = if cfg!(target_os = "windows") {
        let release_dir = httplib_base.join("Release");
        if release_dir.exists() {
            release_dir
        } else {
            httplib_base.clone()
        }
    } else {
        httplib_base
    };
    if httplib.exists() {
        println!("cargo:rustc-link-search=native={}", httplib.display());
        println!("cargo:rustc-link-lib=static=cpp-httplib");
    }

    // Platform-specific linking
    if cfg!(target_os = "macos") {
        // Metal backend for GPU acceleration on macOS
        println!("cargo:rustc-link-lib=static=ggml-metal");
        println!("cargo:rustc-link-lib=framework=Metal");
        println!("cargo:rustc-link-lib=framework=MetalKit");
        println!("cargo:rustc-link-lib=framework=MetalPerformanceShaders");
        println!("cargo:rustc-link-lib=framework=Accelerate");
        println!("cargo:rustc-link-lib=framework=Foundation");
    } else if cfg!(target_os = "windows") {
        // Windows MSVC uses automatic C++ runtime linking via /MD or /MT flag
        // CMake handles this automatically based on CMAKE_MSVC_RUNTIME_LIBRARY
        // No explicit cargo:rustc-link-lib needed for C++ runtime
    } else {
        // Linux specific libraries
        println!("cargo:rustc-link-lib=stdc++");
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=m"); // math library

        // Static linking of OpenMP for zero-config deployment
        let target = env::var("TARGET").unwrap_or_default();
        if target.contains("aarch64") || target.contains("arm64") {
            // ARM64 library paths for cross-compilation and native builds
            // Try multiple GCC versions (Ubuntu 22.04 typically has GCC 11-12)
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/aarch64-linux-gnu/13");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/aarch64-linux-gnu/12");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/aarch64-linux-gnu/11");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/aarch64-linux-gnu/10");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/aarch64-linux-gnu/9");
            println!("cargo:rustc-link-search=native=/usr/lib/aarch64-linux-gnu");
            println!("cargo:rustc-link-search=native=/usr/aarch64-linux-gnu/lib");
            // Also check cross-compilation sysroot
            println!("cargo:rustc-link-search=native=/usr/lib/gcc-cross/aarch64-linux-gnu/13");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc-cross/aarch64-linux-gnu/12");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc-cross/aarch64-linux-gnu/11");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc-cross/aarch64-linux-gnu/10");

            // Vulkan disabled for UNO Q - using CPU-only build
            // See docs/uno-q-hardware-acceleration-blockers.md
        } else {
            // x86_64 library paths
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/13");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/12");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/11");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/10");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/9");
            println!("cargo:rustc-link-search=native=/usr/lib/x86_64-linux-gnu");
        }
        println!("cargo:rustc-link-lib=static=gomp"); // Static OpenMP
    }

    // C++ standard library (handling varies by platform)
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c++");
    }

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Compile the C++ chat wrapper that bridges to common_chat_templates_apply()
    let wrapper_src = manifest_dir.join("arkavo_chat_wrapper.cpp");
    let spec_wrapper_src = manifest_dir.join("arkavo_spec_wrapper.cpp");
    if wrapper_src.exists() {
        println!("cargo:rerun-if-changed={}", wrapper_src.display());
        println!(
            "cargo:rerun-if-changed={}",
            manifest_dir.join("arkavo_chat_wrapper.h").display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            manifest_dir.join("arkavo_grammar_wrapper.h").display()
        );
        println!("cargo:rerun-if-changed={}", spec_wrapper_src.display());
        println!(
            "cargo:rerun-if-changed={}",
            manifest_dir.join("arkavo_spec_wrapper.h").display()
        );

        let mut cc_build = cc::Build::new();
        cc_build
            .cpp(true)
            .std("c++17")
            .file(&wrapper_src)
            .file(&spec_wrapper_src);
        // -fexceptions is GCC/Clang only; MSVC enables exceptions by default
        if !cfg!(target_os = "windows") {
            cc_build.flag("-fexceptions");
            // Suppress warnings from vendored llama.cpp jinja headers
            cc_build.flag("-Wno-unused-function");
        }
        cc_build
            .include(out_path.join("include"))
            .include(vendor_dir.join("common"))
            .include(vendor_dir.join("include"))
            .include(vendor_dir.join("vendor"))
            .include(vendor_dir.join("ggml/include"))
            .include(&manifest_dir)
            .compile("arkavo_chat_wrapper");
        eprintln!("Compiled arkavo_chat_wrapper.cpp");
    }

    let bindings_path = out_path.join("bindings.rs");
    let header = out_path.join("include").join("llama.h");
    let mtmd_header = out_path.join("include").join("mtmd.h");

    // Check if main header exists
    if !header.exists() {
        panic!(
            "llama.h not found at {:?}. CMake build may have failed.",
            header
        );
    }

    // Only regenerate bindings if they don't exist or header changed
    let should_regenerate = !bindings_path.exists() || {
        let header_mtime = std::fs::metadata(&header).and_then(|m| m.modified()).ok();
        let bindings_mtime = std::fs::metadata(&bindings_path)
            .and_then(|m| m.modified())
            .ok();

        match (header_mtime, bindings_mtime) {
            (Some(h), Some(b)) => h > b,
            _ => true,
        }
    };

    if should_regenerate {
        eprintln!("Generating Rust bindings for llama.cpp");

        // Create a wrapper header that includes llama.h, mtmd.h, and chat wrapper
        let wrapper_header = out_path.join("wrapper.h");
        let mut wrapper_content = format!("#include \"{}\"\n", header.display());
        if mtmd_header.exists() {
            wrapper_content.push_str(&format!("#include \"{}\"\n", mtmd_header.display()));
        }
        let chat_wrapper_header = manifest_dir.join("arkavo_chat_wrapper.h");
        if chat_wrapper_header.exists() {
            wrapper_content.push_str(&format!("#include \"{}\"\n", chat_wrapper_header.display()));
        }
        let grammar_wrapper_header = manifest_dir.join("arkavo_grammar_wrapper.h");
        if grammar_wrapper_header.exists() {
            wrapper_content.push_str(&format!(
                "#include \"{}\"\n",
                grammar_wrapper_header.display()
            ));
        }
        let spec_wrapper_header = manifest_dir.join("arkavo_spec_wrapper.h");
        if spec_wrapper_header.exists() {
            wrapper_content.push_str(&format!("#include \"{}\"\n", spec_wrapper_header.display()));
        }
        std::fs::write(&wrapper_header, wrapper_content).expect("Failed to write wrapper header");

        let bindings = bindgen::Builder::default()
            .header(wrapper_header.to_str().unwrap())
            .clang_arg(format!("-I{}", out_path.join("include").display()))
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
            .allowlist_function("llama_.*")
            .allowlist_function("ggml_.*")
            .allowlist_function("mtmd_.*")
            .allowlist_function("clip_.*")
            .allowlist_function("arkavo_.*")
            .allowlist_type("llama_.*")
            .allowlist_type("ggml_.*")
            .allowlist_type("mtmd_.*")
            .allowlist_type("clip_.*")
            .allowlist_type("arkavo_.*")
            .allowlist_var("LLAMA_.*")
            .allowlist_var("GGML_.*")
            .allowlist_var("MTMD_.*")
            .generate()
            .expect("Unable to generate bindings");

        bindings
            .write_to_file(&bindings_path)
            .expect("Couldn't write bindings!");
    }
}
