use super::*;

fn mb(mb: usize) -> usize {
    mb * 1024 * 1024
}

fn check_stats(allocator: &HugeAllocator, desc: &str, expected_segs: usize, expected_mapped: usize) -> HugeAllocatorStats {
    let stats = allocator.stats().unwrap();

    println!("{}: {:?}", desc, stats);

    assert_eq!(expected_segs, stats.segments, "{} segments", desc);

    let avail_bytes = if let Ok(env) = std::env::var("TEST_NR_PAGES") {
        let avail_pages = env.parse::<usize>().expect("TEST_NR_PAGES not numeric");
        avail_pages * mb(2)
    } else {
        0
    };

    if avail_bytes >= mb(6) {
        // Enough huge pages to satisfy
        assert_eq!(expected_mapped, stats.mapped, "{} mapped", desc);
        if expected_mapped >= mb(2) {
            assert_eq!(expected_mapped, stats.huge_mapped, "{} huge mapped", desc);
            assert_eq!(0, stats.default_mapped, "{} default mapped", desc);
        } else {
            assert_eq!(0, stats.huge_mapped, "{} huge mapped", desc);
            assert_eq!(expected_mapped, stats.default_mapped, "{} default mapped", desc);
        }
    } else if stats.huge_segments > 0 {
        // Allocation is huge - check size
        assert_eq!(expected_mapped, stats.mapped, "{} mapped", desc);
        assert_eq!(expected_mapped, stats.huge_mapped, "{} huge mapped", desc);
        assert_eq!(0, stats.default_mapped, "{} default mapped", desc);
    } else {
        // Allocation is in default pages
        assert!(stats.mapped >= stats.alloc, "{} mapped >= alloc", desc);
        assert_eq!(0, stats.huge_mapped, "{} huge mapped", desc);
        assert!(stats.default_mapped >= stats.alloc, "{} default mapped >= alloc", desc);
    }

    assert_eq!(stats.default_segments + stats.huge_segments, stats.segments, "{} segment sum", desc);
    assert_eq!(stats.default_mapped + stats.huge_mapped, stats.mapped, "{} mapped sum", desc);
    assert_eq!(stats.default_alloc + stats.huge_alloc, stats.alloc, "{} alloc sum", desc);

    stats
}

fn check_stats_eq(allocator: &HugeAllocator, desc: &str, expected_alloc: usize, expected_segs: usize, expected_mapped: usize) {
    let stats = check_stats(allocator, desc, expected_segs, expected_mapped);
    assert_eq!(expected_alloc, stats.alloc, "{} alloc", desc);
}

fn check_stats_gt(allocator: &HugeAllocator, desc: &str, expected_alloc: usize, expected_segs: usize, expected_mapped: usize) {
    let stats = check_stats(allocator, desc, expected_segs, expected_mapped);
    assert!(stats.alloc > expected_alloc, "{} alloc", desc);
}

fn check_stats_ge(allocator: &HugeAllocator, desc: &str, expected_alloc: usize, expected_segs: usize, expected_mapped: usize) {
    let stats = check_stats(allocator, desc, expected_segs, expected_mapped);
    assert!(stats.alloc >= expected_alloc, "{} alloc", desc);
}

fn check_stats_lt(allocator: &HugeAllocator, desc: &str, expected_alloc: usize, expected_segs: usize, expected_mapped: usize) {
    let stats = check_stats(allocator, desc, expected_segs, expected_mapped);
    assert!(stats.alloc < expected_alloc, "{} alloc", desc);
}

#[test]
fn huge_alloc() {
    let allocator = HugeAllocator::new(50);
    let mut vec = Vec::new_in(&allocator);

    // 512 * 1024 * 8 = 4mb
    let items = 512 * 1024;

    let vec_mb = |items| {
        let bytes = items * 8;

        if bytes % mb(1) == 0 {
            Some(bytes / mb(1))
        } else {
            None
        }
    };

    for i in 0..items {
        let on_mb = vec_mb(i);

        if let Some(cur) = on_mb {
            match cur {
                0 => check_stats_eq(&allocator, "initial", 0, 0, 0), // No allocation
                1 => check_stats_ge(&allocator, ">= 1mb", mb(cur), 1, mb(2)), // 2mb in huge pages
                2 => check_stats_ge(&allocator, ">= 2mb", mb(cur), 1, mb(2)), // 2mb in huge pages
                3 => check_stats_ge(&allocator, ">= 3mb", mb(cur), 1, mb(4)), // 4mb in huge pages
                _ => panic!("mb boundary not handled"),
            }
        }

        vec.push(i);

        if let Some(cur) = on_mb {
            match cur {
                0 => check_stats_gt(&allocator, "> 0", 0, 1, 4096),  // 4k in default pages
                1 => check_stats_gt(&allocator, "> 1mb", mb(cur), 1, mb(2)), // 2mb in huge pages
                2 => check_stats_gt(&allocator, "> 2mb", mb(cur), 1, mb(4)), // 4mb in huge pages
                3 => check_stats_gt(&allocator, "> 3mb", mb(cur), 1, mb(4)), // 4mb in huge pages
                _ => panic!("mb boundary not handled"),
            }
        }
    }

    assert_eq!(vec.len(), items, "vector entries incorrect");

    println!("Popping {} items ({} bytes)", items, items * 8);

    for i in (0..items).rev() {
        vec.pop().unwrap();

        assert_eq!(i, vec.len());

        if let Some(cur) = vec_mb(i + 1) {
            vec.shrink_to_fit();

            assert_eq!(i, vec.capacity());

            match cur {
                0 => check_stats_eq(&allocator, "0", 0, 0, 0), // No allocation
                1 => check_stats_lt(&allocator, "< 1mb", mb(cur), 1, mb(1)), // < 1mb in default pages
                2 => check_stats_lt(&allocator, "< 2mb", mb(cur), 1, mb(2)), // < 2mb in huge or default pages
                3 => check_stats_lt(&allocator, "< 3mb", mb(cur), 1, mb(4)), // < 4mb in huge or default pages
                4 => check_stats_lt(&allocator, "< 4mb", mb(cur), 1, mb(4)), // < 4mb in huge or default pages
                _ => panic!("mb boundary not handled"),
            }
        }
    }
}
