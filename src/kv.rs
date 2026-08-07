//! 材质代理参数解析：每个代理实现 [`Proxy`] trait，参数定义在各自 struct 内（相互隔离）。
//! KeyValues 布局等逆向依据见项目根目录 AGENTS.md。

use core::ffi::c_void;

/// 材质代理统一接口。
///
/// - [`apply_kv`](Proxy::apply_kv)：VMT `"Proxies" { "代理名" { 键 值 } }` 里的每个键值对
///   调用一次，由实现者按名填充自己的参数；
/// - [`bind`](Proxy::bind)：代理触发时的动作（`material` 为 `IMaterial*`）。
///   返回 `false` 表示材质失效/所需变量缺失，调用方应从活动表移除；
/// - [`per_frame`](Proxy::per_frame)：是否每帧执行（默认 `false`，仅材质加载时执行一次）。
pub trait Proxy: Send {
    fn apply_kv(&mut self, name: &str, value: &str);
    unsafe fn bind(&mut self, material: *mut c_void) -> bool;
    fn per_frame(&self) -> bool {
        false
    }
}

// ---------- 内存可读性检查（防 strlen/CStr 读到坏指针崩溃） ----------
#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualQuery(lp: *const c_void, lp_buffer: *mut c_void, dw_length: usize) -> usize;
}

/// 粗略检查指针指向的内存是否已提交且可读（防止 `strlen`/`CStr` 读到坏指针崩溃）。
pub fn is_readable(p: *const c_void) -> bool {
    if p.is_null() {
        return false;
    }
    const MEM_COMMIT: u32 = 0x1000;
    const PAGE_NOACCESS: u32 = 0x01;
    const PAGE_GUARD: u32 = 0x100;
    unsafe {
        // 32 位 MEMORY_BASIC_INFORMATION 布局（x86）
        #[repr(C)]
        struct Mbi32 {
            base: *mut c_void,
            allocation_base: *mut c_void,
            allocation_protect: u32,
            region_size: usize,
            state: u32,
            protect: u32,
            type_: u32,
        }
        let mut mbi: Mbi32 = core::mem::zeroed();
        let n = VirtualQuery(p, (&mut mbi as *mut Mbi32).cast(), core::mem::size_of::<Mbi32>());
        if n == 0 {
            return false;
        }
        (mbi.state & MEM_COMMIT) != 0
            && (mbi.protect & PAGE_GUARD) == 0
            && (mbi.protect & 0xff) != PAGE_NOACCESS
    }
}
