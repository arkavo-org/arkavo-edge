//! Pinned files, checked against the Hugging Face Hub and the release
//! digest on 2026-10-11. `tests::pins_match_the_hub` re-checks them.

use super::{Allowance, Entry, Limits, PinnedFile, Profile, RuntimePin, SetLabel, WeightSet};

const GIB: u64 = 1 << 30;

/// Provisional working set until measurement on an M-series host replaces
/// it (IMG-018): 3 GiB for 1024x1024 with guidance 1, and nothing else, so
/// larger or guided runs are refused rather than guessed at.
const PROVISIONAL: &[Allowance] = &[Allowance {
    pixels: 1024 * 1024,
    guided: false,
    bytes: 3 * GIB,
}];

pub const RUNTIME: RuntimePin = RuntimePin {
    build: 956,
    tag: "master-956-1b0ba10",
    url: "https://github.com/leejet/stable-diffusion.cpp/releases/download/master-956-1b0ba10/sd-master-1b0ba10-bin-Darwin-macOS-26.6.2-arm64.zip",
    size: 35_158_090,
    sha256: "72c18ca1ba4177ca77cd288a9cb1e0f78d63ad3bde4b91fd7c84f3f4187b5761",
    members: &[
        "sd-cli",
        "libstable-diffusion.dylib",
        "ggml.txt",
        "stable-diffusion.cpp.txt",
    ],
    cli: "sd-cli",
};

const COMFY: &str = "Comfy-Org/Qwen-Image-2.1";
const COMFY_REV: &str = "df94239739eef7205973a145ffa6f441f7b64e84";
const QWEN3VL: &str = "Qwen/Qwen3-VL-8B-Instruct-GGUF";
const QWEN3VL_REV: &str = "f982a07559d4a2f6c8744d840bf6fccab30eea96";
const TURBO_GGUF: &str = "AtomicChat/Qwen-Image-2.1-Turbo-GGUF";
const TURBO_GGUF_REV: &str = "bb25d06bc74119c12207243d68917951e6d9c232";
const FULL_GGUF: &str = "leejet/Qwen-Image-2.1-GGUF";
const FULL_GGUF_REV: &str = "cc11433936a06e9765f7c0c0b1f0436cfd2b9856";

static VAE: PinnedFile = PinnedFile {
    repo: COMFY,
    revision: COMFY_REV,
    path: "vae/qwen_image_2.1_vae_bf16.safetensors",
    size: 675_509_688,
    sha256: "bb21f7473051e1ac368515dd3f2e15cd44d7a11748ee8823e1ddca3e4876b7c9",
};

static ENCODER_F16: PinnedFile = PinnedFile {
    repo: QWEN3VL,
    revision: QWEN3VL_REV,
    path: "Qwen3VL-8B-Instruct-F16.gguf",
    size: 16_388_044_896,
    sha256: "2715c1a097f1943fb88ad59c7c0e9288a7a50c84d60cf18c6227bd9a40520972",
};

static ENCODER_Q8: PinnedFile = PinnedFile {
    repo: QWEN3VL,
    revision: QWEN3VL_REV,
    path: "Qwen3VL-8B-Instruct-Q8_0.gguf",
    size: 8_709_519_456,
    sha256: "0d264b3941185d00a74f75c4245521dae088ff1efc90ab8d1754e83f5844adb0",
};

static ENCODER_Q4: PinnedFile = PinnedFile {
    repo: QWEN3VL,
    revision: QWEN3VL_REV,
    path: "Qwen3VL-8B-Instruct-Q4_K_M.gguf",
    size: 5_027_784_800,
    sha256: "67d1659bfe71b89d50b45a4ad1a9e5b997e5bb16ce5da66a6a6167abd569e9e2",
};

static TURBO_BF16: PinnedFile = PinnedFile {
    repo: TURBO_GGUF,
    revision: TURBO_GGUF_REV,
    path: "Qwen-Image-2.1-Turbo-BF16.gguf",
    size: 14_230_275_808,
    sha256: "9849f4f5606e7130dafa8ca4fe6d773d91d83b0eed983f6245de3195c9105949",
};

static TURBO_Q8: PinnedFile = PinnedFile {
    repo: TURBO_GGUF,
    revision: TURBO_GGUF_REV,
    path: "Qwen-Image-2.1-Turbo-Q8_0.gguf",
    size: 7_591_554_784,
    sha256: "99f498fb7188be7eac9eb5f345a9e074d30eef3ca235b419e92563e5b48073c4",
};

static TURBO_Q4: PinnedFile = PinnedFile {
    repo: TURBO_GGUF,
    revision: TURBO_GGUF_REV,
    path: "Qwen-Image-2.1-Turbo-AD-Q4_K.gguf",
    size: 4_201_694_944,
    sha256: "4bb73c53cbe284bbd6d69b9fdc59539c531f0389dd2aa567663d777f8130bc59",
};

static FULL_BF16: PinnedFile = PinnedFile {
    repo: COMFY,
    revision: COMFY_REV,
    path: "diffusion_models/qwen_image_2.1_bf16.safetensors",
    size: 14_230_280_616,
    sha256: "89f4158d066cc33906a199fca85634f766892dd78f49b6698dabf187ac86c4bc",
};

