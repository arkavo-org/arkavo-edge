//! Progress a request reports, and reading it from sd-cli's output
//! (IMG-005).

use std::fmt;

/// What a running request is doing, for the requester's task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// Weights or the runtime being fetched.
    Fetching { fetched: u64, needed: u64 },
    /// Waiting behind other generations.
    Queued,
    /// Denoising step `step` of `steps` for image `image` of `images`.
    Step {
        image: u32,
        images: u32,
        step: u32,
        steps: u32,
    },
    /// What the member generating a delegated request last reported.
    Relayed { member: String, message: String },
}

impl fmt::Display for Progress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Fetching { fetched, needed } => {
                let gb = |b: u64| b as f64 / 1e9;
                write!(
                    f,
                    "fetching model weights: {:.2} of {:.2} GB",
                    gb(fetched),
                    gb(needed)
                )
            }
            Self::Queued => write!(f, "waiting for the image generator"),
            Self::Step {
                image,
                images,
                step,
                steps,
            } => write!(f, "image {image} of {images}: step {step} of {steps}"),
            Self::Relayed {
                ref member,
                ref message,
            } => write!(f, "{member}: {message}"),
        }
    }
}

/// Something sd-cli printed that the generator acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// "generating image: i/n - seed s"
    ImageStarted { image: u32, images: u32, seed: u32 },
    /// A progress bar line "|===>  | s/S - 1.23s/it" while sampling.
    Step { step: u32, steps: u32 },
    /// The GPU failed a command, most often for lack of memory. sd-cli
    /// carries on and writes an image from whatever the GPU left behind,
    /// which is garbage, and exits as if it had succeeded.
    GpuFailed,
}

/// Turns sd-cli's output, split on `\r` and `\n`, into events. Progress
/// bars count as steps only after an image has started and only with the
/// job's step count, since model loading prints bars of its own.
#[derive(Debug)]
pub struct Parser {
    steps: u32,
    sampling: bool,
}

impl Parser {
    pub const fn new(steps: u32) -> Self {
        Self {
            steps,
            sampling: false,
        }
    }

    pub fn line(&mut self, line: &str) -> Option<Event> {
        if (line.contains("command buffer") && line.contains("failed with status"))
            || line.contains("CommandBufferCallbackErrorOutOfMemory")
        {
            return Some(Event::GpuFailed);
        }
        if let Some(rest) = line.split("generating image: ").nth(1) {
            let (counts, seed) = rest.split_once(" - seed ")?;
            let (image, images) = fraction(counts)?;
            self.sampling = true;
            return Some(Event::ImageStarted {
                image,
                images,
                seed: seed.trim().parse::<u64>().ok().map(|s| s as u32)?,
            });
        }
        if !self.sampling || !line.contains('|') {
            return None;
        }
        let tail = line.rsplit('|').next()?;
        let counts = tail.split(" - ").next()?;
        let (step, steps) = fraction(counts)?;
        (steps == self.steps).then_some(Event::Step { step, steps })
    }
}

fn fraction(text: &str) -> Option<(u32, u32)> {
    let (a, b) = text.trim().split_once('/')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[arkavo_test_macros::spec("IMG-005")]
    #[test]
    fn image_and_step_lines_become_events() {
        let mut parser = Parser::new(8);
        assert_eq!(parser.line("  |=====>      | 3/8 - 1.50s/it"), None);
        assert_eq!(
            parser.line("[INFO ] stable-diffusion.cpp:3405 - generating image: 1/2 - seed 42"),
            Some(Event::ImageStarted {
                image: 1,
                images: 2,
                seed: 42
            })
        );
        assert_eq!(
            parser.line("  |=====>      | 3/8 - 1.50s/it\u{1b}[K"),
            Some(Event::Step { step: 3, steps: 8 })
        );
        // A bar with another total is model loading or decoding, not a step.
        assert_eq!(parser.line("  |=====>      | 120/731 - 9.0it/s"), None);
        assert_eq!(
            parser.line("[INFO ] sampling completed, taking 12.3s"),
            None
        );
    }

    /// sd-cli prints the seed as a 64-bit value; seeds are 32-bit here.
    #[test]
    fn seeds_are_truncated_to_32_bits() {
        let mut parser = Parser::new(40);
        assert_eq!(
            parser.line("generating image: 2/4 - seed 4294967297"),
            Some(Event::ImageStarted {
                image: 2,
                images: 4,
                seed: 1
            })
        );
    }

    #[test]
    fn malformed_lines_are_ignored() {
        let mut parser = Parser::new(8);
        assert_eq!(parser.line("generating image: x/2 - seed 4"), None);
        assert_eq!(parser.line("generating image: 1/2"), None);
        parser.line("generating image: 1/1 - seed 1");
        assert_eq!(parser.line("| abc - 1s/it"), None);
    }

    #[test]
    fn progress_reads_as_status_text() {
        assert_eq!(
            Progress::Step {
                image: 1,
                images: 2,
                step: 3,
                steps: 40
            }
            .to_string(),
            "image 1 of 2: step 3 of 40"
        );
        assert_eq!(
            Progress::Fetching {
                fetched: 3_200_000_000,
                needed: 9_900_000_000
            }
            .to_string(),
            "fetching model weights: 3.20 of 9.90 GB"
        );
        assert_eq!(
            Progress::Queued.to_string(),
            "waiting for the image generator"
        );
    }

    /// IMG-021: a failed Metal command buffer is a GPU failure, whatever
    /// the run prints after it.
    #[arkavo_test_macros::spec("IMG-021")]
    #[test]
    fn a_failed_command_buffer_is_a_gpu_failure() {
        let mut parser = Parser::new(8);
        assert_eq!(
            parser.line(
                "[E] ggml: ggml_metal_synchronize: error: command buffer 1 failed with status 5"
            ),
            Some(Event::GpuFailed)
        );
        assert_eq!(
            parser.line("[E] ggml: error: Insufficient Memory (00000008:kIOGPUCommandBufferCallbackErrorOutOfMemory)"),
            Some(Event::GpuFailed)
        );
        assert_eq!(parser.line("[I] latent 1 decoded, taking 13.98s"), None);
    }
}
