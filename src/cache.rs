//! On-disk cache: catalog index + downloaded video files.
//!
//! Layout (under `$XDG_CACHE_HOME/aerial-linux/`, e.g. `~/.cache/aerial-linux/`):
//! ```text
//!   catalog.json        merged, normalized manifest of all sources
//!   videos/<id>.mov     downloaded clips, keyed by manifest id
//! ```
//! Downloads stream to a `.part` file, are md5-verified when the manifest
//! carries a digest, then atomically renamed into place.

use crate::catalog::{Catalog, Video, VideoFormat};
use anyhow::{Context, Result};
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use md5::{Digest, Md5};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

pub struct Cache {
    root: PathBuf,
}

impl Cache {
    /// Open (and lazily create) the cache directory.
    pub fn open() -> Result<Cache> {
        let dirs = directories::ProjectDirs::from("", "", "aerial-linux")
            .context("could not determine cache directory")?;
        let root = dirs.cache_dir().to_path_buf();
        std::fs::create_dir_all(root.join("videos")).with_context(|| {
            format!("creating cache dir {}", root.join("videos").display())
        })?;
        Ok(Cache { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn catalog_path(&self) -> PathBuf {
        self.root.join("catalog.json")
    }

    /// Path a clip is (or would be) cached at. Extension comes from the URL.
    pub fn video_path(&self, id: &str, url: &str) -> PathBuf {
        let ext = Path::new(url)
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| e.len() <= 4)
            .unwrap_or("mov");
        self.root.join("videos").join(format!("{id}.{ext}"))
    }

    /// True if any file named `<id>.*` already exists in the videos dir.
    pub fn is_cached(&self, id: &str) -> bool {
        let dir = self.root.join("videos");
        let prefix = format!("{id}.");
        std::fs::read_dir(dir)
            .ok()
            .map(|rd| {
                rd.flatten().any(|e| {
                    e.file_name()
                        .to_str()
                        .is_some_and(|n| n.starts_with(&prefix))
                })
            })
            .unwrap_or(false)
    }

    pub fn save_catalog(&self, catalog: &Catalog) -> Result<()> {
        let json = serde_json::to_vec_pretty(catalog)?;
        std::fs::write(self.catalog_path(), json)
            .with_context(|| format!("writing {}", self.catalog_path().display()))?;
        Ok(())
    }

    pub fn load_catalog(&self) -> Result<Catalog> {
        let bytes = std::fs::read(self.catalog_path()).with_context(|| {
            format!(
                "no catalog at {} — run `aerial-linux fetch` first",
                self.catalog_path().display()
            )
        })?;
        serde_json::from_slice(&bytes).context("parsing catalog.json")
    }

    /// Stream-download a video for the chosen format preference. Returns the
    /// path it was cached at. No-op (returns existing path) if already cached.
    pub async fn download_video(
        &self,
        client: &reqwest::Client,
        video: &Video,
        preference: &[VideoFormat],
    ) -> Result<PathBuf> {
        let (fmt, url) = video
            .best_url(preference)
            .with_context(|| format!("no usable URL for video {}", video.id))?;
        let dest = self.video_path(&video.id, url);

        if dest.exists() {
            return Ok(dest);
        }

        let resp = client
            .get(url)
            .send()
            .await
            .with_context(|| format!("requesting {url}"))?
            .error_for_status()
            .with_context(|| format!("HTTP error downloading {}", video.id))?;
        let total = resp.content_length().unwrap_or(0);

        let pb = ProgressBar::new(total);
        pb.set_style(
            ProgressStyle::with_template(
                "  {msg} [{bar:30}] {bytes}/{total_bytes} ({bytes_per_sec})",
            )
            .unwrap()
            .progress_chars("=> "),
        );
        pb.set_message(format!("{} [{}]", video.id, fmt.as_str()));

        let part = dest.with_extension("part");
        let mut file = tokio::fs::File::create(&part)
            .await
            .with_context(|| format!("creating {}", part.display()))?;
        let mut hasher = Md5::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("reading download stream")?;
            hasher.update(&chunk);
            file.write_all(&chunk).await.context("writing to cache")?;
            pb.inc(chunk.len() as u64);
        }
        file.flush().await?;
        pb.finish_and_clear();

        // Verify md5 if the manifest provided one for this format.
        if let Some(expected) = video.md5s.get(&fmt) {
            let got = hex::encode(hasher.finalize());
            if &got != expected {
                let _ = tokio::fs::remove_file(&part).await;
                anyhow::bail!(
                    "md5 mismatch for {} [{}]: expected {expected}, got {got}",
                    video.id,
                    fmt.as_str()
                );
            }
        }

        tokio::fs::rename(&part, &dest)
            .await
            .with_context(|| format!("finalizing {}", dest.display()))?;
        Ok(dest)
    }
}
