//! Tiny file cache for usage responses (freshness = file mtime), so repeated
//! `tokache status` calls don't hammer the endpoint.

use std::path::PathBuf;
use std::time::Duration;

use crate::Result;

pub const DEFAULT_TTL: Duration = Duration::from_secs(60);

pub struct Cache {
    dir: PathBuf,
    ttl: Duration,
}

impl Cache {
    pub fn new(data_dir: &std::path::Path, ttl: Duration) -> Self {
        Self {
            dir: data_dir.join("cache"),
            ttl,
        }
    }

    /// Cached body for `key`, if fresher than the TTL.
    pub fn get(&self, key: &str) -> Option<String> {
        let path = self.path(key);
        let age = std::fs::metadata(&path)
            .ok()?
            .modified()
            .ok()?
            .elapsed()
            .ok()?;
        if age <= self.ttl {
            std::fs::read_to_string(path).ok()
        } else {
            None
        }
    }

    pub fn put(&self, key: &str, body: &str) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.path(key), body)?;
        Ok(())
    }

    fn path(&self, key: &str) -> PathBuf {
        // Keys are validated account names or "current" — safe as file names.
        self.dir.join(format!("usage-{key}.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_get_within_ttl() {
        let dir = std::env::temp_dir().join(format!("tokache-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = Cache::new(&dir, Duration::from_secs(60));
        assert!(cache.get("k").is_none());
        cache.put("k", "{\"a\":1}").unwrap();
        assert_eq!(cache.get("k").as_deref(), Some("{\"a\":1}"));

        let zero = Cache::new(&dir, Duration::ZERO);
        assert!(zero.get("k").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
