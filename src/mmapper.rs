use std::{
    alloc::{AllocError, Layout},
    cmp::min,
    collections::HashMap,
    ptr::{copy_nonoverlapping, NonNull},
    sync::{Mutex, MutexGuard},
};

use crate::mmap::{MMap, PageSize};
use crate::HugeAllocatorStats;

/// A collection of tracked memory mapped segments
pub struct MMapper {
    /// Threshold percentage to try and use huge pages.
    /// For example a threshold percentage of 50 will try and allocate a 2mb page for allocations >= 1mb
    threshold_pct: usize,
    ptr_map: Mutex<HashMap<usize, MMap>>,
    stats: Mutex<MMapperStats>,
}

impl MMapper {
    /// Create a new memory mappings container
    pub fn new(threshold_pct: usize) -> Self {

        Self {
            threshold_pct,
            ptr_map: Mutex::new(HashMap::new()),
            stats: Mutex::new(MMapperStats::default()),
        }
    }

    /// Allocates an anonymous memory mapped segment
    pub fn alloc(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let size = layout.size();

        // Calculate page size for this allocation
        let page_size = self.target_page_size(size);

        // Create the anon memory map with the desired page size
        let mmap = match MMap::new(layout, &page_size) {
            Ok(m) => m,
            _ => {
                // Failed - try default page size
                if page_size == PageSize::SizeDefault {
                    Err(AllocError)?
                } else {
                    match MMap::new(layout, &PageSize::SizeDefault) {
                        Ok(m) => m,
                        _ => Err(AllocError)?
                    }
                }
            }
        };

        if mmap.page_size() == PageSize::SizeDefault {
            // Log missed allocation
            self.add_missed(size)?;
        }

        // Get raw pointer
        let ptr = mmap.fat_ptr();

        // Insert in to hash map
        self.map_add(mmap)?;

        Ok(ptr)
    }

    /// Deallocates an anonymous memory mapped segment
    pub fn dealloc(&self, ptr: NonNull<u8>) -> Result<(), AllocError> {
        // Remove from the map
        self.map_remove(ptr)?;

        Ok(())
    }

    /// Reallocates an anonymous memory mapped segment
    pub fn realloc(&self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let old_size = old_layout.size();
        let new_size = new_layout.size();

        // Remove existing map entry
        let mmap = self.map_remove(ptr)?;

        let mut mmap = match mmap {
            Some(m) => m,
            _ => Err(AllocError)?,
        };

        let was_default = mmap.page_size() == PageSize::SizeDefault;

        if mmap.page_size() == self.target_page_size(new_size) {
            // Try and do a reallocate
            if mmap.remap(new_layout) {
                // Get raw pointer
                let ptr = mmap.fat_ptr();

                if was_default {
                    // Was default
                    if new_size > old_size {
                        // Add extra space as missed
                        self.add_missed(new_size - old_size)?;
                    }
                } else if mmap.page_size() == PageSize::SizeDefault {
                    // Was huge and is now not
                    self.add_missed(new_size)?;
                }

                // Insert it back in to the hash map
                self.map_add(mmap)?;

                return Ok(ptr);
            } else {
                // Failed to remap
                let mut stats = self.lock_stats()?;

                stats.remaps_failed += 1;
            }
        }

        // Allocate new segment
        let new_ptr = self.alloc(new_layout)?;

        // Copy data from old segment to new
        unsafe {
            copy_nonoverlapping(mmap.as_ptr(), new_ptr.as_mut_ptr(), min(old_size, new_size));
        }

        Ok(new_ptr)
    }

    /// Returns the target page size for a given allocation size (or 0 for default)
    fn target_page_size(&self, size: usize) -> PageSize {
        // Test for 2mb page size
        if (size * 100) / (2 * 1024 * 1024) >= self.threshold_pct {
            return PageSize::Size2m;
        }

        PageSize::SizeDefault
    }
    
    /// Returns statistics for the mapper
    pub(crate) fn stats(&self) -> Result<HugeAllocatorStats, AllocError> {
        let mut out_stats = HugeAllocatorStats::default();

        // Lock the ptr_map
        let ptr_map = self.lock_map()?;

        for mmap in ptr_map.values() {
            out_stats.alloc += mmap.size();
            out_stats.mapped += mmap.alloc_size();
            out_stats.segments += 1;

            if mmap.page_size() == PageSize::SizeDefault {
                out_stats.default_alloc += mmap.size();
                out_stats.default_mapped += mmap.alloc_size();
                out_stats.default_segments += 1;
            } else {
                out_stats.huge_alloc += mmap.size();
                out_stats.huge_mapped += mmap.alloc_size();
                out_stats.huge_segments += 1;
            }
        }

        let stats = self.lock_stats()?;

        out_stats.missed_allocs = stats.missed_allocs;
        out_stats.missed_mb = stats.missed_mb as f64 + (stats.missed_bytes as f64 / (1024 * 1024) as f64);
        out_stats.remaps_failed = stats.remaps_failed;

        drop(stats);

        if out_stats.mapped == 0 {
            out_stats.efficiency = 100;
        } else {
            out_stats.efficiency = (out_stats.alloc * 100) / out_stats.mapped;
        }

        Ok(out_stats)
    }

    /// Removes an entry from the pointer map
    fn map_remove(&self, ptr: NonNull<u8>) -> Result<Option<MMap>, AllocError> {
        // Lock the ptr_map
        let mut ptr_map = self.lock_map()?;

        // Remove map entry
        Ok(ptr_map.remove(&(ptr.as_ptr() as usize)))
    }

    /// Adds an entry from the pointer map
    fn map_add(&self, mmap: MMap) -> Result<(), AllocError> {
        // Lock the ptr_map
        let mut ptr_map = self.lock_map()?;

        // Add map entry
        if ptr_map.insert(mmap.as_ptr() as usize, mmap).is_some() {
            Err(AllocError)?;
        }

        Ok(())
    }

    /// Locks the ptr_map for removal
    fn lock_map(&self) -> Result<MutexGuard<HashMap<usize, MMap>>, AllocError> {
        // Lock the ptr_map
        match self.ptr_map.lock() {
            Ok(ptr_map) => Ok(ptr_map),
            _ => Err(AllocError),
        }
    }

    /// Locks statistics
    fn lock_stats(&self) -> Result<MutexGuard<MMapperStats>, AllocError> {
        // Lock stats
        match self.stats.lock() {
            Ok(stats) => Ok(stats),
            _ => Err(AllocError),
        }
    }

    /// Add statistics about missed huge allocations
    fn add_missed(&self, bytes: usize) -> Result<(), AllocError> {
        let mut stats = self.lock_stats()?;

        stats.missed_allocs += 1;

        stats.missed_bytes += bytes;

        if stats.missed_bytes > (1024 * 1024) {
            let mb = stats.missed_bytes / (1024 * 1024);
            stats.missed_bytes -= mb * (1024 * 1024);
            stats.missed_mb += mb;
        }

        Ok(())
    }
}

#[derive(Default)]
struct MMapperStats {
    missed_allocs: usize,
    missed_bytes: usize,
    missed_mb: usize,
    remaps_failed: usize,
}
