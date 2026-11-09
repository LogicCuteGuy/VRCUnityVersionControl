use anyhow::{bail, Context, Result};
use chibivcs_proto::sha256_hex;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).context("create blob root")?;
        fs::create_dir_all(root.join("tmp")).context("create blob tmp")?;
        Ok(Self { root })
    }

    pub fn validate_hash(hash: &str) -> Result<String> {
        let lower = hash.trim().to_ascii_lowercase();
        if lower.len() != 64 || !lower.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("invalid sha256 hash: {}", hash);
        }
        Ok(lower)
    }

    fn blob_path(&self, hash: &str) -> PathBuf {
        let prefix = &hash[0..2];
        self.root.join(prefix).join(hash)
    }

    pub fn has(&self, hash: &str) -> bool {
        let Ok(clean) = Self::validate_hash(hash) else {
            return false;
        };
        self.blob_path(&clean).is_file()
    }

    pub fn put(&self, expected_hash: &str, data: &[u8]) -> Result<()> {
        let clean = Self::validate_hash(expected_hash)?;
        let actual = sha256_hex(data);
        if actual != clean {
            bail!(
                "blob sha256 mismatch: expected {}, got {}",
                clean,
                actual
            );
        }

        let target = self.blob_path(&clean);
        if target.is_file() {
            return Ok(());
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        let rand_id: u64 = rand::random();
        let tmp_path = self.root.join("tmp").join(format!("{}.{}.tmp", clean, rand_id));
        fs::write(&tmp_path, data).context("write temp blob")?;
        fs::rename(&tmp_path, &target).or_else(|e| {
            let _ = fs::remove_file(&tmp_path);
            if target.is_file() {
                Ok(())
            } else {
                Err(e)
            }
        })?;
        Ok(())
    }

    pub fn get(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        let clean = Self::validate_hash(hash)?;
        let path = self.blob_path(&clean);
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = fs::read(&path).context("read blob")?;
        Ok(Some(bytes))
    }
}
