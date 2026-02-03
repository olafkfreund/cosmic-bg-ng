// SPDX-License-Identifier: MPL-2.0

//! Shared LRU image cache for memory-efficient wallpaper management.
//!
//! This module provides a thread-safe least-recently-used (LRU) cache for
//! decoded images, allowing multiple wallpapers to share the same image data
//! when using the same source file.

use image::DynamicImage;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    time::Instant,
};

/// A cached image entry with metadata
#[derive(Clone)]
struct CacheEntry {
    /// The cached image data
    image: Arc<DynamicImage>,
    /// When this entry was last accessed
    last_access: Instant,
    /// Size of the image in bytes (approximate)
    size_bytes: usize,
}

impl CacheEntry {
    fn new(image: DynamicImage) -> Self {
        let size_bytes = Self::estimate_size(&image);
        Self {
            image: Arc::new(image),
            last_access: Instant::now(),
            size_bytes,
        }
    }

    fn estimate_size(image: &DynamicImage) -> usize {
        let (width, height) = (image.width() as usize, image.height() as usize);
        // Assume 4 bytes per pixel (RGBA)
        width * height * 4
    }
}

/// Configuration for the image cache
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of images to cache
    pub max_entries: usize,
    /// Maximum total size in bytes (0 = unlimited)
    pub max_size_bytes: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 50,
            max_size_bytes: 512 * 1024 * 1024, // 512 MB default
        }
    }
}

/// Thread-safe LRU image cache
pub struct ImageCache {
    entries: RwLock<HashMap<PathBuf, CacheEntry>>,
    config: CacheConfig,
    stats: Mutex<CacheStats>,
}

/// Cache statistics for monitoring
#[derive(Debug, Default, Clone)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub current_entries: usize,
    pub current_size_bytes: usize,
}

impl ImageCache {
    /// Create a new image cache with default configuration
    pub fn new() -> Self {
        Self::with_config(CacheConfig::default())
    }

    /// Create a new image cache with custom configuration
    pub fn with_config(config: CacheConfig) -> Self {
        tracing::info!(
            max_entries = config.max_entries,
            max_size_mb = config.max_size_bytes / (1024 * 1024),
            "Image cache initialized"
        );

        Self {
            entries: RwLock::new(HashMap::new()),
            config,
            stats: Mutex::new(CacheStats::default()),
        }
    }

    /// Update cache statistics based on current entries
    fn update_stats(&self, entries: &HashMap<PathBuf, CacheEntry>) {
        let mut stats = self.stats.lock().unwrap();
        stats.current_entries = entries.len();
        stats.current_size_bytes = entries.values().map(|e| e.size_bytes).sum();
    }

    /// Get an image from the cache, returning None if not cached
    pub fn get(&self, path: &PathBuf) -> Option<Arc<DynamicImage>> {
        // Try read lock first for better concurrency
        {
            let entries = self.entries.read().unwrap();
            if let Some(entry) = entries.get(path) {
                let mut stats = self.stats.lock().unwrap();
                stats.hits += 1;
                return Some(Arc::clone(&entry.image));
            }
        }

        // Cache miss
        let mut stats = self.stats.lock().unwrap();
        stats.misses += 1;
        None
    }

    /// Insert an image into the cache
    pub fn insert(&self, path: PathBuf, image: DynamicImage) -> Arc<DynamicImage> {
        let entry = CacheEntry::new(image);
        let image_arc = Arc::clone(&entry.image);
        let entry_size = entry.size_bytes;

        {
            let mut entries = self.entries.write().unwrap();

            // Check if we need to evict entries
            self.evict_if_needed(&mut entries, entry_size);

            // Insert the new entry
            entries.insert(path.clone(), entry);

            // Update stats
            self.update_stats(&entries);
        }

        tracing::trace!(
            path = ?path,
            size_kb = entry_size / 1024,
            "Image cached"
        );

        image_arc
    }

    /// Get an image from cache or insert it using the provided loader function
    pub fn get_or_insert<F, E>(&self, path: &PathBuf, loader: F) -> Result<Arc<DynamicImage>, E>
    where
        F: FnOnce() -> Result<DynamicImage, E>,
    {
        // Check cache first
        if let Some(image) = self.get(path) {
            return Ok(image);
        }

        // Load and cache
        let image = loader()?;
        Ok(self.insert(path.clone(), image))
    }

    /// Remove an image from the cache
    pub fn remove(&self, path: &PathBuf) -> Option<Arc<DynamicImage>> {
        let mut entries = self.entries.write().unwrap();
        let removed = entries.remove(path).map(|e| e.image);

        if removed.is_some() {
            self.update_stats(&entries);
        }

        removed
    }

