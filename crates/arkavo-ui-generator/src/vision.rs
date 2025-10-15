use anyhow::{Context, Result};
use arkavo_llm::{Message, encode_image_bytes};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct ScreenshotCapture {
    pub image_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
}

#[derive(Debug, Clone, Copy)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ScreenshotCapture {
    #[cfg(target_os = "macos")]
    pub fn capture_window() -> Result<Self> {
        use std::fs;
        use tempfile::NamedTempFile;

        let temp_file = NamedTempFile::new().context("Failed to create temporary file")?;
        let temp_path = temp_file.path();

        let output = Command::new("screencapture")
            .arg("-i")
            .arg("-t")
            .arg("png")
            .arg(temp_path)
            .output()
            .context("Failed to execute screencapture")?;

        if !output.status.success() {
            anyhow::bail!("screencapture failed: {:?}", output.stderr);
        }

        let image_data = fs::read(temp_path).context("Failed to read screenshot file")?;

        let (width, height) = Self::get_png_dimensions(&image_data)?;

        Ok(Self {
            image_data,
            width,
            height,
            format: ImageFormat::Png,
        })
    }

    #[cfg(target_os = "linux")]
    pub fn capture_window() -> Result<Self> {
        use std::fs;
        use tempfile::NamedTempFile;

        let temp_file = NamedTempFile::new().context("Failed to create temporary file")?;
        let temp_path = temp_file.path();

        let output = Command::new("import")
            .arg(temp_path)
            .output()
            .context("Failed to execute import (ImageMagick)")?;

        if !output.status.success() {
            anyhow::bail!("import command failed: {:?}", output.stderr);
        }

        let image_data = fs::read(temp_path).context("Failed to read screenshot file")?;

        let (width, height) = Self::get_png_dimensions(&image_data)?;

        Ok(Self {
            image_data,
            width,
            height,
            format: ImageFormat::Png,
        })
    }

    #[cfg(target_os = "windows")]
    pub fn capture_window() -> Result<Self> {
        anyhow::bail!("Screenshot capture not yet implemented for Windows");
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let image_data = std::fs::read(path)
            .with_context(|| format!("Failed to read image file: {:?}", path))?;

        let format = if image_data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
            ImageFormat::Png
        } else if image_data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            ImageFormat::Jpeg
        } else {
            anyhow::bail!("Unsupported image format");
        };

        let (width, height) = match format {
            ImageFormat::Png => Self::get_png_dimensions(&image_data)?,
            ImageFormat::Jpeg => Self::get_jpeg_dimensions(&image_data)?,
        };

        Ok(Self {
            image_data,
            width,
            height,
            format,
        })
    }

    pub fn to_base64(&self) -> Result<String> {
        encode_image_bytes(&self.image_data).context("Failed to encode image to base64")
    }

    pub fn create_vision_message(&self, prompt: &str) -> Result<Message> {
        let base64_image = self.to_base64()?;
        Ok(Message::user_with_images(
            prompt.to_string(),
            vec![base64_image],
        ))
    }

    fn get_png_dimensions(data: &[u8]) -> Result<(u32, u32)> {
        if data.len() < 24 {
            anyhow::bail!("PNG data too small");
        }

        if !data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
            anyhow::bail!("Invalid PNG header");
        }

        let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);

        Ok((width, height))
    }

    fn get_jpeg_dimensions(data: &[u8]) -> Result<(u32, u32)> {
        if data.len() < 10 || !data.starts_with(&[0xFF, 0xD8]) {
            anyhow::bail!("Invalid JPEG header");
        }

        let mut pos = 2;
        while pos + 9 < data.len() {
            if data[pos] != 0xFF {
                anyhow::bail!("Invalid JPEG marker");
            }

            let marker = data[pos + 1];
            if marker == 0xC0 || marker == 0xC2 {
                let height = u16::from_be_bytes([data[pos + 5], data[pos + 6]]) as u32;
                let width = u16::from_be_bytes([data[pos + 7], data[pos + 8]]) as u32;
                return Ok((width, height));
            }

            let length = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
            pos += length + 2;
        }

        anyhow::bail!("Could not find JPEG dimensions");
    }
}

pub struct UiVerifier {
    llm_client: Option<Box<dyn arkavo_llm::Provider>>,
}

impl UiVerifier {
    pub fn new(llm_client: Option<Box<dyn arkavo_llm::Provider>>) -> Self {
        Self { llm_client }
    }

    pub async fn verify_ui(
        &self,
        screenshot: &ScreenshotCapture,
        requirements: &str,
    ) -> Result<VerificationResult> {
        let llm = self
            .llm_client
            .as_ref()
            .context("LLM client not configured for vision verification")?;

        let prompt = format!(
            "You are analyzing a UI screenshot. Requirements: {}\n\n\
             Analyze the screenshot and determine if it matches the requirements. \
             Respond with:\n\
             1. PASS or FAIL\n\
             2. Brief explanation (1-2 sentences)\n\
             3. Suggested improvements (if any)",
            requirements
        );

        let message = screenshot.create_vision_message(&prompt)?;

        let response = llm
            .complete_with_options(vec![message], Some(500))
            .await
            .context("Failed to get LLM response")?;

        let passed =
            response.to_uppercase().contains("PASS") && !response.to_uppercase().contains("FAIL");

        Ok(VerificationResult {
            passed,
            feedback: response,
            screenshot_width: screenshot.width,
            screenshot_height: screenshot.height,
        })
    }
}

#[derive(Debug, Clone)]
pub struct VerificationResult {
    pub passed: bool,
    pub feedback: String,
    pub screenshot_width: u32,
    pub screenshot_height: u32,
}

impl VerificationResult {
    pub fn display_summary(&self) -> String {
        let status = if self.passed {
            "✅ PASSED"
        } else {
            "❌ FAILED"
        };
        format!(
            "{} ({}x{})\n\nFeedback:\n{}",
            status, self.screenshot_width, self.screenshot_height, self.feedback
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_png_dimensions() {
        let png_header = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x03, 0x00,
        ];

        let result = ScreenshotCapture::get_png_dimensions(&png_header);
        assert!(result.is_ok());
        let (width, height) = result.unwrap();
        assert_eq!(width, 1024);
        assert_eq!(height, 768);
    }

    #[test]
    fn test_invalid_png() {
        let invalid_data = vec![0x00, 0x01, 0x02, 0x03];
        let result = ScreenshotCapture::get_png_dimensions(&invalid_data);
        assert!(result.is_err());
    }
}
