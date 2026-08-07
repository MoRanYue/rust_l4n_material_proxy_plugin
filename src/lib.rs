//! L4N 示例插件：注册自定义材质代理（material proxy）。
//!
//! VMT `"Proxies"` 块里写代理名即可触发 Rust 回调读写材质 VMT 变量。
//! 机制 / 逆向依据 / 注意事项见项目根目录 AGENTS.md。

#![allow(unsafe_op_in_unsafe_fn)]

mod engine;
mod kv;
mod material;
mod util;

use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::OnceLock;

use kv::Proxy;
use util::RelativeCompare;

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
// 自定义材质代理（每个代理一个 struct，参数隔离）
// ---------------------------------------------------------------------------

/// 从 &str 构造 CString。
fn cstr_of(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("").unwrap())
}

/// 同步更新变量名 `String` 与其缓存的 `CString`（apply_kv 使用）。
fn set_kv(dst: &mut String, c: &mut CString, value: &str) {
    *dst = value.to_string();
    *c = cstr_of(value);
}

/// l4nrp_color_ramp —— 演示可配置参数（color_r/g/b + rate/scale + text）。
#[derive(Default)]
struct ColorRampProxy {
    color_r: f32,
    color_g: f32,
    color_b: f32,
    rate: f32,
    scale: f32,
    flag: i32,
    text: String,
}
impl Proxy for ColorRampProxy {
    fn apply_kv(&mut self, name: &str, value: &str) {
        let lname = name.to_ascii_lowercase();
        let f = |def: f32| value.trim().parse::<f32>().unwrap_or(def);
        match lname.as_str() {
            "color_r" | "red" | "r" => self.color_r = f(self.color_r),
            "color_g" | "green" | "g" => self.color_g = f(self.color_g),
            "color_b" | "blue" | "b" => self.color_b = f(self.color_b),
            "rate" | "speed" => self.rate = f(self.rate),
            "scale" => self.scale = f(self.scale),
            "flag" | "enabled" | "enable" => self.flag = value.trim().parse().unwrap_or(self.flag),
            "text" | "string" => self.text = value.to_string(),
            _ => {}
        }
    }
    unsafe fn bind(&mut self, _material: *mut c_void) -> bool {
        log(&format!(
            "OnBind(color_ramp): color=({:.2},{:.2},{:.2}) rate={:.2} scale={:.2} flag={} text='{}'",
            self.color_r, self.color_g, self.color_b, self.rate, self.scale, self.flag, self.text
        ));
        true
    }
}

/// l4nrp_log_pulse —— 演示带 enabled 开关的参数。
#[derive(Default)]
struct LogPulseProxy {
    flag: i32,
    color_b: f32,
    rate: f32,
}
impl Proxy for LogPulseProxy {
    fn apply_kv(&mut self, name: &str, value: &str) {
        let lname = name.to_ascii_lowercase();
        let f = |def: f32| value.trim().parse::<f32>().unwrap_or(def);
        match lname.as_str() {
            "color_b" | "blue" | "b" => self.color_b = f(self.color_b),
            "rate" | "speed" => self.rate = f(self.rate),
            "flag" | "enabled" | "enable" => self.flag = value.trim().parse().unwrap_or(self.flag),
            _ => {}
        }
    }
    unsafe fn bind(&mut self, _material: *mut c_void) -> bool {
        if self.flag != 0 {
            log(&format!(
                "OnBind(log_pulse): enabled, color_b={:.2} rate={:.2}",
                self.color_b, self.rate
            ));
        } else {
            log("OnBind(log_pulse): disabled (flag=0)");
        }
        true
    }
}

/// l4nrp_force_red —— 把材质整体调色改成纯红 (1,0,0)，肉眼验证代理链路。
#[derive(Default)]
struct ForceRedProxy;
impl Proxy for ForceRedProxy {
    fn apply_kv(&mut self, _name: &str, _value: &str) {}
    unsafe fn bind(&mut self, material: *mut c_void) -> bool {
        if material.is_null() {
            return false;
        }
        let candidates: [&CStr; 4] = [c"$color2", c"$color", c"$selfillumtint", c"$envmaptint"];
        for name in candidates {
            let out = material::find_var(material, name);
            if !out.is_null() {
                material::set_vec(out, &[1.0, 0.0, 0.0]);
                log(&format!("force_red: {name:?} = (1,0,0) (should appear red)"));
                return true;
            }
        }
        log("force_red: no tint variable found ($color2/$color/$selfillumtint/$envmaptint)");
        true
    }
}

