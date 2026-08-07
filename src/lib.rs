//! L4N 示例插件：注册自定义材质代理（material proxy）。
//!
//! VMT `"Proxies"` 块里写代理名即可触发 Rust 回调读写材质 VMT 变量。
//! 机制 / 逆向依据 / 注意事项见项目根目录 AGENTS.md。

#![allow(unsafe_op_in_unsafe_fn)]

mod engine;
mod kv;
mod expr;
mod material;
mod util;

use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::OnceLock;

use kv::Proxy;
use util::{RelativeCompare, EPS};

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

/// l4nrp_does_equal —— 读两个输入变量（变量名可用参数配置），相等则输出 1 否则 0。
struct DoesEqualProxy {
    src_a: String,
    src_b: String,
    result: String,
    last_result: i32,
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
            last_result: -1,
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
        let v = if (a - b).abs() < EPS { 1 } else { 0 };
        material::set_int(out, v);
        #[cfg(debug_assertions)]
        {
            if v != self.last_result {
                let mat_name = material::get_name(material).unwrap_or_else(|| "?".into());
                log(&format!(
                    "does_equal[{}]: {}={a:.4} {}={b:.4} -> {}={v}",
                    mat_name, self.src_a, self.src_b, self.result
                ));
                self.last_result = v;
            }
        }
        true
    }
}

/// l4nrp_compare —— 读两个输入变量（变量名可用参数配置），a == b => 0，a > b => 1，a < b => -1。
struct CompareProxy {
    src_a: String,
    src_b: String,
    result: String,
    last_result: i32,
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
            last_result: -1,
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
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1
        };
        material::set_int(out, v);
        #[cfg(debug_assertions)]
        {
            if v != self.last_result {
                let mat_name = material::get_name(material).unwrap_or_else(|| "?".into());
                log(&format!(
                    "compare[{}]: {}={a:.4} {}={b:.4} -> {}={v}",
                    mat_name, self.src_a, self.src_b, self.result
                ));
                self.last_result = v;
            }
        }
        true
    }
}

/// l4nrp_is_in_range —— 读输入变量（变量名可配置），在 [min,max] 内则输出 1 否则 0。
struct IsInRangeProxy {
    src: String,
    min: String,
    max: String,
    result: String,
    last_result: i32,
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
            last_result: -1,
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
        let v = if src >= min && src <= max { 1 } else { 0 };
        material::set_int(out, v);
        #[cfg(debug_assertions)]
        {
            if v != self.last_result {
                let mat_name = material::get_name(material).unwrap_or_else(|| "?".into());
                log(&format!(
                    "is_in_range[{}]: {}={src:.4} in [{}={min:.4},{}={max:.4}] -> {}={v}",
                    mat_name, self.src, self.min, self.max, self.result
                ));
                self.last_result = v;
            }
        }
        true
    }
}

/// l4nrp_print_variable —— 每帧读取并打印变量值（`type` 可配置：float / int / vector / string）。
///
/// 数值（float/int）、向量（vector）、字符串（string）分开处理：读取函数与日志格式各自独立。
#[derive(Clone, Copy, PartialEq)]
enum VarType {
    Float,
    Int,
    Vector,
    String,
}
impl VarType {
    fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "int" | "integer" => VarType::Int,
            "vector" | "vec3" | "vec" => VarType::Vector,
            "string" | "str" | "text" => VarType::String,
            _ => VarType::String,
        }
    }
}

struct PrintVariable {
    var: String,
    var_type: VarType,
    last_float: f32,
    last_int: i32,
    last_vec3: [f32; 3],
    last_str: String,
    var_n: CString,
}
impl Default for PrintVariable {
    fn default() -> Self {
        Self {
            var: "$var".into(),
            var_type: VarType::Float,
            last_float: f32::NEG_INFINITY,
            last_int: i32::MIN,
            last_vec3: [f32::NEG_INFINITY; 3],
            last_str: String::new(),
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
                let mut o = [f32::NEG_INFINITY; 3];
                material::get_vec(var, &mut o);
                if (o[0] - self.last_vec3[0]).abs() > EPS ||
                    (o[1] - self.last_vec3[1]).abs() > EPS ||
                    (o[2] - self.last_vec3[2]).abs() > EPS {
                    log(&format!(
                        "print_variable[{}]: {}={:.4},{:.4},{:.4} (vec)",
                        mat_name, self.var, o[0], o[1], o[2]
                    ));
                    self.last_vec3 = o;
                }
            }
            // ---- 整数：get_int 直接读整数值（vtable +0x68）----
            VarType::Int => {
                let v = material::get_int(var);
                if v != self.last_int {
                    log(&format!("print_variable[{}]: {}={v} (int)", mat_name, self.var));
                    self.last_int = v;
                }
            }
            // ---- 字符串：get_string 读字符串（vtable +0x18）----
            VarType::String => {
                let s = material::get_string(var).unwrap_or_else(|| "<null>".into());
                if s != self.last_str {
                    log(&format!("print_variable[{}]: {}='{s}' (str)", mat_name, self.var));
                    self.last_str = s;
                }
            }
            // ---- 标量浮点 ----
            VarType::Float => {
                let v = material::get_float(var);
                if (v - self.last_float).abs() > EPS {
                    log(&format!("print_variable[{}]: {}={v:.4} (float)", mat_name, self.var));
                    self.last_float = v;
                }
            }
        }
        true
    }
}

