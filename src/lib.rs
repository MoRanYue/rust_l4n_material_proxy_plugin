//! L4N 示例插件：注册自定义材质代理（material proxy）。
//!
//! VMT `"Proxies"` 块里写代理名即可触发 Rust 回调读写材质 VMT 变量。
//! 机制 / 逆向依据 / 注意事项见项目根目录 AGENTS.md。

#![allow(unsafe_op_in_unsafe_fn)]
// 禁止 `&T as *const T` / `&mut T as *mut T`：裸指针创建统一走 `&raw const` / `&raw mut`
// （见 AGENTS.md「裸指针创建约定」）。`with_exposed_provenance` 是仅有的例外，用于没有
// `&raw` 来源的外部地址（模块基址 + RVA、函数指针转 usize）。
#![warn(clippy::ref_as_ptr)]

pub mod engine;
mod kv;
mod expr;
mod material;
mod menu;
mod proxy;
mod util;
mod error;

use windows::Win32::Foundation::GetLastError;
use windows::core::{HSTRING, s};
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::{Mutex, OnceLock};

use error::PluginError;
use proxy::*;

// ---------------------------------------------------------------------------
// IL4NPlugin 虚表（与 bin/neko/plugins/l4n_plugin.h 一致）
//
// 槽位 0-7 来自 `IL4NPluginV1`，槽位 8（字节偏移 0x20）是接口版本 2 新增的
// `IL4NPlugin::RequestHudMenu`——虚表里只有这一个新增槽位，详见 `menu` 模块。
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
    request_hud_menu: unsafe extern "thiscall" fn(*const L4NPlugin, u8) -> *const c_char,
}

#[repr(C)]
struct L4NPlugin {
    vtable: &'static L4NPluginVtable,
}

fn engine_msg_ptr() -> Result<unsafe extern "C" fn(*const c_char, ...), PluginError> {
    unsafe {
        let h = GetModuleHandleA(s!("tier0.dll"))?;
        GetProcAddress(h, s!("Msg"))
            .ok_or(PluginError::Windows(GetLastError().into()))
            .map(|p| core::mem::transmute(p) )
    }
}
static ENGINE_MSG: OnceLock<Option<unsafe extern "C" fn(*const c_char, ...)>> = OnceLock::new();

/// 全局日志文件句柄：首次写日志时以截断方式创建（清除上一次运行日志），之后复用句柄追加写入。
struct LogSink {
    file: std::fs::File,
}

static LOG_SINK: OnceLock<Mutex<Option<LogSink>>> = OnceLock::new();