/// l4nrp_does_equal —— 读两个输入变量（变量名可用参数配置），相等则输出 1.0 否则 0.0。
struct DoesEqualProxy {
    src_a: String,
    src_b: String,
    result: String,
    last_result: f32,
    // 缓存变量名，避免每帧堆分配（apply_kv 更新时重建）
    src_a_n: CString,
    src_b_n: CString,
    result_n: CString,
}
impl Default for DoesEqualProxy {
    fn default() -> Self {
        DoesEqualProxy {
            src_a: "$src_var_1".into(),
            src_b: "$src_var_2".into(),
            result: "$result_var".into(),
            last_result: -1.0,
            src_a_n: cstr_of("$src_var_1"),
            src_b_n: cstr_of("$src_var_2"),
            result_n: cstr_of("$result_var"),
        }
    }
}
impl Proxy for DoesEqualProxy {
    fn apply_kv(&mut self, name: &str, value: &str) {
        match name.to_ascii_lowercase().as_str() {
            "src_a" | "src1" | "input_a" | "input1" => set_kv(&mut self.src_a, &mut self.src_a_n, value),
            "src_b" | "src2" | "input_b" | "input2" => set_kv(&mut self.src_b, &mut self.src_b_n, value),
            "result" | "result_var" | "output" => set_kv(&mut self.result, &mut self.result_n, value),
            _ => {}
        }
    }
    // 依赖每帧变化的输入，需要每帧重算
    fn per_frame(&self) -> bool {
        true
    }
    unsafe fn bind(&mut self, material: *mut c_void) -> bool {
        if material.is_null() {
            return false;
        }
        let a = material::get_float(material::find_var(material, &self.src_a_n));
        let b = material::get_float(material::find_var(material, &self.src_b_n));
        let out = material::find_var(material, &self.result_n);
        if out.is_null() {
            // 材质失效/变量缺失 → 从活动表移除
            return false;
        }
        let v = if (a - b).abs() < 1e-6 { 1.0 } else { 0.0 };
        material::set_float(out, v);
        #[cfg(debug_assertions)]
        {
            if (v - self.last_result).abs() > 0.5 {
                let mat_name = material::get_name(material).unwrap_or_else(|| "?".into());
                log(&format!(
                    "does_equal[{}]: {}={a:.4} {}={b:.4} -> {}={v:.0}",
                    mat_name, self.src_a, self.src_b, self.result
                ));
                self.last_result = v;
            }
        }
        true
    }
}

/// l4nrp_compare —— 读两个输入变量（变量名可用参数配置），a == b => 0.0，a > b => 1.0，a < b => -1.0。
struct CompareProxy {
    src_a: String,
    src_b: String,
    result: String,
    last_result: f32,
    src_a_n: CString,
    src_b_n: CString,
    result_n: CString,
}
impl Default for CompareProxy {
    fn default() -> Self {
        Self {
            src_a: "$src_var_1".into(),
            src_b: "$src_var_2".into(),
            result: "$result_var".into(),
            last_result: -1.0,
            src_a_n: cstr_of("$src_var_1"),
            src_b_n: cstr_of("$src_var_2"),
            result_n: cstr_of("$result_var"),
        }
    }
}
impl Proxy for CompareProxy {
    fn apply_kv(&mut self, name: &str, value: &str) {
        match name.to_ascii_lowercase().as_str() {
            "src_a" | "src1" | "input_a" | "input1" => set_kv(&mut self.src_a, &mut self.src_a_n, value),
            "src_b" | "src2" | "input_b" | "input2" => set_kv(&mut self.src_b, &mut self.src_b_n, value),
            "result" | "result_var" | "output" => set_kv(&mut self.result, &mut self.result_n, value),
            _ => {}
        }
    }
    fn per_frame(&self) -> bool {
        true
    }
    unsafe fn bind(&mut self, material: *mut c_void) -> bool {
        if material.is_null() {
            return false;
        }
        let a = material::get_float(material::find_var(material, &self.src_a_n));
        let b = material::get_float(material::find_var(material, &self.src_b_n));
        let out = material::find_var(material, &self.result_n);
        if out.is_null() {
            // 材质失效/变量缺失 → 从活动表移除
            return false;
        }
        let v = match a.relative_cmp(&b) {
            std::cmp::Ordering::Less => -1.0,
            std::cmp::Ordering::Equal => 0.0,
            std::cmp::Ordering::Greater => 1.0
        };
        material::set_float(out, v);
        #[cfg(debug_assertions)]
        {
            if (v - self.last_result).abs() > 0.5 {
                let mat_name = material::get_name(material).unwrap_or_else(|| "?".into());
                log(&format!(
                    "compare[{}]: {}={a:.4} {}={b:.4} -> {}={v:.0}",
                    mat_name, self.src_a, self.src_b, self.result
                ));
                self.last_result = v;
            }
        }
        true
    }
}

