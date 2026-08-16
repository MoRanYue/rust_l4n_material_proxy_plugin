//! 材质代理参数解析：每个代理实现 [`Proxy`] trait，参数定义在各自 struct 内（相互隔离）。
//! KeyValues 布局等逆向依据见项目根目录 AGENTS.md。

use core::ffi::c_void;

use windows::Win32::{Foundation::GetLastError, System::Memory::{MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery}};

use crate::error::PluginError;

/// 材质代理统一接口。
pub trait Proxy: Send {
    /// 对 VMT 中每个 `"Proxies" { "代理名" { "键" "值" } }` 键值对调用一次，由实现者按名填充自己的参数。
    fn apply_kv(&mut self, name: &str, value: &str);
    /// 代理触发时的动作（`material` 为 `IMaterial*`）。返回 `Err(_)` 时调用方会将材质从活动表移除；
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError>;
    /// 是否每帧执行，为 `false` 时仅材质加载时执行一次。
    fn per_frame(&self) -> bool {
        false
    }
}

// ---------- 内存可读性检查（防 strlen/CStr 读到坏指针崩溃） ----------

/// 粗略检查指针指向的内存是否已提交且可读（防止 `strlen`/`CStr` 读到坏指针崩溃）。
pub fn test_readable(p: *const c_void) -> Result<(), PluginError> {
    if p.is_null() {
        return Err(PluginError::InvalidPointer);
    }
    unsafe {
        let mut mbi = core::mem::zeroed();
        let n = VirtualQuery(Some(p), &mut mbi, core::mem::size_of::<MEMORY_BASIC_INFORMATION>());
        if n == 0 {
            return Err(PluginError::Windows(GetLastError().into()));
        }
        
        if mbi.State != MEM_COMMIT {
            return Err(PluginError::InaccesibleMemory);
        }
        if (mbi.Protect & PAGE_NOACCESS).0 != 0 || (mbi.Protect & PAGE_GUARD).0 != 0 {
            return Err(PluginError::InaccesibleMemory);
        }
        if mbi.Protect.0 & 0xEE == 0 {
            return Err(PluginError::InaccesibleMemory);
        }

        Ok(())
    }
}
