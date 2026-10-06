//! 材质系统绑定：获取 IMaterialSystem 接口 + proxy 解析函数 `FUN_10002d50` 地址。
//! 详细机制 / 逆向依据见项目根目录 AGENTS.md。

use core::ffi::{CStr, c_char, c_int, c_void};

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

// ---------------------------------------------------------------------------
// client.dll 的 CMaterialProxyDict（引擎自带的材质代理工厂）
//
// 逆向依据见 AGENTS.md「材质代理注册」：工厂单例由 `client.dll` RVA `0x13ab40`
// 的 getter 惰性构造，返回 `&CMaterialProxyDict`，其 vtable（RVA `0x576d94`）
// 只有 3 个槽位：`[0]` CreateProxy、`[1]` 标量删除析构、`[2]` AddProxy。
//
// 把自定义代理用 `AddProxy(name, create_fn)` 注册进来之后，引擎自己的 VMT
// 解析器（`materialsystem` RVA `0x2d50`）就能用 `CreateProxy(name)` 找到它，
// 并调用我们对象的 `Init(IMaterial*, KeyValues*)`——无需再 detour 引擎函数。
// ---------------------------------------------------------------------------

/// 工厂单例 getter（`__cdecl`，无参，`RET` 无操作数，返回 `CMaterialProxyDict*`）。
type GetProxyFactoryFn = unsafe extern "C" fn() -> *mut c_void;
/// `CMaterialProxyDict::AddProxy(const char* name, CreateFn create_fn)`（`RET 0x8`）。
type AddProxyFn = unsafe extern "thiscall" fn(*mut c_void, *const c_char, *const c_void);

/// `client.dll` 模块基址（未加载时为空指针）。
///
/// `HMODULE` 本身就是模块映射基址（来自 C，见 Rust 文档「4. Get it from C」），
/// 自带 provenance，可直接做指针算术，无需降级成 `usize`。
fn client_base() -> *const u8 {
    unsafe {
        GetModuleHandleA(s!("client.dll"))
            .map(|h| h.0.cast::<u8>().cast_const())
            .unwrap_or(core::ptr::null())
    }
}

/// 取 `CMaterialProxyDict` 单例（必要时由 getter 自己完成惰性初始化）。
///
/// # Safety
/// 必须在 `client.dll` 已加载后调用（`OnModuleLoaded("client")` 时机满足）。
pub unsafe fn material_proxy_factory() -> Result<*mut c_void, PluginError> {
    let base = client_base();
    if base.is_null() {
        return Err(PluginError::Unexpected(Box::from("client.dll not loaded")));
    }
    let getter_p = base.add(0x13ab40);
    // 先确认入口可读，再 transmute（函数指针不可为 null，不能靠 transmute 后判空）
    if crate::kv::test_readable(getter_p.cast()).is_err() {
        return Err(PluginError::InvalidPointer);
    }
    let getter: GetProxyFactoryFn = core::mem::transmute(getter_p);
    let factory = getter();
    if factory.is_null() {
        return Err(PluginError::Unexpected(Box::from(
            "CMaterialProxyDict not available",
        )));
    }

    Ok(factory)
}

/// 调用 `CMaterialProxyDict::AddProxy(name, create_fn)`（vtable 槽位 2）。
///
/// 重名时工厂会覆盖旧条目（幂等），因此重复调用安全。
///
/// # Safety
/// `factory` 必须是 [`material_proxy_factory`] 的返回值；`create_fn` 必须是
/// 无参 `__cdecl` 函数，返回一个 `IMaterialProxy` 兼容对象（见 `material` 模块）。
pub unsafe fn add_material_proxy(
    factory: *mut c_void,
    name: &CStr,
    create_fn: *const c_void,
) -> Result<(), PluginError> {
    // COM 对象首字段是 vtable 指针；用 `&raw const` 读取，不构造引用
    let vft: *const usize = *(&raw const *factory).cast::<*const usize>();
    if vft.is_null() {
        return Err(PluginError::InvalidPointer);
    }
    // vtable 槽存的是函数地址（外部函数指针，无 `&raw` 来源）：读写全程用 usize
    let slot = *vft.add(2);
    if slot == 0 {
        return Err(PluginError::InvalidPointer);
    }
    let add: AddProxyFn = core::mem::transmute(slot);
    add(factory, name.as_ptr(), create_fn);

    Ok(())
}