/// l4nrp_is_in_range —— 读输入变量（变量名可配置），在 [min,max] 内则输出 1.0 否则 0.0。
struct IsInRangeProxy {
    src: String,
    min: String,
    max: String,
    result: String,
    last_result: f32,
    // 缓存变量名，避免每帧堆分配（apply_kv 更新时重建）
    src_n: CString,
    min_n: CString,
    max_n: CString,
    result_n: CString,
}
impl Default for IsInRangeProxy {
    fn default() -> Self {
        IsInRangeProxy {
            src: "$src_var".into(),
            min: "$min_var".into(),
            max: "$max_var".into(),
            result: "$result_var".into(),
            last_result: -1.0,
            src_n: cstr_of("$src_var"),
            min_n: cstr_of("$min_var"),
            max_n: cstr_of("$max_var"),
            result_n: cstr_of("$result_var"),
        }
    }
}
impl Proxy for IsInRangeProxy {
    fn apply_kv(&mut self, name: &str, value: &str) {
        match name.to_ascii_lowercase().as_str() {
            "src" | "src_var" | "input" => set_kv(&mut self.src, &mut self.src_n, value),
            "min" | "min_var" => set_kv(&mut self.min, &mut self.min_n, value),
            "max" | "max_var" => set_kv(&mut self.max, &mut self.max_n, value),
            "result" | "result_var" | "output" => set_kv(&mut self.result, &mut self.result_n, value),
            _ => {}
        }
    }
    // 依赖每帧变化的输入，需要每帧重算
    fn per_frame(&self) -> bool {
        true
    }
    unsafe fn bind(&mut self, material: *mut c_void) -> bool {
        if material.is_null() {
            return false;
        }
        let src = material::get_float(material::find_var(material, &self.src_n));
        let min = material::get_float(material::find_var(material, &self.min_n));
        let max = material::get_float(material::find_var(material, &self.max_n));
        let out = material::find_var(material, &self.result_n);
        if out.is_null() {
            // 材质失效/变量缺失 → 从活动表移除
            return false;
        }
        let v = if src >= min && src <= max { 1.0 } else { 0.0 };
        material::set_float(out, v);
        #[cfg(debug_assertions)]
        {
            if (v - self.last_result).abs() > 0.5 {
                let mat_name = material::get_name(material).unwrap_or_else(|| "?".into());
                log(&format!(
                    "is_in_range[{}]: {}={src:.4} in [{}={min:.4},{}={max:.4}] -> {}={v:.0}",
                    mat_name, self.src, self.min, self.max, self.result
                ));
                self.last_result = v;
            }
        }
        true
    }
}

/// l4nrp_print_variable —— 每帧读取并打印变量值（`type` 可配置：float / int / vector）。
///
/// 数值（float/int）与向量（vector）分开处理：读取函数与日志格式各自独立。
#[derive(Clone, Copy, PartialEq)]
enum VarType {
    Float,
    Int,
    Vector,
}
impl VarType {
    /// 解析 VMT 参数值（"float"/"int"/"vector" 等），未知值回退到 Float。
    fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "int" | "integer" => VarType::Int,
            "vector" | "vec3" | "vec" => VarType::Vector,
            _ => VarType::Float,
        }
    }
}

struct PrintVariable {
    var: String,
    var_type: VarType,
    // 缓存变量名，避免每帧堆分配（apply_kv 更新时重建）
    var_n: CString,
}
impl Default for PrintVariable {
    fn default() -> Self {
        PrintVariable {
            var: "$var".into(),
            var_type: VarType::Float,
            var_n: cstr_of("$var"),
        }
    }
}
impl Proxy for PrintVariable {
    fn apply_kv(&mut self, name: &str, value: &str) {
        match name.to_ascii_lowercase().as_str() {
            "var" | "var_name" | "variable" | "src" | "input" => set_kv(&mut self.var, &mut self.var_n, value),
            "type" | "var_type" | "variable_type" => self.var_type = VarType::parse(value),
            _ => {}
        }
    }
    // 依赖每帧变化的输入，需要每帧重算
    fn per_frame(&self) -> bool {
        true
    }
    unsafe fn bind(&mut self, material: *mut c_void) -> bool {
        if material.is_null() {
            return false;
        }
        let var = material::find_var(material, &self.var_n);
        if var.is_null() {
            // 材质失效/变量缺失 → 从活动表移除
            return false;
        }
        let mat_name = material::get_name(material).unwrap_or_else(|| "?".into());
        match self.var_type {
            // ---- 向量：get_vec 读三分量 ----
            VarType::Vector => {
                let mut o = [0.0f32; 3];
                material::get_vec(var, &mut o);
                log(&format!(
                    "print_variable[{}]: {}={:.4},{:.4},{:.4} (vec)",
                    mat_name, self.var, o[0], o[1], o[2]
                ));
            }
            // ---- 整数：get_float 后取整显示 ----
            VarType::Int => {
                let v = material::get_float(var);
                log(&format!(
                    "print_variable[{}]: {}={} (int)",
                    mat_name, self.var, v as i32
                ));
            }
            // ---- 标量浮点 ----
            VarType::Float => {
                let v = material::get_float(var);
                log(&format!("print_variable[{}]: {}={v:.4} (float)", mat_name, self.var));
            }
        }
        true
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

    // 2. 注册自定义材质代理
    #[cfg(debug_assertions)]
    {
        material::register_proxy::<ColorRampProxy>("l4nrp_color_ramp");
        material::register_proxy::<LogPulseProxy>("l4nrp_log_pulse");
        material::register_proxy::<ForceRedProxy>("l4nrp_force_red");
    }
    material::register_proxy::<DoesEqualProxy>("l4nrp_does_equal");
    material::register_proxy::<CompareProxy>("l4nrp_compare");
    material::register_proxy::<IsInRangeProxy>("l4nrp_is_in_range");
    material::register_proxy::<PrintVariable>("l4nrp_print_variable");
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
