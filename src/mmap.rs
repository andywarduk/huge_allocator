use std::alloc::Layout;
use std::ffi::c_void;
use std::ptr::{null_mut, slice_from_raw_parts_mut, NonNull};

use lazy_static::lazy_static;

use nix::{
    sys::mman::{mmap, mremap, munmap, MRemapFlags, MapFlags, ProtFlags},
    unistd::{sysconf, SysconfVar},
};

lazy_static! {
    /// The default page size for the platform
    static ref DEFAULT_PAGE_SIZE: usize = {
        match sysconf(SysconfVar::PAGE_SIZE) {
            Ok(val) => match val {
                Some(val) => val as usize,
                None => panic!("sysconf PAGE_SIZE returned no value")
            }
            Err(e) => panic!("sysconf PAGE_SIZE failed ({})", e)
        }
    };
}

/// Available page sizes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageSize {
    SizeDefault = 0,
    Size2m = 2 * 1024 * 1024
}

impl PageSize {
    pub fn bytes(&self) -> usize {
        match self {
            PageSize::SizeDefault => *DEFAULT_PAGE_SIZE,
            _ => *self as usize,
        }
    }

    fn map_flags(&self) -> MapFlags {
        match self {
            PageSize::SizeDefault => MapFlags::empty(),
            PageSize::Size2m => MapFlags::MAP_HUGETLB | MapFlags::MAP_HUGE_2MB,
        }
    }
}

/// Descriptor for anonymous memory mapped segments
#[derive(Debug)]
pub struct MMap {
    /// Pointer to memory mapped section
    ptr: usize,
    /// Requested layout
    layout: Layout,
    /// Allocation size
    alloc_size: usize,
    /// Page size
    page_size: PageSize,
}

impl MMap {
    /// Creates a new anonymous memory mapped segment. A huge page allocation is tried initially if the
    /// size is above the threshold percentage. If that fails a default page size allocation is tried.
    pub fn new(layout: Layout, page_size: &PageSize) -> nix::Result<MMap> {
        Self::map(layout, page_size)
    }

    /// Returns the fat pointer
    pub fn fat_ptr(&self) -> NonNull<[u8]> {
        NonNull::new(self.as_fat_ptr()).unwrap()
    }
    
    /// Returns the raw pointer to the memory mapped segment
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr as *mut u8
    }

    /// Returns the raw pointer to the memory mapped segment
    pub fn as_fat_ptr(&self) -> *mut [u8] {
        slice_from_raw_parts_mut(self.as_ptr(), self.alloc_size)
    }
    
    /// Returns the allocation size of the segment
    pub fn size(&self) -> usize {
        self.layout.size()
    }

    /// Returns the total mapped size of the segment
    pub fn alloc_size(&self) -> usize {
        self.alloc_size
    }

    /// Returns the mapped page size
    pub fn page_size(&self) -> PageSize {
        self.page_size
    }

    /// Remaps a memory section
    pub fn remap(&mut self, new_layout: Layout) -> bool {
        let new_size = new_layout.size();
        let new_alloc_size = Self::calc_alloc_size(new_size, &self.page_size);

        let ok = if self.alloc_size != new_alloc_size {
            // Try and remap
            match unsafe {
                mremap(
                    self.ptr as *mut c_void,
                    self.alloc_size,
                    new_alloc_size,
                    MRemapFlags::MREMAP_MAYMOVE,
                    None,
                )
            } {
                Ok(ptr) => {
                    // Success
                    self.ptr = ptr as usize;
                    self.alloc_size = new_alloc_size;

                    true
                }
                Err(_) => {
                    // Failed
                    false
                }
            }
        } else {
            true
        };

        if ok {
            self.layout = new_layout;
        }

        ok
    }

    /// Tries to map an anonymous read write segment with given page size.
    /// Reverts to default page size on failure
    fn map(layout: Layout, page_size: &PageSize) -> nix::Result<MMap> {
        // Calculate mmap flags for this page size
        let map_flags = page_size.map_flags();

        // Calculate size of mapped area
        let alloc_size = Self::calc_alloc_size(layout.size(), page_size);

        // Try and map the memory
        let ptr = unsafe {
            mmap(
                null_mut::<c_void>(),
                alloc_size,
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
                MapFlags::MAP_ANON | MapFlags::MAP_PRIVATE | map_flags,
                0,
                0,
            )
        }?;

        Ok(MMap {
            ptr: ptr as usize,
            layout,
            alloc_size,
            page_size: *page_size,
        })
    }

    /// Calculates the allocation size (whole pages) required for the size required
    fn calc_alloc_size(size: usize, page_size: &PageSize) -> usize {
        if size > 0 {
            let page_bytes = page_size.bytes();

            (((size - 1) / page_bytes) + 1) * page_bytes
        } else {
            0
        }
    }
}

impl Drop for MMap {
    /// Unmaps the anonymous memory mapped segment on drop
    fn drop(&mut self) {
        let size = self.alloc_size();

        if unsafe { munmap(self.ptr as *mut c_void, size) }.is_err() {
            panic!("MMap::drop: failed to unmap ({:?})", self.layout);
        }
    }
}
