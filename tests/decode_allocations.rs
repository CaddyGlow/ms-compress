use ms_compress::context::Decompressor;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static FAIL_AT: Cell<Option<usize>> = const { Cell::new(None) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}
struct CountingAllocator;
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        COUNTING.with(|enabled| {
            if enabled.get() {
                ALLOCATIONS.with(|count| count.set(count.get() + 1));
            }
        });
        if FAIL_AT.with(|limit| {
            limit
                .get()
                .is_some_and(|value| ALLOCATIONS.with(Cell::get) == value)
        }) {
            return std::ptr::null_mut();
        }
        // SAFETY: Delegate the valid layout unchanged to the system allocator.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: Pointer and layout originate from this system allocator.
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        COUNTING.with(|enabled| {
            if enabled.get() {
                ALLOCATIONS.with(|count| count.set(count.get() + 1));
            }
        });
        // SAFETY: Forward the caller's valid allocation and size unchanged.
        unsafe { System.realloc(pointer, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn reusable_lzms_success_and_failures_allocate_nothing() {
    let mut state = 0x12345678u32;
    let plain: Vec<u8> = (0..8192)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    let packed = ms_compress::lzms::encode::compress_lzms(&plain, 32768)
        .unwrap()
        .unwrap();
    let short_plain = [0u8; 1];
    // Zero range/code words and zero literal bits encode a single zero byte.
    let short_packed = [0u8; 4];
    let mut short_output = [0u8; 1];
    let medium_plain = [17u8; 257];
    let medium_packed = ms_compress::lzms::encode::compress_lzms(&medium_plain, 32768)
        .unwrap()
        .unwrap();
    let mut medium_output = [0u8; 257];
    let mut output = vec![0; plain.len()];
    let mut decoder = Decompressor::new(3, 32768).unwrap();
    ALLOCATIONS.with(|count| count.set(0));
    COUNTING.with(|enabled| enabled.set(true));
    let mut valid = true;
    for _ in 0..3 {
        valid &= decoder.decompress(&short_packed, &mut short_output).is_ok()
            && short_output == short_plain;
        valid &= decoder
            .decompress(&medium_packed, &mut medium_output)
            .is_ok()
            && medium_output == medium_plain;
        valid &= decoder.decompress(&packed, &mut output).is_ok() && output == plain;
        valid &= decoder.decompress(&[0xff; 4], &mut output).is_err();
        valid &= decoder.decompress(&[0; 3], &mut output).is_err();
        valid &= decoder.decompress(&packed, &mut output).is_ok() && output == plain;
    }
    COUNTING.with(|enabled| enabled.set(false));
    assert!(valid);
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}

#[test]
fn every_lzms_workspace_allocation_failure_is_recoverable() {
    ALLOCATIONS.with(|count| count.set(0));
    COUNTING.with(|enabled| enabled.set(true));
    let initial = Decompressor::new(3, 32768);
    COUNTING.with(|enabled| enabled.set(false));
    let count = ALLOCATIONS.with(Cell::get);
    assert!(initial.is_ok() && count > 0);
    for index in 1..=count {
        ALLOCATIONS.with(|count| count.set(0));
        FAIL_AT.with(|limit| limit.set(Some(index)));
        COUNTING.with(|enabled| enabled.set(true));
        let result = Decompressor::new(3, 32768);
        COUNTING.with(|enabled| enabled.set(false));
        FAIL_AT.with(|limit| limit.set(None));
        assert!(
            matches!(result, Err(ms_compress::context::ContextError::OutOfMemory)),
            "allocation {index}"
        );
    }
}
