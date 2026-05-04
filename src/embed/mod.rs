use std::path::PathBuf;
use std::sync::Mutex;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

/// Trait so tests can inject a cheap fake embedder without downloading the model.
pub trait EmbedText: Send + Sync + 'static {
    fn embed_one(&self, text: &str) -> anyhow::Result<Vec<f32>>;
}

pub struct Embedder {
    model: Mutex<TextEmbedding>,
}

impl Embedder {
    pub fn try_new(cache_dir: PathBuf) -> anyhow::Result<Self> {
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGESmallENV15)
                .with_show_download_progress(true)
                .with_cache_dir(cache_dir),
        )?;
        Ok(Self {
            model: Mutex::new(model),
        })
    }
}

impl EmbedText for Embedder {
    fn embed_one(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut model = self
            .model
            .lock()
            .map_err(|e| anyhow::anyhow!("embedder lock: {e}"))?;
        let mut results = model.embed(vec![text], None)?;
        anyhow::ensure!(!results.is_empty(), "model returned no embeddings");
        Ok(results.remove(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Requires the ONNX model to be downloaded — skipped in normal CI.
    #[test]
    #[ignore]
    fn embedder_produces_384_dim_vector() {
        let embedder = Embedder::try_new(std::env::temp_dir()).expect("model load failed");
        let vec = embedder.embed_one("hello world").unwrap();
        assert_eq!(vec.len(), 384);
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.01,
            "BGE embeddings should be unit-normalised"
        );
    }
}
