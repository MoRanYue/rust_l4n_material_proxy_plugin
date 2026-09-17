//! 材质系统绑定：获取 IMaterialSystem 接口 + proxy 解析函数 `FUN_10002d50` 地址。
//! 详细机制 / 逆向依据见项目根目录 AGENTS.md。

use core::ffi::{c_char, c_int, c_void};

use windows::core::{PCSTR, s};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};

use crate::error::PluginError;

type CreateInterfaceFn = unsafe extern "C" fn(*const c_char, *mut c_int) -> *mut c_void;

const MATERIAL_SYSTEM_INTERFACE: PCSTR = s!("VMaterialSystem080");
const MATERIAL_SYSTEM_INTERFACE_OLD: PCSTR = s!("VMaterialSystem079");

/// 获取 IMaterialSystem 接口（用于确认 materialsystem.dll 已加载）。
pub fn bind_material_system() -> Result<*mut c_void, PluginError> {
    unsafe {
        for m in [s!("materialsystem.dll"), s!("engine.dll"), s!("tier0.dll")] {
            let Ok(h) = GetModuleHandleA(m) else { continue; };
            let Some(create) = GetProcAddress(h, s!("CreateInterface")) else { continue; };
            let f: CreateInterfaceFn = core::mem::transmute(create);
            for ver in [MATERIAL_SYSTEM_INTERFACE, MATERIAL_SYSTEM_INTERFACE_OLD] {
                // `ver.0` 已是 `*const u8`，直接 `cast` 成 `*const c_char`（同宽度，非创建新指针）
                let ms = f(ver.0.cast::<c_char>(), core::ptr::null_mut());
                if !ms.is_null() {
                    return Ok(ms);
                }
            }
        }

        Err(PluginError::Unexpected(Box::from("failed to get IMaterialSystem")))
    }
}

/// 获取 proxy 解析函数 `FUN_10002d50` 的入口指针（materialsystem RVA `0x2d50`）。
///
/// 返回**带 provenance 的指针**：`HMODULE` 即模块映射基址（来自 C，见 Rust 文档
/// 「4. Get it from C」），偏移用 `.add()` 做指针算术，全程不经过 `usize`，
/// 因此 detour 可以直接读写该入口字节。
/// # Safety
/// 必须等 `materialsystem.dll` 加载后调用。
pub unsafe fn get_proxy_parse_addr() -> Result<*const u8, PluginError> {
    let base = GetModuleHandleA(s!("materialsystem.dll"))?;
    Ok(base.0.cast_const().cast::<u8>().add(0x2d50))
}
