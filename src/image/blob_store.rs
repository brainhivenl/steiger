use std::path::PathBuf;

pub struct BlobStore {
    root: PathBuf,
}

fn split_algo_hash(digest: &str) -> (&str, &str) {
    digest.split_once(':').unwrap_or_default()
}

impl BlobStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub async fn read_blob(&self, digest: &str) -> Result<Vec<u8>, std::io::Error> {
        let (alg, hash) = split_algo_hash(digest);
        tokio::fs::read(self.root.join("blobs").join(alg).join(hash)).await
    }
}
