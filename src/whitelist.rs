use crate::settings::DynamicWhitelistConfig;
use log::{debug, error, info};
use std::collections::HashMap;
use std::process::Command;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
struct CacheEntry {
    allowed: bool,
    expires_at: Instant,
}

pub struct DynamicWhitelist {
    config: DynamicWhitelistConfig,
    cache: RwLock<HashMap<String, CacheEntry>>,
}

impl DynamicWhitelist {
    pub fn new(config: DynamicWhitelistConfig) -> Self {
        Self {
            config,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Check if a pubkey is allowed access
    pub async fn is_allowed(&self, pubkey: &str) -> bool {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(pubkey) {
                if entry.expires_at > Instant::now() {
                    debug!("Cache hit for pubkey {}: allowed = {}", pubkey, entry.allowed);
                    return entry.allowed;
                }
            }
        }

        // Call external program
        let allowed = self.call_external_program(pubkey).await;
        
        // Cache the result if it's positive (allowed)
        if allowed {
            let expires_at = Instant::now() + Duration::from_secs(self.config.cache_duration_seconds);
            let entry = CacheEntry { allowed, expires_at };
            
            let mut cache = self.cache.write().await;
            cache.insert(pubkey.to_string(), entry);
            
            info!("Cached positive result for pubkey {} for {} seconds", 
                  pubkey, self.config.cache_duration_seconds);
        }

        allowed
    }

    async fn call_external_program(&self, pubkey: &str) -> bool {
        let program_path = self.config.user_exit_program.clone();
        let pubkey = pubkey.to_string();
        let program_path_display = program_path.display().to_string();
        let pubkey_clone = pubkey.clone();
        
        debug!("Calling external program {} with pubkey {}", 
               program_path_display, pubkey);

        // Run the external program in a blocking task
        let result = tokio::task::spawn_blocking(move || {
            Command::new(program_path)
                .arg(&pubkey)
                .output()
        }).await;

        match result {
            Ok(Ok(output)) => {
                let exit_code = output.status.code().unwrap_or(-1);
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

                info!("External program output - exit_code: {}, stdout: '{}', stderr: '{}'", 
                       exit_code, stdout, stderr);

                // Consider it allowed if exit code is 0
                let allowed = exit_code == 0;
                
                if allowed {
                    info!("External program allowed access for pubkey: {}", pubkey_clone);
                } else {
                    info!("External program denied access for pubkey: {} (exit code: {})", 
                          pubkey_clone, exit_code);
                }

                allowed
            }
            Ok(Err(e)) => {
                error!("Failed to execute external program {}: {}", program_path_display, e);
                false
            }
            Err(e) => {
                error!("Failed to spawn task for external program: {}", e);
                false
            }
        }
    }

    /// Clear the cache (useful for testing or manual cache invalidation)
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        info!("Whitelist cache cleared");
    }

    /// Get cache statistics
    pub async fn cache_stats(&self) -> (usize, usize) {
        let cache = self.cache.read().await;
        let total_entries = cache.len();
        let expired_entries = cache.values()
            .filter(|entry| entry.expires_at <= Instant::now())
            .count();
        (total_entries, expired_entries)
    }

    /// Clean up expired cache entries
    pub async fn cleanup_expired(&self) {
        let mut cache = self.cache.write().await;
        let before = cache.len();
        cache.retain(|_, entry| entry.expires_at > Instant::now());
        let after = cache.len();
        
        if before != after {
            info!("Cleaned up {} expired cache entries", before - after);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_dynamic_whitelist_cache() {
        let temp_dir = tempdir().unwrap();
        let script_path = temp_dir.path().join("test_script.sh");
        
        // Create a simple test script that always returns success
        std::fs::write(&script_path, "#!/bin/bash\necho 'allowed'\nexit 0").unwrap();
        
        // Make it executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();
        }

        let config = DynamicWhitelistConfig {
            user_exit_program: script_path,
            cache_duration_seconds: 1, // 1 second for testing
        };

        let whitelist = DynamicWhitelist::new(config);
        let pubkey = "test_pubkey_123";

        // First call should hit the external program
        let result1 = whitelist.is_allowed(pubkey).await;
        assert!(result1);

        // Second call should hit the cache
        let result2 = whitelist.is_allowed(pubkey).await;
        assert!(result2);

        // Wait for cache to expire
        tokio::time::sleep(Duration::from_millis(1100)).await;

        // Third call should hit the external program again
        let result3 = whitelist.is_allowed(pubkey).await;
        assert!(result3);
    }

    #[tokio::test]
    async fn test_dynamic_whitelist_denial() {
        let temp_dir = tempdir().unwrap();
        let script_path = temp_dir.path().join("test_script.sh");
        
        // Create a test script that always returns failure
        std::fs::write(&script_path, "#!/bin/bash\necho 'denied'\nexit 1").unwrap();
        
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();
        }

        let config = DynamicWhitelistConfig {
            user_exit_program: script_path,
            cache_duration_seconds: 3600,
        };

        let whitelist = DynamicWhitelist::new(config);
        let pubkey = "test_pubkey_456";

        // Should be denied and not cached
        let result = whitelist.is_allowed(pubkey).await;
        assert!(!result);

        // Check that it's not cached (should call external program again)
        let result2 = whitelist.is_allowed(pubkey).await;
        assert!(!result2);
    }
} 