//! Taking delivery of the assets an agent's answer references (CLI-018).
//!
//! The answer carries references only. Each asset is fetched from the agent
//! that staged it over the data plane, checked against its manifest, and
//! written beside the others for the task; a file already there is kept.

use std::io::Write;
use std::path::{Path, PathBuf};

use arkavo_image_gen::transfer::{FetchPolicy, fetch};
use arkavo_image_gen::{ASSET_MANIFEST_SCHEMA, Delivery};
use arkavo_protocol::types::MessagePart;
use arkavo_tdf_iroh::{IrohNode, IrohTransport};

/// Where assets go when no directory is named.
pub const DEFAULT_DIR: &str = "arkavo-assets";

/// Every asset manifest in an answer, in order.
pub fn deliveries(parts: &[MessagePart]) -> Vec<Delivery> {
    parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Data { schema, content } if schema == ASSET_MANIFEST_SCHEMA => {
                match serde_json::from_value(content.clone()) {
                    Ok(delivery) => Some(delivery),
                    Err(e) => {
                        tracing::warn!(error = %e, "unreadable asset manifest");
                        None
                    }
                }
            }
            _ => None,
        })
        .collect()
}

/// The directory a task's assets are written to. The task id comes from the
/// remote agent, so anything but a plain id is replaced rather than used as
/// a path.
pub fn task_dir(out: &Path, task_id: &str) -> PathBuf {
    let plain = !task_id.is_empty()
        && task_id.len() <= 64
        && task_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if plain {
        out.join(task_id)
    } else {
        out.join(format!("task-{:016x}", rand::random::<u64>()))
    }
}

/// Fetch, verify and save every asset of `deliveries` under `dir`.
///
/// One line per file and the license of each set go to `w`. Assets that
/// fail a check are reported and the rest are kept; the result is an error
/// when any failed.
pub async fn save<W: Write>(
    deliveries: &[Delivery],
    dir: &Path,
    w: &mut W,
) -> Result<(), Box<dyn std::error::Error>> {
    if deliveries.is_empty() {
        return Ok(());
    }
    tokio::fs::create_dir_all(dir).await?;
    let node = IrohNode::memory()
        .await
        .map_err(|e| format!("could not start the data plane to fetch assets: {e}"))?;
    let transport = IrohTransport::new(node);
    let mut failed = 0;
    for delivery in deliveries {
        let fetched = fetch(&transport, delivery, dir, FetchPolicy::default()).await;
        for (entry, path) in &fetched.saved {
            let alpha = if entry.has_alpha { "alpha" } else { "no alpha" };
            writeln!(
                w,
                "  {}  {} bytes, {}x{}, {alpha}",
                path.display(),
                entry.bytes,
                entry.width,
                entry.height
            )?;
        }
        for (name, error) in &fetched.failed {
            writeln!(w, "  ✗ {name}: {error}")?;
        }
        failed += fetched.failed.len();
        writeln!(w, "  {}", delivery.manifest.license_line())?;
    }
    let _ = transport.shutdown().await;
    if failed > 0 {
        return Err(format!("{failed} asset(s) could not be taken").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // `#[tokio::test]` expands to `Runtime::block_on`, which the workspace
    // lint set disallows in library code.
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use arkavo_image_gen::testing::generated;
    use arkavo_image_gen::transfer::stage;
    use arkavo_test_macros::spec;

    /// CLI-018: each asset is fetched, verified and saved under the task's
    /// directory, with its size, dimensions and alpha, then the license.
    #[spec("CLI-018")]
    #[tokio::test]
    async fn delivered_assets_are_saved_and_described() {
        let src = tempfile::tempdir().unwrap();
        let assets = generated(src.path(), &["coin_000.png"]);
        let producer = IrohTransport::new(IrohNode::memory().await.unwrap());
        let delivery = stage(&producer, &assets).await.unwrap();
        let parts = vec![
            MessagePart::Text {
                content: "Here is your coin.".into(),
            },
            MessagePart::Data {
                schema: ASSET_MANIFEST_SCHEMA.to_string(),
                content: serde_json::to_value(&delivery).unwrap(),
            },
        ];
        let out = tempfile::tempdir().unwrap();
        let dir = task_dir(out.path(), "7f1f4a2e-6f0e-4c3b-9d67-2f6c1f0a9b11");
        let mut printed = Vec::new();

        save(&deliveries(&parts), &dir, &mut printed).await.unwrap();

        let printed = String::from_utf8(printed).unwrap();
        let saved = dir.join("coin_000.png");
        assert_eq!(
            std::fs::read(&saved).unwrap(),
            std::fs::read(src.path().join("coin_000.png")).unwrap()
        );
        assert!(printed.contains("32x32, alpha"), "{printed}");
        assert!(
            printed.contains("license: Qwen Research License"),
            "{printed}"
        );
        assert!(!printed.contains("base64"));

        // A second delivery to the same place keeps the file that is there.
        let mut again = Vec::new();
        assert!(save(&deliveries(&parts), &dir, &mut again).await.is_err());
        assert!(
            String::from_utf8(again)
                .unwrap()
                .contains("nothing was overwritten")
        );
        producer.shutdown().await.unwrap();
    }

    #[test]
    fn task_ids_that_are_not_plain_never_become_paths() {
        let out = Path::new("/tmp/out");
        assert_eq!(task_dir(out, "abc-123"), out.join("abc-123"));
        for hostile in ["../../etc", "a/b", "", "..", "x y"] {
            let dir = task_dir(out, hostile);
            assert_eq!(dir.parent(), Some(out), "{hostile}");
            assert!(
                dir.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("task-")
            );
        }
    }
}
