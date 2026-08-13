//! L4N 示例插件：注册自定义材质代理（material proxy）。
//!
//! VMT `"Proxies"` 块里写代理名即可触发 Rust 回调读写材质 VMT 变量。
//! 机制 / 逆向依据 / 注意事项见项目根目录 AGENTS.md。

#![allow(unsafe_op_in_unsafe_fn)]

pub mod engine;
mod kv;
mod expr;
mod material;
mod proxy;
mod util;

use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::OnceLock;

use proxy::*;

// ---------------------------------------------------------------------------
// IL4NPlugin 虚表（与 bin/neko/plugins/l4n_plugin.h 一致）
// ---------------------------------------------------------------------------
#[repr(C)]
struct L4NPluginVtable {
    destructor: unsafe extern "thiscall" fn(*mut L4NPlugin),
    get_interface_version: unsafe extern "thiscall" fn(*const L4NPlugin) -> u32,
    get_name: unsafe extern "thiscall" fn(*const L4NPlugin) -> *const c_char,
    get_version: unsafe extern "thiscall" fn(*const L4NPlugin) -> *const c_char,
    on_module_loaded: unsafe extern "thiscall" fn(*mut L4NPlugin, *const c_char, usize),
    on_game_launch: unsafe extern "thiscall" fn(*mut L4NPlugin),
    on_d3d_created: unsafe extern "thiscall" fn(*mut L4NPlugin, *mut c_void),
    on_d3d_device_created: unsafe extern "thiscall" fn(*mut L4NPlugin, *mut c_void, u8),
}

#[repr(C)]
struct L4NPlugin {
    vtable: &'static L4NPluginVtable,
}

// ---------------------------------------------------------------------------
// 日志
// ---------------------------------------------------------------------------
#[link(name = "kernel32")]
unsafe extern "system" {
    fn OutputDebugStringA(lp: *const c_char);
    fn GetModuleHandleA(lp: *const u8) -> *mut c_void;
    fn GetProcAddress(hmodule: *mut c_void, name: *const u8) -> *mut c_void;
}

fn engine_msg_ptr() -> Option<unsafe extern "C" fn(*const c_char, ...)> {
    unsafe {
        let h = GetModuleHandleA(b"tier0.dll\0".as_ptr());
        if h.is_null() {
            return None;
        }
        let p = GetProcAddress(h, b"Msg\0".as_ptr());
        if p.is_null() {
            None
        } else {
            Some(core::mem::transmute(p))
        }
    }
}
static ENGINE_MSG: OnceLock<Option<unsafe extern "C" fn(*const c_char, ...)>> = OnceLock::new();

fn log(msg: &str) {
    use std::io::Write;
    let line = format!("[l4n-proxy] {msg}\n");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("l4n_material_proxy_plugin.log")
    {
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }
    if let Some(m) = *ENGINE_MSG.get_or_init(engine_msg_ptr) {
        if let Ok(c) = CString::new(line.clone()) {
            unsafe { m(c.as_ptr()) };
        }
    }
    if let Ok(c) = CString::new(line) {
        unsafe { OutputDebugStringA(c.as_ptr()) };
    }
}

// ---------------------------------------------------------------------------
// IL4NPlugin 实现
// ---------------------------------------------------------------------------
static NAME: &CStr = c"L4NRP Material Proxy - MoRanYue";
static VERSION: &CStr = {
    const BYTES: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();

    match CStr::from_bytes_until_nul(BYTES) {
        Ok(c) => c,
        Err(_) => unreachable!()
    }
};

unsafe extern "thiscall" fn dtor(_this: *mut L4NPlugin) {
    material::uninstall();
}
unsafe extern "thiscall" fn get_interface_version(_this: *const L4NPlugin) -> u32 {
    1
}
unsafe extern "thiscall" fn get_name(_this: *const L4NPlugin) -> *const c_char {
    NAME.as_ptr()
}
unsafe extern "thiscall" fn get_version(_this: *const L4NPlugin) -> *const c_char {
    VERSION.as_ptr()
}

