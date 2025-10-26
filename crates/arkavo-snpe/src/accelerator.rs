use crate::error::{Result, SnpeError};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcceleratorTarget {
    Gpu,
    GpuFp16,
    Dsp,
    Cpu,
}

impl AcceleratorTarget {
    pub fn priority(&self) -> u8 {
        match self {
            AcceleratorTarget::GpuFp16 => 0,
            AcceleratorTarget::Gpu => 1,
            AcceleratorTarget::Dsp => 2,
            AcceleratorTarget::Cpu => 3,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            AcceleratorTarget::Gpu => "GPU",
            AcceleratorTarget::GpuFp16 => "GPU_FP16",
            AcceleratorTarget::Dsp => "DSP",
            AcceleratorTarget::Cpu => "CPU",
        }
    }

    pub fn from_env() -> Option<Self> {
        std::env::var("ARKAVO_SNPE_RUNTIME")
            .ok()
            .and_then(|s| s.parse().ok())
    }
}

impl fmt::Display for AcceleratorTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl std::str::FromStr for AcceleratorTarget {
    type Err = SnpeError;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_uppercase().as_str() {
            "GPU" => Ok(AcceleratorTarget::Gpu),
            "GPU_FP16" => Ok(AcceleratorTarget::GpuFp16),
            "DSP" => Ok(AcceleratorTarget::Dsp),
            "CPU" => Ok(AcceleratorTarget::Cpu),
            _ => Err(SnpeError::Config(format!("Unknown accelerator: {}", s))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeCapabilities {
    pub gpu_available: bool,
    pub gpu_fp16_available: bool,
    pub dsp_available: bool,
    pub cpu_available: bool,
}

impl RuntimeCapabilities {
    pub fn detect() -> Self {
        Self {
            gpu_available: Self::check_gpu(),
            gpu_fp16_available: Self::check_gpu_fp16(),
            dsp_available: Self::check_dsp(),
            cpu_available: true,
        }
    }

    fn check_gpu() -> bool {
        std::path::Path::new("/dev/kgsl-3d0").exists()
            || std::path::Path::new("/sys/class/kgsl/kgsl-3d0").exists()
    }

    fn check_gpu_fp16() -> bool {
        Self::check_gpu()
    }

    fn check_dsp() -> bool {
        std::path::Path::new("/dev/cdsp").exists()
            || std::path::Path::new("/sys/kernel/debug/msm_vidc").exists()
    }

    pub fn is_available(&self, target: AcceleratorTarget) -> bool {
        match target {
            AcceleratorTarget::Gpu => self.gpu_available,
            AcceleratorTarget::GpuFp16 => self.gpu_fp16_available,
            AcceleratorTarget::Dsp => self.dsp_available,
            AcceleratorTarget::Cpu => self.cpu_available,
        }
    }

    pub fn select_best(&self, preferred: Option<AcceleratorTarget>) -> AcceleratorTarget {
        if let Some(target) = preferred {
            if self.is_available(target) {
                return target;
            }
        }

        let fallback_order = [
            AcceleratorTarget::GpuFp16,
            AcceleratorTarget::Gpu,
            AcceleratorTarget::Dsp,
            AcceleratorTarget::Cpu,
        ];

        for target in &fallback_order {
            if self.is_available(*target) {
                log::info!("Selected accelerator: {}", target);
                return *target;
            }
        }

        log::warn!("No accelerators available, falling back to CPU");
        AcceleratorTarget::Cpu
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_accelerator_priority() {
        assert!(AcceleratorTarget::GpuFp16.priority() < AcceleratorTarget::Gpu.priority());
        assert!(AcceleratorTarget::Gpu.priority() < AcceleratorTarget::Dsp.priority());
        assert!(AcceleratorTarget::Dsp.priority() < AcceleratorTarget::Cpu.priority());
    }

    #[test]
    fn test_accelerator_from_str() {
        assert_eq!(
            "GPU_FP16".parse::<AcceleratorTarget>().unwrap(),
            AcceleratorTarget::GpuFp16
        );
        assert_eq!(
            "gpu".parse::<AcceleratorTarget>().unwrap(),
            AcceleratorTarget::Gpu
        );
        assert_eq!(
            "DSP".parse::<AcceleratorTarget>().unwrap(),
            AcceleratorTarget::Dsp
        );
        assert_eq!(
            "CPU".parse::<AcceleratorTarget>().unwrap(),
            AcceleratorTarget::Cpu
        );
    }

    #[test]
    fn test_runtime_capabilities() {
        let caps = RuntimeCapabilities::detect();
        assert!(caps.cpu_available);
    }

    #[test]
    fn test_select_best() {
        let caps = RuntimeCapabilities {
            gpu_available: false,
            gpu_fp16_available: false,
            dsp_available: true,
            cpu_available: true,
        };

        assert_eq!(caps.select_best(None), AcceleratorTarget::Dsp);
        assert_eq!(
            caps.select_best(Some(AcceleratorTarget::Gpu)),
            AcceleratorTarget::Dsp
        );
    }
}