static FULL_Q8: PinnedFile = PinnedFile {
    repo: FULL_GGUF,
    revision: FULL_GGUF_REV,
    path: "qwen_image_2.1-Q8_0.gguf",
    size: 7_687_155_744,
    sha256: "f8b244b00937f0e444a40dbf7866460871b89b30142594973b6012d1b471dc0a",
};

static FULL_Q4: PinnedFile = PinnedFile {
    repo: FULL_GGUF,
    revision: FULL_GGUF_REV,
    path: "qwen_image_2.1-Q4_K.gguf",
    size: 4_197_494_816,
    sha256: "29f9c83c249ff0292fb2943fceddfa2319b446601866c82a4f8be062abea72c2",
};

/// Qwen's recommended framing for transparent output, the same for both
/// Qwen-Image-2.1 checkpoints.
const QWEN_RGBA: (&str, &str) = (
    "This is an RGBA image with transparency. ",
    ". The image has alpha channel and the background is transparent.",
);

/// The Qwen-Image-2.1 presets top out at 2752 on a side and 2400x1792 in
/// area, so both entries accept every official aspect ratio.
const fn qwen_21_limits(default_side: u32) -> Limits {
    Limits {
        step: 32,
        min_side: 256,
        max_side: 2752,
        max_pixels: 2400 * 1792,
        default_width: default_side,
        default_height: default_side,
    }
}

pub(super) static ENTRIES: &[Entry] = &[
    Entry {
        id: "qwen-image-2.1-turbo",
        model_repo: "Qwen/Qwen-Image-2.1-Turbo",
        model_revision: "d65dbc9a7e8f6b5479e33dee6030eaab2a906509",
        summary: "Qwen-Image-2.1-Turbo: 8 steps, 1024x1024 by default; fast drafts and sprite sets",
        license: "Qwen Research License Agreement",
        license_url: "https://huggingface.co/Qwen/Qwen-Image-2.1-Turbo/blob/main/LICENSE",
        // The distilled schedule is the checkpoint's own sample_sigmas from
        // model_index.json, plus the terminal 0.0 sd.cpp expects.
        profile: Profile {
            steps: 8,
            sampler: "euler",
            sigmas: Some("1.0,0.978453,0.95418,0.926626,0.89508,0.845148,0.704534,0.414568,0.0"),
            max_guidance: None,
        },
        limits: qwen_21_limits(1024),
        rgba_template: Some(QWEN_RGBA),
        min_runtime_build: 956,
        sets: &[
            WeightSet {
                label: SetLabel::Xl,
                diffusion: &TURBO_BF16,
                text_encoder: &ENCODER_F16,
                vae: &VAE,
            },
            WeightSet {
                label: SetLabel::L,
                diffusion: &TURBO_Q8,
                text_encoder: &ENCODER_Q8,
                vae: &VAE,
            },
            WeightSet {
                label: SetLabel::M,
                diffusion: &TURBO_Q8,
                text_encoder: &ENCODER_Q4,
                vae: &VAE,
            },
            WeightSet {
                label: SetLabel::S,
                diffusion: &TURBO_Q4,
                text_encoder: &ENCODER_Q4,
                vae: &VAE,
            },
        ],
        allowances: PROVISIONAL,
    },
    Entry {
        id: "qwen-image-2.1",
        model_repo: "Qwen/Qwen-Image-2.1",
        model_revision: "d26bb61231c349cf6b7896fa83353113880e1ba3",
        summary: "Qwen-Image-2.1: 40 steps, 2048x2048 by default, optional guidance with a negative prompt; final art",
        license: "Qwen Research License Agreement",
        license_url: "https://huggingface.co/Qwen/Qwen-Image-2.1/blob/main/LICENSE",
        // QwenImage21Pipeline defaults to 40 steps and true_cfg_scale 1.0;
        // no --sigmas, so sd.cpp follows the resolution-dependent shift the
        // scheduler config sets (use_dynamic_shifting).
        profile: Profile {
            steps: 40,
            sampler: "euler",
            sigmas: None,
            max_guidance: Some(10.0),
        },
        limits: qwen_21_limits(2048),
        rgba_template: Some(QWEN_RGBA),
        min_runtime_build: 956,
        sets: &[
            WeightSet {
                label: SetLabel::Xl,
                diffusion: &FULL_BF16,
                text_encoder: &ENCODER_F16,
                vae: &VAE,
            },
            WeightSet {
                label: SetLabel::L,
                diffusion: &FULL_Q8,
                text_encoder: &ENCODER_Q8,
                vae: &VAE,
            },
            WeightSet {
                label: SetLabel::M,
                diffusion: &FULL_Q8,
                text_encoder: &ENCODER_Q4,
                vae: &VAE,
            },
            WeightSet {
                label: SetLabel::S,
                diffusion: &FULL_Q4,
                text_encoder: &ENCODER_Q4,
                vae: &VAE,
            },
        ],
        allowances: PROVISIONAL,
    },
];
