#![feature(allocator_api)]
#![feature(slice_ptr_get)]

#![warn(missing_docs)]

//! A memory allocator which tries to use huge pages for big allocations

mod mmap;
mod mmapper;

use std::alloc::{AllocError, Allocator, Layout};
use std::ptr::NonNull;

use mmapper::MMapper;

/// Huge page allocator
pub struct HugeAllocator {
    mapper: MMapper,
}

impl HugeAllocator {
    /// Creates a new huge page allocator with a given threshold percentage.
    /// As an example a threshold percentage of 50 will try and allocate a 2mb page for allocations >= 1mb
    ///
    /// ```rust
    /// #![feature(allocator_api)]
    /// use huge_allocator::HugeAllocator;
    ///
    /// let allocator = HugeAllocator::new(50);
    ///
    /// let vec1: Vec<u8, _> = Vec::with_capacity_in(2 * 1024 * 1024, &allocator);
    /// let vec2: Vec<u8, &HugeAllocator> = Vec::with_capacity_in(2 * 1024 * 1024, &allocator);
    /// #
    /// # let stats = allocator.stats().unwrap();
    /// # assert_eq!(2, stats.segments, "Segments allocated should be 2");
    /// ```
    pub fn new(threshold_pct: usize) -> Self {
        Self {
            mapper: MMapper::new(threshold_pct),
        }
    }

    /// Returns allocator statistics
    /// ```rust
    /// #![feature(allocator_api)]
    /// use huge_allocator::HugeAllocator;
    /// let allocator = HugeAllocator::new(50);
    ///
    /// let vec: Vec<u8, _> = Vec::with_capacity_in(2 * 1024 * 1024, &allocator);
    ///
    /// let stats = allocator.stats().unwrap();
    ///
    /// assert_eq!(1, stats.segments, "Segments allocated should be 1");
    /// ```
    pub fn stats(&self) -> Result<HugeAllocatorStats, AllocError> {
        self.mapper.stats()
    }
}

unsafe impl Allocator for HugeAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.mapper.alloc(layout)
    }

    unsafe fn deallocate(&self, ptr: std::ptr::NonNull<u8>, _layout: Layout) {
        match self.mapper.dealloc(ptr) {
            Ok(p) => p,
            Err(e) => panic!("HugeAllocator::deallocate: Failed to dealloc ({})", e),
        }
    }

    fn allocate_zeroed(&self, layout: Layout) -> Result<std::ptr::NonNull<[u8]>, std::alloc::AllocError> {
        // Mapped pages are zeroed by default so just revert to allocate()
        self.allocate(layout)
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        debug_assert!(
            new_layout.size() >= old_layout.size(),
            "`new_layout.size()` must be greater than or equal to `old_layout.size()`"
        );

        self.mapper.realloc(ptr, old_layout, new_layout)
    }

    unsafe fn grow_zeroed(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        // Mapped pages are zeroed by default so just revert to grow()
        self.grow(ptr, old_layout, new_layout)
    }

    unsafe fn shrink(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        debug_assert!(
            new_layout.size() <= old_layout.size(),
            "`new_layout.size()` must be smaller than or equal to `old_layout.size()`"
        );

        self.mapper.realloc(ptr, old_layout, new_layout)
    }
}

/// Allocator performance statistics
#[derive(Debug, Default)]
pub struct HugeAllocatorStats {
    /// Total amount of memory allocated in bytes
    pub alloc: usize,
    /// Total amount of memory mapped in bytes
    pub mapped: usize,
    /// Total number of segments mapped
    pub segments: usize,

    /// Amount of memory allocated in default page size pages in bytes
    pub default_alloc: usize,
    /// Amount of memory mapped in default page size pages in bytes
    pub default_mapped: usize,
    /// Number of default page size segments mapped
    pub default_segments: usize,

    /// Amount of memory allocated in huge pages in bytes
    pub huge_alloc: usize,
    /// Amount of memory mapped in huge pages in bytes
    pub huge_mapped: usize,
    /// Number of huge page segments mapped
    pub huge_segments: usize,

    /// Number of allocations missed due to lack of huge pages
    pub missed_allocs: usize,
    /// Allocations missed due to lack of huge pages in total megabytes
    pub missed_mb: f64,
    /// Number of failed remaps
    pub remaps_failed: usize,
    /// Percentage of mapped memory used by allocations
    pub efficiency: usize,
}

#[cfg(test)]
mod tests;