/// l4nrp_str_concat —— 拼接 2 个字符串变量（`src_a` + `src_b` → `result`）。
struct StrConcatProxy {
    src_a: String,
    src_b: String,
    result: String,
    last_result: CString,
    src_a_n: CString,
    src_b_n: CString,
    result_n: CString,
}
impl Default for StrConcatProxy {
    fn default() -> Self {
        Self {
            src_a: "$src_var_1".into(),
            src_b: "$src_var_2".into(),
            result: "$result_var".into(),
            last_result: cstr_of(""),
            src_a_n: cstr_of("$src_var_1"),
            src_b_n: cstr_of("$src_var_2"),
            result_n: cstr_of("$result_var"),
        }
    }
}
impl Proxy for StrConcatProxy {
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
        let a = material::get_string(material::find_var(material, &self.src_a_n)).unwrap_or_default();
        let b = material::get_string(material::find_var(material, &self.src_b_n)).unwrap_or_default();
        let out = material::find_var(material, &self.result_n);
        if out.is_null() {
            // 材质失效/变量缺失 → 从活动表移除
            return false;
        }
        let v = format!("{a}{b}");
        let c = cstr_of(&v);
        material::set_string(out, &c);
        #[cfg(debug_assertions)]
        {
            if self.last_result.to_bytes() != v.as_bytes() {
                let mat_name = material::get_name(material).unwrap_or_else(|| "?".into());
                log(&format!(
                    "str_concat[{}]: {}='{a}' {}='{b}' -> {}='{v}'",
                    mat_name, self.src_a, self.src_b, self.result
                ));
                self.last_result = cstr_of(&v);
            }
        }
        true
    }
}

