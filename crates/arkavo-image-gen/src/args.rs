//! The stable-diffusion.cpp command line for a job (IMG-008, IMG-009).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::request::Job;

/// The weight files a run loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPaths {
    pub diffusion: PathBuf,
    pub text_encoder: PathBuf,
    pub vae: PathBuf,
}

/// Where a run's prompts are written. Passing them as files keeps them out
/// of the process table, which every local account can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptFiles {
    pub prompt: PathBuf,
    pub negative: Option<PathBuf>,
}

/// The -o pattern; sd-cli numbers batch images from 0 when given `%03d`.
pub fn output_pattern(job: &Job) -> String {
    format!("{}_%03d.png", job.name)
}

/// Every argument after the program name.
pub fn sd_args(
    job: &Job,
    models: &ModelPaths,
    prompts: &PromptFiles,
    out_dir: &Path,
) -> Vec<OsString> {
    let profile = &job.entry.profile;
    let mut args: Vec<OsString> = Vec::new();
    let mut push = |flag: &str, value: OsString| {
        args.push(flag.into());
        args.push(value);
    };
    push("--diffusion-model", models.diffusion.clone().into());
    push("--llm", models.text_encoder.clone().into());
    push("--vae", models.vae.clone().into());
    push("--prompt-file", prompts.prompt.clone().into());
    if let Some(negative) = &prompts.negative {
        push("--negative-prompt-file", negative.clone().into());
    }
    push("-W", job.width.to_string().into());
    push("-H", job.height.to_string().into());
    push("--steps", profile.steps.to_string().into());
    push("--cfg-scale", format!("{:.2}", job.guidance).into());
    push("--sampling-method", profile.sampler.into());
    if let Some(sigmas) = profile.sigmas {
        push("--sigmas", sigmas.into());
    }
    push("-s", job.seed.to_string().into());
    push("-b", job.count.to_string().into());
    push("-o", out_dir.join(output_pattern(job)).into());
    args.push("--diffusion-fa".into());
    args.push("--disable-image-metadata".into());
    if job.tileable {
        args.push("--circular".into());
    }
    args
}

/// Prompts go to files only the agent's user can read (IMG-006).
pub async fn write_prompts(job: &Job, dir: &Path) -> Result<PromptFiles> {
    let prompt = dir.join(".prompt");
    write_private(&prompt, &job.effective_prompt()).await?;
    let negative = match &job.negative_prompt {
        Some(text) => {
            let path = dir.join(".negative");
            write_private(&path, text).await?;
            Some(path)
        }
        None => None,
    };
    Ok(PromptFiles { prompt, negative })
}

async fn write_private(path: &Path, text: &str) -> Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).await?;
    tokio::io::AsyncWriteExt::write_all(&mut file, text.as_bytes()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::ImageRequest;

    fn models() -> ModelPaths {
        ModelPaths {
            diffusion: "/m/d.gguf".into(),
            text_encoder: "/m/te.gguf".into(),
            vae: "/m/vae.safetensors".into(),
        }
    }

    fn prompts(negative: bool) -> PromptFiles {
        PromptFiles {
            prompt: "/out/.prompt".into(),
            negative: negative.then(|| "/out/.negative".into()),
        }
    }

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter()
            .position(|a| a == flag)
            .map(|i| args[i + 1].as_str())
    }

    /// IMG-008: Turbo gets its distilled schedule at CFG 1.
    #[arkavo_test_macros::spec("IMG-008")]
    #[test]
    fn turbo_uses_its_sigmas() {
        let job = ImageRequest {
            prompt: "coin".into(),
            seed: Some(7),
            count: Some(2),
            name: Some("coin".into()),
            ..ImageRequest::default()
        }
        .validate()
        .unwrap();
        let args = strings(&sd_args(
            &job,
            &models(),
            &prompts(false),
            Path::new("/out"),
        ));
        assert_eq!(value(&args, "--steps"), Some("8"));
        assert_eq!(value(&args, "--cfg-scale"), Some("1.00"));
        assert_eq!(value(&args, "--sampling-method"), Some("euler"));
        assert_eq!(
            value(&args, "--sigmas"),
            Some("1.0,0.978453,0.95418,0.926626,0.89508,0.845148,0.704534,0.414568,0.0")
        );
        assert_eq!(value(&args, "-s"), Some("7"));
        assert_eq!(value(&args, "-b"), Some("2"));
        assert_eq!(value(&args, "-o"), Some("/out/coin_%03d.png"));
        assert_eq!(value(&args, "--llm"), Some("/m/te.gguf"));
        assert_eq!(value(&args, "--prompt-file"), Some("/out/.prompt"));
        assert!(args.contains(&"--diffusion-fa".to_string()));
        assert!(args.contains(&"--disable-image-metadata".to_string()));
        assert!(!args.contains(&"--circular".to_string()));
        assert!(
            !args
                .iter()
                .any(|a| a.contains("coin") && !a.starts_with("/out/coin"))
        );
        assert_eq!(value(&args, "--negative-prompt-file"), None);
    }

    /// IMG-008: the full model lets sd.cpp pick its schedule and applies
    /// guidance with the negative prompt.
    #[arkavo_test_macros::spec("IMG-008")]
    #[test]
    fn full_model_uses_guidance_and_no_sigmas() {
        let job = ImageRequest {
            prompt: "castle".into(),
            model: Some("qwen-image-2.1".into()),
            guidance: Some(4.0),
            negative_prompt: Some("blurry".into()),
            tileable: Some(true),
            ..ImageRequest::default()
        }
        .validate()
        .unwrap();
        let args = strings(&sd_args(&job, &models(), &prompts(true), Path::new("/out")));
        assert_eq!(value(&args, "--steps"), Some("40"));
        assert_eq!(value(&args, "--cfg-scale"), Some("4.00"));
        assert_eq!(value(&args, "--sigmas"), None);
        assert_eq!(
            value(&args, "--negative-prompt-file"),
            Some("/out/.negative")
        );
        assert_eq!(value(&args, "-W"), Some("2048"));
        assert!(args.contains(&"--circular".to_string()));
    }
}
