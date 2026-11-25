//! Separate profiling builds count requested Rust heap storage, not RSS.
// [spec:pgorm:req:turso.performance]

#[cfg(feature = "alloc-profile")]
mod counted {
    use serde_json::{Value, json};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicI64, AtomicU64, Ordering::Relaxed};

    pub struct Allocator;
    static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
    static REALLOCATIONS: AtomicU64 = AtomicU64::new(0);
    static ALLOCATED: AtomicU64 = AtomicU64::new(0);
    static DEALLOCATED: AtomicU64 = AtomicU64::new(0);
    static LIVE: AtomicI64 = AtomicI64::new(0);
    static PEAK: AtomicI64 = AtomicI64::new(0);

    fn allocated(size: usize) {
        ALLOCATED.fetch_add(size as u64, Relaxed);
        let live = LIVE.fetch_add(size as i64, Relaxed) + size as i64;
        PEAK.fetch_max(live, Relaxed);
    }

    fn deallocated(size: usize) {
        DEALLOCATED.fetch_add(size as u64, Relaxed);
        LIVE.fetch_sub(size as i64, Relaxed);
    }

    // SAFETY: every operation forwards the original pointer/layout unchanged
    // to System. Counters allocate nothing and inspect no allocation contents.
    unsafe impl GlobalAlloc for Allocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let ptr = unsafe { System.alloc(layout) };
            if !ptr.is_null() {
                ALLOCATIONS.fetch_add(1, Relaxed);
                allocated(layout.size());
            }
            ptr
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let ptr = unsafe { System.alloc_zeroed(layout) };
            if !ptr.is_null() {
                ALLOCATIONS.fetch_add(1, Relaxed);
                allocated(layout.size());
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) };
            deallocated(layout.size());
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            let result = unsafe { System.realloc(ptr, layout, size) };
            if !result.is_null() {
                REALLOCATIONS.fetch_add(1, Relaxed);
                deallocated(layout.size());
                allocated(size);
            }
            result
        }
    }

    pub struct Snapshot {
        allocations: u64,
        reallocations: u64,
        allocated: u64,
        deallocated: u64,
        live: i64,
    }

    impl Snapshot {
        pub fn start() -> Self {
            let live = LIVE.load(Relaxed);
            PEAK.store(live, Relaxed);
            Self {
                allocations: ALLOCATIONS.load(Relaxed),
                reallocations: REALLOCATIONS.load(Relaxed),
                allocated: ALLOCATED.load(Relaxed),
                deallocated: DEALLOCATED.load(Relaxed),
                live,
            }
        }

        pub fn finish(self) -> Value {
            // Capture before constructing JSON, which allocates itself.
            let allocations = ALLOCATIONS.load(Relaxed) - self.allocations;
            let reallocations = REALLOCATIONS.load(Relaxed) - self.reallocations;
            let allocated = ALLOCATED.load(Relaxed) - self.allocated;
            let deallocated = DEALLOCATED.load(Relaxed) - self.deallocated;
            let peak = PEAK.load(Relaxed) - self.live;
            let retained = LIVE.load(Relaxed) - self.live;
            json!({
                "allocations": allocations, "reallocations": reallocations,
                "requested_allocated_bytes": allocated,
                "requested_deallocated_bytes": deallocated,
                "peak_live_requested_bytes_above_start": peak.max(0),
                "live_requested_bytes_delta": retained,
                "live_requested_bytes_at_start": self.live,
            })
        }
    }
}

#[cfg(feature = "alloc-profile")]
#[global_allocator]
static ALLOCATOR: counted::Allocator = counted::Allocator;

#[cfg(feature = "alloc-profile")]
pub use counted::Snapshot;

#[cfg(not(feature = "alloc-profile"))]
pub struct Snapshot;

#[cfg(not(feature = "alloc-profile"))]
impl Snapshot {
    pub fn start() -> Self {
        Self
    }

    pub fn finish(self) -> serde_json::Value {
        serde_json::Value::Null
    }
}