/// l4nrp_str_replace —— 把 `src` 字符串里所有 `search` 替换为 `replace` 并写入 `result`。
///
/// `search` / `replace` 若以 `$` 开头则当作变量名读取（`get_string`），否则当作字面字符串。
struct StrReplaceProxy {
    src: String,
    search: String,
    replace: String,
    result: String,
    last_result: CString,
    src_n: CString,
    search_n: CString,
    replace_n: CString,
    result_n: CString,
}
impl Default for StrReplaceProxy {
    fn default() -> Self {
        Self {
            src: "$src_var".into(),
            search: String::new(),
            replace: String::new(),
            result: "$result_var".into(),
            last_result: cstr_of(""),
            src_n: cstr_of("$src_var"),
            search_n: cstr_of(""),
            replace_n: cstr_of(""),
            result_n: cstr_of("$result_var"),
        }
    }
}
impl StrReplaceProxy {
    /// 参数值以 `$` 开头时当作变量名读取，否则当作字面字符串。
    unsafe fn read_str_or_var(&self, material: *mut c_void, val: &str, c: &CString) -> String {
        if val.starts_with('$') {
            material::get_string(material::find_var(material, c)).unwrap_or_default()
        } else {
            val.to_string()
        }
    }
}
impl Proxy for StrReplaceProxy {
    fn apply_kv(&mut self, name: &str, value: &str) {
        match name.to_ascii_lowercase().as_str() {
            "src" | "src_var" | "input" | "source" => set_kv(&mut self.src, &mut self.src_n, value),
            "search" | "find" | "from" | "needle" => set_kv(&mut self.search, &mut self.search_n, value),
            "replace" | "to" | "replacement" | "repl" => set_kv(&mut self.replace, &mut self.replace_n, value),
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
        let src = material::get_string(material::find_var(material, &self.src_n)).unwrap_or_default();
        let search = self.read_str_or_var(material, &self.search, &self.search_n);
        let replace = self.read_str_or_var(material, &self.replace, &self.replace_n);
        let out = material::find_var(material, &self.result_n);
        if out.is_null() {
            // 材质失效/变量缺失 → 从活动表移除
            return false;
        }
        let v = src.replace(&search, &replace);
        let c = cstr_of(&v);
        material::set_string(out, &c);
        #[cfg(debug_assertions)]
        {
            if self.last_result.to_bytes() != v.as_bytes() {
                let mat_name = material::get_name(material).unwrap_or_else(|| "?".into());
                log(&format!(
                    "str_replace[{}]: {}='{src}' '{}'->'{}' -> {}='{v}'",
                    mat_name, self.src, self.search, self.replace, self.result
                ));
                self.last_result = cstr_of(&v);
            }
        }
        true
    }
}

/// l4nrp_vec3 —— 将 3 个浮点数变量作为分量组成 3 维向量。
struct Vec3Proxy {
    src_x: String,
    src_y: String,
    src_z: String,
    result: String,
    last_result: [f32; 3],
    src_x_n: CString,
    src_y_n: CString,
    src_z_n: CString,
    result_n: CString,
}
impl Default for Vec3Proxy {
    fn default() -> Self {
        Self {
            src_x: "$src_x".into(),
            src_y: "$src_y".into(),
            src_z: "$src_z".into(),
            result: "$result_var".into(),
            last_result: [f32::NEG_INFINITY; 3],
            src_x_n: cstr_of(""),
            src_y_n: cstr_of(""),
            src_z_n: cstr_of(""),
            result_n: cstr_of("$result_var"),
        }
    }
}
impl Proxy for Vec3Proxy {
    fn apply_kv(&mut self, name: &str, value: &str) {
        match name.to_ascii_lowercase().as_str() {
            "src_x" | "src_var_x" | "src_1" | "src_var_1" | "x" => set_kv(&mut self.src_x, &mut self.src_x_n, value),
            "src_y" | "src_var_y" | "src_2" | "src_var_2" | "y" => set_kv(&mut self.src_y, &mut self.src_y_n, value),
            "src_z" | "src_var_z" | "src_3" | "src_var_3" | "z" => set_kv(&mut self.src_z, &mut self.src_z_n, value),
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
        let x = material::get_float(material::find_var(material, &self.src_x_n));
        let y = material::get_float(material::find_var(material, &self.src_y_n));
        let z = material::get_float(material::find_var(material, &self.src_z_n));
        let out = material::find_var(material, &self.result_n);
        if out.is_null() {
            // 材质失效/变量缺失 → 从活动表移除
            return false;
        }
        let o = [x, y, z];
        material::set_vec(out, &o);
        #[cfg(debug_assertions)]
        {
            if (o[0] - self.last_result[0]).abs() > EPS ||
                (o[1] - self.last_result[1]).abs() > EPS ||
                (o[2] - self.last_result[2]).abs() > EPS {
                let mat_name = material::get_name(material).unwrap_or_else(|| "?".into());
                log(&format!(
                    "vec3[{}]: {}={x:.4} {}={y:.4} {}={z:.4} -> {}={x:.4},{y:.4},{z:.4}",
                    mat_name, self.src_x, self.src_y, self.src_z, self.result
                ));
                self.last_result = o;
            }
        }
        true
    }
}

/// l4nrp_math —— 计算数学表达式，支持读取材质已定义的 VMT 变量（`$var` 或 `var`）。
///
/// 表达式由 [`expr`](expr.rs) 求值器解析：运算符 `+ - * / % ^`、括号、一元负号，以及常用函数
/// （`sin/cos/tan/asin/acos/atan/atan2/sqrt/cbrt/abs/floor/ceil/round/sign/min/max/clamp/pow/
/// exp/ln/log/log10/fmod/lerp/pi`）。表达式里的 `$name` 或 `name` 会经 `get_float` 读取该材质
/// 已声明的 VMT 变量（未定义变量按 0.0 处理）。结果写入 `result` 变量（每帧）。
struct MathProxy {
    expr: String,
    result: String,
    last_result: f32,
    result_n: CString,
}
impl Default for MathProxy {
    fn default() -> Self {
        Self {
            expr: "0".into(),
            result: "$result_var".into(),
            last_result: f32::NEG_INFINITY,
            result_n: cstr_of("$result_var"),
        }
    }
}
impl Proxy for MathProxy {
    fn apply_kv(&mut self, name: &str, value: &str) {
        match name.to_ascii_lowercase().as_str() {
            "expr" | "expression" | "formula" | "calc" | "math" => self.expr = value.to_string(),
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
        let out = material::find_var(material, &self.result_n);
        if out.is_null() {
            // 材质失效/变量缺失 → 从活动表移除
            return false;
        }
        // 变量解析：表达式里的 `name` → 读材质 `$name` 变量
        let mut resolve = |name: &str| -> f32 {
            let full = format!("${name}");
            let c = cstr_of(&full);
            unsafe { material::get_float(material::find_var(material, &c)) }
        };
        match expr::eval(&self.expr, &mut resolve) {
            Ok(v) => {
                material::set_float(out, v);
                #[cfg(debug_assertions)]
                {
                    if (v - self.last_result).abs() > EPS {
                        let mat_name = material::get_name(material).unwrap_or_else(|| "?".into());
                        log(&format!(
                            "math[{}]: {}='{}' = {}",
                            mat_name, self.result, self.expr, v
                        ));
                        self.last_result = v;
                    }
                }
            }
            Err(e) => {
                log(&format!("math: expr '{}' error: {}", self.expr, e.0));
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
    material::register_proxy::<StrConcatProxy>("l4nrp_str_concat");
    material::register_proxy::<StrReplaceProxy>("l4nrp_str_replace");
    material::register_proxy::<Vec3Proxy>("l4nrp_vec3");
    material::register_proxy::<MathProxy>("l4nrp_math");
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
