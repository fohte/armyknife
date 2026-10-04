use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::infra::external_tool::ExternalTool;
use crate::shared::cache;

const ICON_URL: &str =
    "https://raw.githubusercontent.com/primer/octicons/main/icons/git-merge-24.svg";
const ICON_FILENAME: &str = "git-merge-24-a371f7.png";
const ICON_FILL: &str = "#a371f7";

pub async fn ensure_icon() -> Option<PathBuf> {
    let cache_dir = cache::base_dir()?;
    match ensure_icon_with(&cache_dir, download_svg, convert_svg).await {
        Ok(path) => Some(path),
        Err(error) => {
            tracing::warn!(event = "notification.merge_icon_failed", error = %error);
            None
        }
    }
}

pub(crate) async fn ensure_icon_with<F, Fut, C>(
    cache_dir: &Path,
    download: F,
    convert: C,
) -> Result<PathBuf>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<String>>,
    C: FnOnce(&str) -> Result<Vec<u8>>,
{
    let icon_path = cache_dir.join(ICON_FILENAME);
    if icon_path.is_file() {
        return Ok(icon_path);
    }

    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("creating icon cache directory {}", cache_dir.display()))?;
    let svg = download().await?;
    let svg = add_fill(&svg)?;
    let png = convert(&svg)?;
    if png.is_empty() {
        bail!("ImageMagick produced an empty PNG");
    }

    let mut cache_file = tempfile::NamedTempFile::new_in(cache_dir)
        .with_context(|| format!("creating cached icon in {}", cache_dir.display()))?;
    cache_file
        .write_all(&png)
        .context("writing cached merge icon")?;
    cache_file.flush().context("flushing cached merge icon")?;
    if let Err(error) = cache_file.persist(&icon_path) {
        if icon_path.is_file() {
            return Ok(icon_path);
        }
        return Err(error.error).context("persisting cached merge icon");
    }

    Ok(icon_path)
}

async fn download_svg() -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("creating Octicons HTTP client")?;
    let response = client
        .get(ICON_URL)
        .send()
        .await
        .context("downloading the Octicons git-merge icon")?;
    let response = response
        .error_for_status()
        .context("Octicons returned an unsuccessful response")?;
    response
        .text()
        .await
        .context("reading the Octicons git-merge SVG")
}

fn add_fill(svg: &str) -> Result<String> {
    if !svg.contains("<svg") {
        bail!("Octicons response does not contain an SVG element");
    }
    Ok(svg.replacen("<svg", &format!("<svg fill=\"{ICON_FILL}\""), 1))
}

fn convert_svg(svg: &str) -> Result<Vec<u8>> {
    let mut child = ExternalTool::ImageMagick
        .command()
        .args([
            "-background",
            "none",
            "-density",
            "1024",
            "svg:-",
            "-resize",
            "256x256",
            "png:-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting ImageMagick")?;
    let mut stdin = child.stdin.take().context("opening ImageMagick input")?;
    if let Err(error) = stdin.write_all(svg.as_bytes()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error).context("sending SVG to ImageMagick");
    }
    drop(stdin);

    let output = child
        .wait_with_output()
        .context("waiting for ImageMagick")?;
    if !output.status.success() {
        bail!(
            "ImageMagick failed to convert the SVG: {}",
            String::from_utf8_lossy(&output.stderr).trim(),
        );
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn caches_the_converted_icon_and_reuses_it() {
        let temp = TempDir::new().unwrap();
        let fetch_calls = Cell::new(0);
        let convert_calls = Cell::new(0);

        let first_path = ensure_icon_with(
            temp.path(),
            || async {
                fetch_calls.set(fetch_calls.get() + 1);
                Ok("<svg></svg>".to_string())
            },
            |svg| {
                convert_calls.set(convert_calls.get() + 1);
                if svg == "<svg fill=\"#a371f7\"></svg>" {
                    Ok(b"png-bytes".to_vec())
                } else {
                    bail!("unexpected SVG input")
                }
            },
        )
        .await
        .unwrap();
        let second_path = ensure_icon_with(
            temp.path(),
            || async { bail!("cached icon should not be downloaded") },
            |_| bail!("cached icon should not be converted"),
        )
        .await
        .unwrap();

        assert_eq!(
            (
                first_path,
                second_path,
                std::fs::read(temp.path().join(ICON_FILENAME)).unwrap(),
                fetch_calls.get(),
                convert_calls.get(),
            ),
            (
                temp.path().join(ICON_FILENAME),
                temp.path().join(ICON_FILENAME),
                b"png-bytes".to_vec(),
                1,
                1,
            ),
        );
    }

    #[tokio::test]
    async fn download_or_conversion_failure_does_not_leave_a_cached_icon() {
        let temp = TempDir::new().unwrap();
        let mut results = Vec::new();
        let mut cached_files = Vec::new();

        for download_fails in [true, false] {
            let dir = temp.path().join(if download_fails {
                "download-failure"
            } else {
                "conversion-failure"
            });
            let result = ensure_icon_with(
                &dir,
                || async move {
                    if download_fails {
                        bail!("download failed")
                    }
                    Ok("<svg></svg>".to_string())
                },
                |_| bail!("conversion failed"),
            )
            .await;
            results.push(result.is_err());
            cached_files.push(dir.join(ICON_FILENAME).exists());
        }

        assert_eq!(
            (results, cached_files),
            (vec![true, true], vec![false, false])
        );
    }
}