fn log(msg: &str) {
    use std::io::Write;
    let line = format!("[l4n-proxy] {msg}\n");
    {
        let sink = LOG_SINK.get_or_init(|| Mutex::new(None));
        let mut guard = sink.lock().unwrap();
        if guard.is_none() {
            // 每次游戏运行（插件加载）首次写日志前，清空上一次运行留下的日志文件。
            *guard = std::fs::File::create("l4n_material_proxy_plugin.log")
                .ok()
                .map(|file| LogSink { file });
        }
        if let Some(s) = guard.as_mut() {
            let _ = s.file.write_all(line.as_bytes());
            let _ = s.file.flush();
        }
    }
    if let Some(m) = *ENGINE_MSG.get_or_init(|| engine_msg_ptr().ok()) {
        if let Ok(c) = CString::new(line.clone()) {
            unsafe { m(c.as_ptr()) };
        }
    }
    unsafe { OutputDebugStringW(&HSTRING::from(line)); }
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
/// 返回本插件实现的接口版本。left4neko 的插件加载器把这个返回值存进插件表
/// （每项 12 字节：`{+0, +4 实例指针, +8 版本号}`），`PluginManager::BuildHudMenu`
/// （`0x100d2970`）只对 **版本 >= 2** 的插件调用虚表槽位 8 `RequestHudMenu`。
unsafe extern "thiscall" fn get_interface_version(_this: *const L4NPlugin) -> u32 {
    2
}
unsafe extern "thiscall" fn get_name(_this: *const L4NPlugin) -> *const c_char {
    NAME.as_ptr()
}
unsafe extern "thiscall" fn get_version(_this: *const L4NPlugin) -> *const c_char {
    VERSION.as_ptr()
}

/// 尝试绑定 IMaterialSystem + 注册代理 + hook 引擎 proxy 解析函数（幂等）。
unsafe fn try_bind_and_install() -> Result<(), PluginError> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static HOOKED: AtomicBool = AtomicBool::new(false);
    if HOOKED.load(Ordering::SeqCst) {
        return Ok(());
    }

    // 1. 绑定 IMaterialSystem
    let _mat = engine::bind_material_system()?;

    material::register_proxy::<DoesEqualProxy>("l4nrp_does_equal");
    material::register_proxy::<CompareProxy>("l4nrp_compare");
    material::register_proxy::<IsInRangeProxy>("l4nrp_is_in_range");
    material::register_proxy::<PrintVariable>("l4nrp_print_variable");
    material::register_proxy::<StrConcatProxy>("l4nrp_str_concat");
    material::register_proxy::<StrReplaceProxy>("l4nrp_str_replace");
    material::register_proxy::<StrSliceProxy>("l4nrp_str_slice");
    material::register_proxy::<StrLenProxy>("l4nrp_str_len");
    material::register_proxy::<VmtName>("l4nrp_vmt_name");
    material::register_proxy::<Vec3Proxy>("l4nrp_vec3");
    material::register_proxy::<MathProxy>("l4nrp_math");
    material::register_proxy::<LogicProxy>("l4nrp_logic");
    material::register_proxy::<RandomProxy>("l4nrp_random");
    material::register_proxy::<DelaySetProxy>("l4nrp_delay_set");
    material::register_proxy::<DelayAbortProxy>("l4nrp_delay_abort");
    log(&format!(
        "registered proxies: {:?}",
        material::registered_names()
    ));

    // 3. hook 引擎 proxy 解析函数 FUN_10002d50
    let parse = engine::get_proxy_parse_addr()?;
    material::install(parse)?;
    HOOKED.store(true, Ordering::SeqCst);

    Ok(())
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
            match try_bind_and_install() {
                Ok(_) => log("Successfully installed hooks"),
                Err(e) => log(&format!("Failed to install hooks: {}", e)),
            }
        }
    }
}

unsafe extern "thiscall" fn on_game_launch(_this: *mut L4NPlugin) {
    log("OnGameLaunch");
}

unsafe extern "thiscall" fn on_d3d_created(_this: *mut L4NPlugin, d3d: *mut c_void) {
    log(&format!("OnD3DCreated d3d=0x{:x}", d3d.addr()));
}

unsafe extern "thiscall" fn on_d3d_device_created(
    _this: *mut L4NPlugin,
    device: *mut c_void,
    is_dxvk: u8,
) {
    log(&format!("OnD3DDeviceCreated device=0x{:x} is_dxvk={}", device.addr(), is_dxvk != 0));
    match material::install_d3d_endscene(device) {
        Ok(()) => log(&format!(
            "D3D EndScene hook installed (device=0x{:x})",
            device.addr()
        )),
        Err(e) => log(&format!(
            "D3D EndScene hook error: {}",
            e
        ))
    }
}

/// `IL4NPlugin::RequestHudMenu`（接口版本 2 新增，虚表槽位 8 / 字节偏移 `0x20`）。
///
/// left4neko 以 MSVC x86 的 `thiscall` 调用本函数，`bool request_title` 占一个
/// 32 位栈槽（调用点实测为 `PUSH 0x1` / `PUSH 0x0`），因此这里用 `u8` 接收。
/// 返回值语义与实现见 [`menu`]。
unsafe extern "thiscall" fn request_hud_menu(
    _this: *const L4NPlugin,
    request_title: u8,
) -> *const c_char {
    menu::request(request_title)
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
    request_hud_menu,
};

static INSTANCE: OnceLock<L4NPlugin> = OnceLock::new();

#[allow(private_interfaces)]
#[unsafe(export_name = "GetL4NPluginInstance")]
pub extern "C" fn l4n_plugin_instance() -> *mut L4NPlugin {
    // 这里拿到的是共享引用 `&L4NPlugin`（`INSTANCE` 为 `static`，`get_or_init` 只给 `&T`），
    // 无法用 `&raw mut` 取裸指针，只能先 `&raw const` 再 cast 成 `*mut` 交给 left4neko。
    let inst: *const L4NPlugin = &raw const *INSTANCE.get_or_init(|| L4NPlugin { vtable: &VTABLE });
    inst.cast_mut()
}