/// 尝试绑定 IMaterialSystem + 注册代理 + hook 引擎 proxy 解析函数（幂等）。
unsafe fn try_bind_and_install() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static HOOKED: AtomicBool = AtomicBool::new(false);
    if HOOKED.load(Ordering::SeqCst) {
        return;
    }

    // 1. 绑定 IMaterialSystem
    let _mat = match engine::bind_material_system() {
        Some(m) => m,
        None => {
            log("materialsystem not loaded yet, will retry on D3D device created");
            return;
        }
    };

    material::register_proxy::<DoesEqualProxy>("l4nrp_does_equal");
    material::register_proxy::<CompareProxy>("l4nrp_compare");
    material::register_proxy::<IsInRangeProxy>("l4nrp_is_in_range");
    material::register_proxy::<PrintVariable>("l4nrp_print_variable");
    material::register_proxy::<StrConcatProxy>("l4nrp_str_concat");
    material::register_proxy::<StrReplaceProxy>("l4nrp_str_replace");
    material::register_proxy::<StrSliceProxy>("l4nrp_str_slice");
    material::register_proxy::<VmtName>("l4nrp_vmt_name");
    material::register_proxy::<Vec3Proxy>("l4nrp_vec3");
    material::register_proxy::<MathProxy>("l4nrp_math");
    material::register_proxy::<LogicProxy>("l4nrp_logic");
    material::register_proxy::<DelaySetProxy>("l4nrp_delay_set");
    material::register_proxy::<DelayAbortProxy>("l4nrp_delay_abort");
    log(&format!(
        "registered proxies: {:?}",
        material::registered_names()
    ));

    // 3. hook 引擎 proxy 解析函数 FUN_10002d50
    let parse = engine::get_proxy_parse_addr();
    if parse != 0 && material::install(parse) {
        HOOKED.store(true, Ordering::SeqCst);
        log("proxy parse hook installed (FUN_10002d50, direct var set)");
    } else {
        log("proxy parse hook install FAILED (materialsystem.dll not ready)");
    }
}

unsafe extern "thiscall" fn on_module_loaded(
    _this: *mut L4NPlugin,
    module_name: *const c_char,
    _handle: usize,
) {
    if !module_name.is_null() {
        let name = CStr::from_ptr(module_name).to_string_lossy().into_owned();
        log(&format!("OnModuleLoaded: {name}"));

        if name == "client" {
            try_bind_and_install();
        }
    }
}

unsafe extern "thiscall" fn on_game_launch(_this: *mut L4NPlugin) {
    log("Game launched");
}

unsafe extern "thiscall" fn on_d3d_created(_this: *mut L4NPlugin, d3d: *mut c_void) {
    log(&format!("OnD3DCreated d3d=0x{:x}", d3d as usize));
}

unsafe extern "thiscall" fn on_d3d_device_created(
    _this: *mut L4NPlugin,
    device: *mut c_void,
    is_dxvk: u8,
) {
    log(&format!("OnD3DDeviceCreated device=0x{:x} is_dxvk={}", device as usize, is_dxvk != 0));
    // D3D 首帧 fallback：若此前未绑定成功，再试一次
    try_bind_and_install();
    // 每帧执行持续计算的材质代理（EndScene hook）
    material::install_d3d_endscene(device);
}

// ---------------------------------------------------------------------------
// 虚表 + 实例 + 导出
// ---------------------------------------------------------------------------
static VTABLE: L4NPluginVtable = L4NPluginVtable {
    destructor: dtor,
    get_interface_version,
    get_name,
    get_version,
    on_module_loaded,
    on_game_launch,
    on_d3d_created,
    on_d3d_device_created,
};

static INSTANCE: OnceLock<L4NPlugin> = OnceLock::new();

#[allow(private_interfaces)]
#[unsafe(export_name = "GetL4NPluginInstance")]
pub extern "C" fn l4n_plugin_instance() -> *mut L4NPlugin {
    let inst = INSTANCE.get_or_init(|| L4NPlugin { vtable: &VTABLE });
    let p: *const L4NPlugin = inst;
    p as *mut L4NPlugin
}
