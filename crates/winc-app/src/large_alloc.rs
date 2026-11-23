//! 全局分配器：1 MiB 及以上的大块直接向系统申请（`mmap`），释放时立即归还系统（`munmap`）；更小的块
//! 和超高对齐的块由系统分配器处理。仅在 macOS 上编译。
//!
//! macOS 的系统分配器把已释放的大块保留在空闲缓存中（`MALLOC_LARGE (empty)`），这些页面仍然有效，
//! 仍计入进程内存：在 macOS 26 上实测，反复分配并释放约 2.7 MB 的块 60 次后累计保留 56 MB；从命令行
//! 启动时设置 `MallocLargeCache=0` 不再生效。改用 `mmap` 的代价是每次分配都取得清零页，2.7 MB 的块
//! 约 0.3 ms。

#![allow(unsafe_code)] // 全局分配器和 mmap、munmap

use std::alloc::{GlobalAlloc, Layout, System};

/// 经 `mmap` 直接映射的最小块大小（字节），1 MiB。
///
/// 取值取自调参：更小的块分配次数多、复用频繁，交给系统分配器后进程常驻内存的增长可以忽略；
/// 该阈值按块的大小分布取整到 1 MiB。
const LARGE: usize = 1 << 20;

/// 允许经 `mmap` 分配的地址对齐上限（字节），4 KiB。
///
/// 依据：`mmap` 返回的地址按页对齐，macOS 的页大小为 4 KiB；对齐要求高于该值的块由系统
/// 分配器处理，否则 `mmap` 无法满足调用方给出的对齐要求。
const PAGE_ALIGN: usize = 4096;

/// 映射区域在 `vmmap`、`footprint` 中的内存标记。
///
/// 取应用自定义的第一个标记（`VM_MEMORY_APPLICATION_SPECIFIC_1`），显示为 `Memory Tag 240`，
/// 与系统分配器的内存分开统计。
const VM_TAG: i32 = 240 << 24;

/// 大块经 `mmap` 映射、其余块交由系统分配器的全局分配器。
pub struct LargeAlloc;

/// 测试中当前占用的堆内存（字节），内存测量以此为准。
///
/// 物理内存还包含分配器未归还的空闲页和绘制缓存，无法反映数据结构本身的大小。
/// 仅在测试中计数，正常构建不增加原子操作。
#[cfg(test)]
pub static LIVE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// 记录一次分配（`delta` 为正）或一次释放（`delta` 为负）。
#[inline(always)]
fn count(delta: isize) {
    #[cfg(test)]
    LIVE.fetch_add(delta, std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(test))]
    let _ = delta;
}

/// 是否经 `mmap` 分配 `layout` 描述的内存块。
fn is_large(layout: Layout) -> bool {
    layout.size() >= LARGE && layout.align() <= PAGE_ALIGN
}

/// 映射 `size` 字节清零的匿名内存；失败时返回空指针。
fn map(size: usize) -> *mut u8 {
    // SAFETY: 使用匿名私有映射，不涉及文件描述符；第六个参数位置按 macOS 约定传入内存
    // 标记。返回值等于 `MAP_FAILED` 时在下面检查并转换为空指针。
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            VM_TAG,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        std::ptr::null_mut()
    } else {
        ptr.cast()
    }
}

// 四个方法都不内联：分配器入口会被内联到每一处分配，展开后 release 版二进制增大 280 KB
// （在 release 构建下实测）；多一次函数调用约 1 ns，相对于该体积代价可以忽略。
//
// SAFETY: 大块由 `map` 映射、在 `dealloc` 中按同样的长度解除映射，地址按页对齐，满足
// `is_large` 限定的对齐要求；其余块原样交由 `System` 处理。`realloc` 跨越两种来源时先分配
// 新块、复制内容、再按原来的 `layout` 释放旧块。
unsafe impl GlobalAlloc for LargeAlloc {
    #[inline(never)]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size() as isize);
        if is_large(layout) {
            map(layout.size())
        } else {
            // SAFETY: 调用方保证 `layout` 的大小不为 0。
            unsafe { System.alloc(layout) }
        }
    }

    #[inline(never)]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size() as isize);
        if is_large(layout) {
            // 匿名映射返回的页已清零。
            map(layout.size())
        } else {
            // SAFETY: 调用方保证 `layout` 的大小不为 0、对齐是 2 的幂；`System` 的 `alloc_zeroed`
            // 要求与 `alloc` 相同的条件；`count` 也已按同一大小记账。
            unsafe { System.alloc_zeroed(layout) }
        }
    }

    #[inline(never)]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count(-(layout.size() as isize));
        if is_large(layout) {
            // SAFETY: `ptr` 是 `map(layout.size())` 返回且尚未解除的映射。
            unsafe { libc::munmap(ptr.cast(), layout.size()) };
        } else {
            // SAFETY: `ptr` 由 `System` 按 `layout` 分配。
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[inline(never)]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: 调用方保证 `new_size` 不为 0，且按 `layout.align()` 取整后不溢出。
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        if !is_large(layout) && !is_large(new_layout) {
            count(new_size as isize - layout.size() as isize);
            // SAFETY: 新旧布局都由系统分配器处理，参数原样转交。
            return unsafe { System.realloc(ptr, layout, new_size) };
        }
        // SAFETY: `new_layout` 由上一句的检查保证合法。
        let new = unsafe { self.alloc(new_layout) };
        if !new.is_null() {
            // SAFETY: 两块都至少有 `min(旧大小, 新大小)` 字节，且是不同的分配，不重叠；旧块
            // 按原来的 `layout` 释放。
            unsafe {
                std::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
                self.dealloc(ptr, layout);
            }
        }
        new
    }
}

#[cfg(test)]
mod tests;