    /// Clear all cached images
    pub fn clear(&self) {
        let mut entries = self.entries.write().unwrap();
        let count = entries.len();
        entries.clear();

        self.update_stats(&entries);

        tracing::debug!(evicted = count, "Cache cleared");
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        self.stats.lock().unwrap().clone()
    }

    /// Check if the cache contains an image for the given path
    pub fn contains(&self, path: &PathBuf) -> bool {
        self.entries.read().unwrap().contains_key(path)
    }

    /// Get the number of cached images
    pub fn len(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        self.entries.read().unwrap().is_empty()
    }

    /// Evict entries if needed to make room for a new entry
    fn evict_if_needed(&self, entries: &mut HashMap<PathBuf, CacheEntry>, new_entry_size: usize) {
        let mut stats = self.stats.lock().unwrap();

        // Check entry count limit
        while entries.len() >= self.config.max_entries {
            if let Some(path) = self.find_lru_entry(entries) {
                entries.remove(&path);
                stats.evictions += 1;
            } else {
                break;
            }
        }

        // Check size limit
        if self.config.max_size_bytes > 0 {
            let current_size: usize = entries.values().map(|e| e.size_bytes).sum();
            let mut size_to_free = (current_size + new_entry_size).saturating_sub(self.config.max_size_bytes);

            while size_to_free > 0 {
                if let Some(path) = self.find_lru_entry(entries) {
                    if let Some(entry) = entries.remove(&path) {
                        size_to_free = size_to_free.saturating_sub(entry.size_bytes);
                        stats.evictions += 1;
                    }
                } else {
                    break;
                }
            }
        }
    }

    /// Find the least recently used entry
    fn find_lru_entry(&self, entries: &HashMap<PathBuf, CacheEntry>) -> Option<PathBuf> {
        entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_access)
            .map(|(path, _)| path.clone())
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ImageCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stats = self.stats();
        f.debug_struct("ImageCache")
            .field("entries", &stats.current_entries)
            .field("size_mb", &(stats.current_size_bytes / (1024 * 1024)))
            .field("hits", &stats.hits)
            .field("misses", &stats.misses)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    fn create_test_image(width: u32, height: u32) -> DynamicImage {
        let buffer = ImageBuffer::from_pixel(width, height, Rgba([255u8, 0, 0, 255]));
        DynamicImage::ImageRgba8(buffer)
    }

    #[test]
    fn test_cache_insert_and_get() {
        let cache = ImageCache::new();
        let path = PathBuf::from("/test/image.png");
        let image = create_test_image(100, 100);

        let cached = cache.insert(path.clone(), image);
        assert_eq!(cached.width(), 100);

        let retrieved = cache.get(&path);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().width(), 100);
    }

    #[test]
    fn test_cache_miss() {
        let cache = ImageCache::new();
        let path = PathBuf::from("/nonexistent/image.png");

        let result = cache.get(&path);
        assert!(result.is_none());

        let stats = cache.stats();
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_cache_eviction_by_count() {
        let config = CacheConfig {
            max_entries: 2,
            max_size_bytes: 0, // Unlimited size
        };
        let cache = ImageCache::with_config(config);

        // Insert 3 images, should evict the oldest
        for i in 0..3 {
            let path = PathBuf::from(format!("/test/image{}.png", i));
            cache.insert(path, create_test_image(10, 10));
        }

        assert_eq!(cache.len(), 2);

        let stats = cache.stats();
        assert_eq!(stats.evictions, 1);
    }

    #[test]
    fn test_cache_clear() {
        let cache = ImageCache::new();

        for i in 0..5 {
            let path = PathBuf::from(format!("/test/image{}.png", i));
            cache.insert(path, create_test_image(10, 10));
        }

        assert_eq!(cache.len(), 5);

        cache.clear();

        assert!(cache.is_empty());
    }

    #[test]
    fn test_get_or_insert() {
        let cache = ImageCache::new();
        let path = PathBuf::from("/test/image.png");

        // First call should load
        let result: Result<_, std::io::Error> = cache.get_or_insert(&path, || {
            Ok(create_test_image(50, 50))
        });
        assert!(result.is_ok());

        // Second call should hit cache
        let stats_before = cache.stats();
        let _ = cache.get_or_insert(&path, || -> Result<_, std::io::Error> {
            panic!("Loader should not be called on cache hit");
        });
        let stats_after = cache.stats();

        assert_eq!(stats_after.hits, stats_before.hits + 1);
    }
}
