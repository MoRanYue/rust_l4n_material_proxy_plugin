//! 材质系统绑定：获取 IMaterialSystem 接口 + proxy 解析函数 `FUN_10002d50` 地址。
//! 详细机制 / 逆向依据见项目根目录 AGENTS.md。

use core::ffi::{c_char, c_int, c_void};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleA(lp: *const u8) -> *mut c_void;
    fn GetProcAddress(hmodule: *mut c_void, name: *const u8) -> *mut c_void;
}

type CreateInterfaceFn = unsafe extern "C" fn(*const c_char, *mut c_int) -> *mut c_void;

const MATERIAL_SYSTEM_INTERFACE: &[u8] = b"VMaterialSystem080\0";
const MATERIAL_SYSTEM_INTERFACE_OLD: &[u8] = b"VMaterialSystem079\0";

/// 获取 IMaterialSystem 接口（用于确认 materialsystem.dll 已加载）。
pub fn bind_material_system() -> Option<*mut c_void> {
    unsafe {
        for m in [b"materialsystem.dll\0".as_ptr(), b"engine.dll\0".as_ptr(), b"tier0.dll\0".as_ptr()] {
            let h = GetModuleHandleA(m);
            if h.is_null() {
                continue;
            }
            let create = GetProcAddress(h, b"CreateInterface\0".as_ptr());
            if create.is_null() {
                continue;
            }
            let f: CreateInterfaceFn = core::mem::transmute(create);
            for ver in [MATERIAL_SYSTEM_INTERFACE.as_ptr(), MATERIAL_SYSTEM_INTERFACE_OLD.as_ptr()] {
                let ms = f(ver as *const c_char, core::ptr::null_mut());
                if !ms.is_null() {
                    return Some(ms);
                }
            }
        }
        None
    }
}

/// 获取 proxy 解析函数 `FUN_10002d50` 地址（materialsystem RVA `0x2d50`）。
/// # Safety
/// 必须等 `materialsystem.dll` 加载后调用。
pub unsafe fn get_proxy_parse_addr() -> usize {
    let base = GetModuleHandleA(b"materialsystem.dll\0".as_ptr()) as usize;
    if base == 0 {
        return 0;
    }
    base + 0x2d50
}
