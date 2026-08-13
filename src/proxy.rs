use core::ffi::c_void;
use std::ffi::CString;

use crate::kv::Proxy;
use crate::{material, expr};
use crate::util::{EPS, RelativeCompare};
use crate::log;

/// 从 &str 构造 CString。
fn cstr_of(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("").unwrap())
}

/// 同步更新变量名 `String` 与其缓存的 `CString`（apply_kv 使用）。
fn set_kv(dst: &mut String, c: &mut CString, value: &str) {
    *dst = value.to_string();
    *c = cstr_of(value);
}

/// l4nrp_does_equal —— 读两个输入变量（变量名可用参数配置），相等则输出 1 否则 0。
pub struct DoesEqualProxy {
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
pub struct CompareProxy {
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
pub struct IsInRangeProxy {
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

pub struct PrintVariable {
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

/// l4nrp_str_slice —— 从变量 `src` 的 `src_start` 开始，截取长度 `src_len` 的字符串切片并写入 `result`。
pub struct StrSliceProxy {
    src: String,
    src_start: String,
    src_len: String,
    result: String,
    last_result: CString,
    src_n: CString,
    src_start_n: CString,
    src_len_n: CString,
    result_n: CString,
}
impl Default for StrSliceProxy {
    fn default() -> Self {
        Self {
            src: "$src_var".into(),
            src_start: "$src_start".into(),
            src_len: "$src_len".into(),
            result: "$result_var".into(),
            last_result: cstr_of(""),
            src_n: cstr_of("$src_var"),
            src_start_n: cstr_of("$src_start"),
            src_len_n: cstr_of("$src_len"),
            result_n: cstr_of("$result_var"),
        }
    }
}
impl Proxy for StrSliceProxy {
    fn apply_kv(&mut self, name: &str, value: &str) {
        match name.to_ascii_lowercase().as_str() {
            "src" | "src_var" | "input" | "source" => set_kv(&mut self.src, &mut self.src_n, value),
            "src_start" | "start" | "offset" | "begin" => set_kv(&mut self.src_start, &mut self.src_start_n, value),
            "src_len" | "len" | "length" | "count" | "size" => set_kv(&mut self.src_len, &mut self.src_len_n, value),
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
        let s = material::get_string(material::find_var(material, &self.src_n)).unwrap_or_default();
        // src_start / src_len 为整型变量：起始位置 / 截取长度
        let start = material::get_int(material::find_var(material, &self.src_start_n));
        let len = material::get_int(material::find_var(material, &self.src_len_n));
        let out = material::find_var(material, &self.result_n);
        if out.is_null() {
            // 材质失效/变量缺失 → 从活动表移除
            return false;
        }
        // 按字符（而非字节）切片，避免 UTF-8 边界 panic；越界 clamp 到合法范围
        let chars: Vec<char> = s.chars().collect();
        let n = chars.len() as i32;
        let start = start.clamp(0, n);
        let len = len.clamp(0, n - start);
        let v: String = chars[start as usize..(start + len) as usize].iter().collect();
        let c = cstr_of(&v);
        material::set_string(out, &c);
        #[cfg(debug_assertions)]
        {
            if self.last_result.to_bytes() != v.as_bytes() {
                let mat_name = material::get_name(material).unwrap_or_else(|| "?".into());
                log(&format!(
                    "str_slice[{}]: {}='{s}' start={start} len={len} -> {}='{v}'",
                    mat_name, self.src, self.result
                ));
                self.last_result = cstr_of(&v);
            }
        }
        true
    }
}

/// l4nrp_vmt_name —— 把材质自身文件名（`material::get_name`，即 VMT 去掉 `.vmt` 后的路径）
/// 写入 `result` 变量（字符串类型）。材质名不会变化，仅材质加载时执行一次。
pub struct VmtName {
    result: String,
    last_result: CString,
    result_n: CString,
}
impl Default for VmtName {
    fn default() -> Self {
        Self {
            result: "$result_var".into(),
            last_result: cstr_of(""),
            result_n: cstr_of("$result_var"),
        }
    }
}
impl Proxy for VmtName {
    fn apply_kv(&mut self, name: &str, value: &str) {
        match name.to_ascii_lowercase().as_str() {
            "result" | "result_var" | "output" => set_kv(&mut self.result, &mut self.result_n, value),
            _ => {}
        }
    }
    unsafe fn bind(&mut self, material: *mut c_void) -> bool {
        if material.is_null() {
            return false;
        }
        let out = material::find_var(material, &self.result_n);
        if out.is_null() {
            return false;
        }
        let name = material::get_name(material).unwrap_or_default();
        let c = cstr_of(&name);
        material::set_string(out, &c);
        #[cfg(debug_assertions)]
        {
            if self.last_result.to_bytes() != name.as_bytes() {
                log(&format!(
                    "vmt_name[{}]: {}='{name}'",
                    name, self.result
                ));
                self.last_result = cstr_of(&name);
            }
        }
        true
    }
}

/// l4nrp_str_concat —— 拼接 2 个字符串变量（`src_a` + `src_b` → `result`）。
pub struct StrConcatProxy {
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
pub struct StrReplaceProxy {
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
pub struct Vec3Proxy {
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
pub struct MathProxy {
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
        match expr::eval_math(&self.expr, &mut resolve) {
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

/// l4nrp_logic —— 每帧求值逻辑/布尔表达式（比较 `== != < <= > >=` 与逻辑 `&& || !`，
/// 非 0 视为真，另有 `in_range` / `in_range_exclusively` 范围函数；见 [`expr.rs`](src/expr.rs)
/// `eval_logic`），结果写 `result`（整型 0/1，set_int）。
/// 表达式里的 `$name` 或 `name` 经 `get_float` 读取该材质已声明变量（未定义按 0.0）。
pub struct LogicProxy {
    expr: String,
    result: String,
    last_result: i32,
    result_n: CString,
}
impl Default for LogicProxy {
    fn default() -> Self {
        Self {
            expr: "0".into(),
            result: "$result_var".into(),
            last_result: -1,
            result_n: cstr_of("$result_var"),
        }
    }
}
impl Proxy for LogicProxy {
    fn apply_kv(&mut self, name: &str, value: &str) {
        match name.to_ascii_lowercase().as_str() {
            "expr" | "expression" | "condition" | "logic" | "bool" | "test" => {
                self.expr = value.to_string()
            }
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
        match expr::eval_logic(&self.expr, &mut resolve) {
            Ok(v) => {
                let r = if v != 0.0 { 1 } else { 0 };
                material::set_int(out, r);
                #[cfg(debug_assertions)]
                {
                    if r != self.last_result {
                        log(&format!(
                            "logic[{}]: '{}' = {}",
                            material::get_name(material).unwrap_or_else(|| "?".into()),
                            self.expr,
                            r
                        ));
                        self.last_result = r;
                    }
                }
            }
            Err(e) => {
                log(&format!("logic: expr '{}' error: {}", self.expr, e.0));
            }
        }
        true
    }
}

/// l4nrp_delay_set —— 检测 `trigger` 变量（整型）非 0 的**上升沿**后，启动一个计时器，
/// 延迟 `delay` 毫秒后把 `value` 变量（整型）的值复制到 `output`；`handle`（可选）写出本次
/// 计时器的 **UUID v4 字符串手柄**（无计时器时写空字符串），供其它代理（如 `l4nrp_delay_abort`）中断。
///
/// 计时器由 [`material.rs`](material.rs) 的全局注册表托管：到期由 `run_timers` 每帧触发
/// （先取出再释放锁，见 AGENTS.md）。`trigger` 未先回到 0 再置位前不会重复触发。
pub struct DelaySetProxy {
    trigger: String,
    delay_ms: u64,
    output: String,
    value: String, // 变量名：到期后读取其整型值并写入 output
    handle: String, // 可选；空 = 不写手柄变量
    last_trigger: i32,
    current: String, // 当前计时器 UUID 手柄；空 = 无
    trigger_n: CString,
    output_n: CString,
    value_n: CString,
    handle_n: CString,
}
impl Default for DelaySetProxy {
    fn default() -> Self {
        Self {
            trigger: "$trigger_var".into(),
            delay_ms: 1000,
            output: "$result_var".into(),
            value: "$value_var".into(),
            handle: String::new(),
            last_trigger: 0,
            current: String::new(),
            trigger_n: cstr_of("$trigger_var"),
            output_n: cstr_of("$result_var"),
            value_n: cstr_of("$value_var"),
            handle_n: cstr_of(""),
        }
    }
}
impl DelaySetProxy {
    /// 写手柄状态变量（可选，未配置则跳过）。UUID 为字符串，写入字符串类型变量。
    fn write_handle(&self, material: *mut c_void, h: &str) {
        if self.handle.is_empty() {
            return;
        }
        let c = cstr_of(h);
        unsafe {
            let v = material::find_var(material, &self.handle_n);
            if !v.is_null() {
                material::set_string(v, &c);
            }
        }
    }
}
impl Proxy for DelaySetProxy {
    fn apply_kv(&mut self, name: &str, value: &str) {
        match name.to_ascii_lowercase().as_str() {
            "trigger" | "input" | "src" | "condition" | "flag" => {
                set_kv(&mut self.trigger, &mut self.trigger_n, value)
            }
            "delay" | "delay_ms" | "time" | "ms" => {
                self.delay_ms = value.trim().parse().unwrap_or(self.delay_ms)
            }
            "output" | "result" | "result_var" | "target" => {
                set_kv(&mut self.output, &mut self.output_n, value)
            }
            "value" | "set" | "to" | "value_var" => {
                set_kv(&mut self.value, &mut self.value_n, value)
            }
            "handle" | "handle_var" | "timer_handle" | "running" | "busy" | "active" => {
                set_kv(&mut self.handle, &mut self.handle_n, value)
            }
            _ => {}
        }
    }
    // 依赖每帧时间推进，需要每帧执行
    fn per_frame(&self) -> bool {
        true
    }
    unsafe fn bind(&mut self, material: *mut c_void) -> bool {
        if material.is_null() {
            return false;
        }
        let out = material::find_var(material, &self.output_n);
        if out.is_null() {
            // 材质失效/变量缺失 → 从活动表移除，并清理挂起的计时器（防泄漏/悬垂）
            if !self.current.is_empty() {
                material::abort_timer(&self.current);
                self.current.clear();
            }
            return false;
        }
        let t = material::get_int(material::find_var(material, &self.trigger_n));

        // 同步手柄状态：运行中写回手柄，否则写空字符串
        if !self.current.is_empty() {
            if material::timer_active(&self.current) {
                self.write_handle(material, &self.current);
            } else {
                // 已到期（run_timers 已写 output）/ 被 l4nrp_delay_abort 中断
                self.current.clear();
                self.write_handle(material, "");
            }
        } else {
            self.write_handle(material, "");
        }

        // 上升沿启动新计时器（注册表托管，返回 UUID 手柄）
        if t != 0 && self.last_trigger == 0 && self.current.is_empty() {
            self.current = material::start_timer(
                material,
                self.output_n.clone(),
                self.value_n.clone(),
                self.delay_ms,
            );
            self.write_handle(material, &self.current);
            #[cfg(debug_assertions)]
            {
                log(&format!(
                    "delay_set: timer {} started ({}ms), trigger={}",
                    self.current, self.delay_ms, t
                ));
            }
        }
        self.last_trigger = t;
        true
    }
}

/// l4nrp_delay_abort —— 当 `trigger`（整型）非 0 时，中断 `handle` 变量指定的计时器
/// （该手柄由 `l4nrp_delay_set` 的 `handle` 参数写入）。
pub struct DelayAbortProxy {
    trigger: String,
    handle: String,
    trigger_n: CString,
    handle_n: CString,
}
impl Default for DelayAbortProxy {
    fn default() -> Self {
        Self {
            trigger: "$trigger_var".into(),
            handle: "$timer_handle".into(),
            trigger_n: cstr_of("$trigger_var"),
            handle_n: cstr_of("$timer_handle"),
        }
    }
}
impl Proxy for DelayAbortProxy {
    fn apply_kv(&mut self, name: &str, value: &str) {
        match name.to_ascii_lowercase().as_str() {
            "trigger" | "input" | "src" | "condition" | "flag" => {
                set_kv(&mut self.trigger, &mut self.trigger_n, value)
            }
            "handle" | "handle_var" | "timer" | "timer_handle" => {
                set_kv(&mut self.handle, &mut self.handle_n, value)
            }
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
        let t = material::get_int(material::find_var(material, &self.trigger_n));
        // handle 变量是字符串类型（UUID v4），用 get_string 读取
        let h = material::get_string(material::find_var(material, &self.handle_n));
        if t != 0 {
            if let Some(h) = h {
                if !h.is_empty() && material::abort_timer(&h) {
                    #[cfg(debug_assertions)]
                    {
                        log(&format!("delay_abort: aborted timer {h}"));
                    }
                }
            }
        }
        true
    }
}