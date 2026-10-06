use core::ffi::c_void;
use std::ffi::CString;

use crate::error::{MaterialError, PluginError};
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
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        let a = material::get_float(material::find_var(material, &self.src_a_n)?)?;
        let b = material::get_float(material::find_var(material, &self.src_b_n)?)?;
        let out = material::find_var(material, &self.result_n)?;

        let v = if (a - b).abs() < EPS { 1 } else { 0 };
        material::set_int(out, v)?;
        #[cfg(debug_assertions)]
        {
            if v != self.last_result {
                let mat_name = material::get_name(material).unwrap_or_else(|_| "?".into());
                log(&format!(
                    "does_equal[{}]: {}={a:.4} {}={b:.4} -> {}={v}",
                    mat_name, self.src_a, self.src_b, self.result
                ));
                self.last_result = v;
            }
        }

        Ok(())
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
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        let a = material::get_float(material::find_var(material, &self.src_a_n)?)?;
        let b = material::get_float(material::find_var(material, &self.src_b_n)?)?;
        let out = material::find_var(material, &self.result_n)?;
        let v = match a.relative_cmp(&b) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1
        };
        material::set_int(out, v)?;
        #[cfg(debug_assertions)]
        {
            if v != self.last_result {
                let mat_name = material::get_name(material).unwrap_or_else(|_| "?".into());
                log(&format!(
                    "compare[{}]: {}={a:.4} {}={b:.4} -> {}={v}",
                    mat_name, self.src_a, self.src_b, self.result
                ));
                self.last_result = v;
            }
        }
        Ok(())
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
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        let src = material::get_float(material::find_var(material, &self.src_n)?)?;
        let min = material::get_float(material::find_var(material, &self.min_n)?)?;
        let max = material::get_float(material::find_var(material, &self.max_n)?)?;
        let out = material::find_var(material, &self.result_n)?;
        let v = if src >= min && src <= max { 1 } else { 0 };
        material::set_int(out, v)?;
        #[cfg(debug_assertions)]
        {
            if v != self.last_result {
                let mat_name = material::get_name(material).unwrap_or_else(|_| "?".into());
                log(&format!(
                    "is_in_range[{}]: {}={src:.4} in [{}={min:.4},{}={max:.4}] -> {}={v}",
                    mat_name, self.src, self.min, self.max, self.result
                ));
                self.last_result = v;
            }
        }
        Ok(())
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
            "float" | "f32" | "number" | "double" => VarType::Float,
            "int" | "integer" => VarType::Int,
            "vector" | "vec3" | "vec" => VarType::Vector,
            "string" | "str" | "text" => VarType::String,
            // 未识别的写法按默认值（Float）处理，与不写 `type` 时一致；
            // 早先这里返回 String，导致显式写 `type "float"` 反而按字符串读取。
            _ => VarType::Float,
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
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        let var = material::find_var(material, &self.var_n)?;
        let mat_name = material::get_name(material).unwrap_or_else(|_| "?".into());
        match self.var_type {
            // ---- 向量：get_vec 读三分量 ----
            VarType::Vector => {
                let mut o = [f32::NEG_INFINITY; 3];
                material::get_vec(var, &mut o)?;
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
                let v = material::get_int(var)?;
                if v != self.last_int {
                    log(&format!("print_variable[{}]: {}={v} (int)", mat_name, self.var));
                    self.last_int = v;
                }
            }
            // ---- 字符串：get_string 读字符串（vtable +0x18）----
            VarType::String => {
                let s = material::get_string(var).unwrap_or_else(|_| "<null>".into());
                if s != self.last_str {
                    log(&format!("print_variable[{}]: {}='{s}' (str)", mat_name, self.var));
                    self.last_str = s;
                }
            }
            // ---- 标量浮点 ----
            VarType::Float => {
                let v = material::get_float(var)?;
                if (v - self.last_float).abs() > EPS {
                    log(&format!("print_variable[{}]: {}={v:.4} (float)", mat_name, self.var));
                    self.last_float = v;
                }
            }
        }
        Ok(())
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
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        let s = material::get_string(material::find_var(material, &self.src_n)?).unwrap_or_default();
        // src_start / src_len 为整型变量：起始位置 / 截取长度
        let start = material::get_int(material::find_var(material, &self.src_start_n)?)?;
        let len = material::get_int(material::find_var(material, &self.src_len_n)?)?;
        let out = material::find_var(material, &self.result_n)?;
        // 按字符（而非字节）切片，避免 UTF-8 边界 panic；越界 clamp 到合法范围
        let chars: Vec<char> = s.chars().collect();
        let n = chars.len() as i32;
        let start = start.clamp(0, n);
        let len = len.clamp(0, n - start);
        let v: String = chars[start as usize..(start + len) as usize].iter().collect();
        let c = cstr_of(&v);
        material::set_string(out, &c)?;
        #[cfg(debug_assertions)]
        {
            if self.last_result.to_bytes() != v.as_bytes() {
                let mat_name = material::get_name(material).unwrap_or_else(|_| "?".into());
                log(&format!(
                    "str_slice[{}]: {}='{s}' start={start} len={len} -> {}='{v}'",
                    mat_name, self.src, self.result
                ));
                self.last_result = cstr_of(&v);
            }
        }
        Ok(())
    }
}

/// l4nrp_str_len —— 读取 `src` 字符串变量，计算其字符长度（按字符，非字节）并写入 `result`（整型）。
pub struct StrLenProxy {
    src: String,
    result: String,
    last_result: i32,
    src_n: CString,
    result_n: CString,
}
impl Default for StrLenProxy {
    fn default() -> Self {
        Self {
            src: "$src_var".into(),
            result: "$result_var".into(),
            last_result: -1,
            src_n: cstr_of("$src_var"),
            result_n: cstr_of("$result_var"),
        }
    }
}
impl Proxy for StrLenProxy {
    fn apply_kv(&mut self, name: &str, value: &str) {
        match name.to_ascii_lowercase().as_str() {
            "src" | "src_var" | "input" | "source" => set_kv(&mut self.src, &mut self.src_n, value),
            "result" | "result_var" | "output" => set_kv(&mut self.result, &mut self.result_n, value),
            _ => {}
        }
    }
    // 依赖每帧变化的输入，需要每帧重算
    fn per_frame(&self) -> bool {
        true
    }
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        let s = material::get_string(material::find_var(material, &self.src_n)?).unwrap_or_default();
        let out = material::find_var(material, &self.result_n)?;
        // 按字符（而非字节）计数，避免 UTF-8 边界问题
        let len = s.chars().count() as i32;
        material::set_int(out, len)?;
        #[cfg(debug_assertions)]
        {
            if len != self.last_result {
                let mat_name = material::get_name(material).unwrap_or_else(|_| "?".into());
                log(&format!(
                    "str_len[{}]: {}='{s}' -> {}={len}",
                    mat_name, self.src, self.result
                ));
                self.last_result = len;
            }
        }
        Ok(())
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
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        let out = material::find_var(material, &self.result_n)?;
        let name = material::get_name(material).unwrap_or_default();
        let c = cstr_of(&name);
        material::set_string(out, &c)?;
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
        Ok(())
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
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        let a = material::get_string(material::find_var(material, &self.src_a_n)?).unwrap_or_default();
        let b = material::get_string(material::find_var(material, &self.src_b_n)?).unwrap_or_default();
        let out = material::find_var(material, &self.result_n)?;
        let v = format!("{a}{b}");
        let c = cstr_of(&v);
        material::set_string(out, &c)?;
        #[cfg(debug_assertions)]
        {
            if self.last_result.to_bytes() != v.as_bytes() {
                let mat_name = material::get_name(material).unwrap_or_else(|_| "?".into());
                log(&format!(
                    "str_concat[{}]: {}='{a}' {}='{b}' -> {}='{v}'",
                    mat_name, self.src_a, self.src_b, self.result
                ));
                self.last_result = cstr_of(&v);
            }
        }
        Ok(())
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
            material::find_var(material, c)
                .ok()
                .and_then(|v| material::get_string(v).ok())
                .unwrap_or_default()
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
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        let src = material::get_string(material::find_var(material, &self.src_n)?).unwrap_or_default();
        let search = self.read_str_or_var(material, &self.search, &self.search_n);
        let replace = self.read_str_or_var(material, &self.replace, &self.replace_n);
        let out = material::find_var(material, &self.result_n)?;
        let v = src.replace(&search, &replace);
        let c = cstr_of(&v);
        material::set_string(out, &c)?;
        #[cfg(debug_assertions)]
        {
            if self.last_result.to_bytes() != v.as_bytes() {
                let mat_name = material::get_name(material).unwrap_or_else(|_| "?".into());
                log(&format!(
                    "str_replace[{}]: {}='{src}' '{}'->'{}' -> {}='{v}'",
                    mat_name, self.src, self.search, self.replace, self.result
                ));
                self.last_result = cstr_of(&v);
            }
        }
        Ok(())
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
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        let x = material::get_float(material::find_var(material, &self.src_x_n)?)?;
        let y = material::get_float(material::find_var(material, &self.src_y_n)?)?;
        let z = material::get_float(material::find_var(material, &self.src_z_n)?)?;
        let out = material::find_var(material, &self.result_n)?;
        let o = [x, y, z];
        material::set_vec(out, &o)?;
        #[cfg(debug_assertions)]
        {
            if (o[0] - self.last_result[0]).abs() > EPS ||
                (o[1] - self.last_result[1]).abs() > EPS ||
                (o[2] - self.last_result[2]).abs() > EPS {
                let mat_name = material::get_name(material).unwrap_or_else(|_| "?".into());
                log(&format!(
                    "vec3[{}]: {}={x:.4} {}={y:.4} {}={z:.4} -> {}={x:.4},{y:.4},{z:.4}",
                    mat_name, self.src_x, self.src_y, self.src_z, self.result
                ));
                self.last_result = o;
            }
        }
        Ok(())
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
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        let out = material::find_var(material, &self.result_n)?;
        // 变量解析：表达式里的 `name` → 读材质 `$name` 变量（未定义/读取失败按 0.0）
        let mut resolve = |name: &str| -> f32 {
            let full = format!("${name}");
            let c = cstr_of(&full);
            match unsafe { material::find_var(material, &c) } {
                Ok(v) => unsafe { material::get_float(v) }.unwrap_or(0.0),
                Err(_) => 0.0,
            }
        };
        match expr::eval_math(&self.expr, &mut resolve) {
            Ok(v) => {
                material::set_float(out, v)?;
                #[cfg(debug_assertions)]
                {
                    if (v - self.last_result).abs() > EPS {
                        let mat_name = material::get_name(material).unwrap_or_else(|_| "?".into());
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
        Ok(())
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
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        let out = material::find_var(material, &self.result_n)?;
        // 变量解析：表达式里的 `name` → 读材质 `$name` 变量（未定义/读取失败按 0.0）
        let mut resolve = |name: &str| -> f32 {
            let full = format!("${name}");
            let c = cstr_of(&full);
            match unsafe { material::find_var(material, &c) } {
                Ok(v) => unsafe { material::get_float(v) }.unwrap_or(0.0),
                Err(_) => 0.0,
            }
        };
        match expr::eval_logic(&self.expr, &mut resolve) {
            Ok(v) => {
                let r = if v != 0.0 { 1 } else { 0 };
                material::set_int(out, r)?;
                #[cfg(debug_assertions)]
                {
                    if r != self.last_result {
                        log(&format!(
                            "logic[{}]: '{}' = {}",
                            material::get_name(material).unwrap_or_else(|_| "?".into()),
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
        Ok(())
    }
}

/// l4nrp_delay_set —— 检测 `trigger` 变量（整型）非 0 的**上升沿**后，启动一个计时器，
/// 延迟 `delay` 毫秒后把 `value` 变量（整型）的**触发时快照值**写入 `output`；`handle`（可选）写出本次
/// 计时器的 **UUID v4 字符串手柄**（无计时器时写空字符串），供其它代理（如 `l4nrp_delay_abort`）中断。
///
/// 计时器由 [`material.rs`](material.rs) 的全局注册表托管：到期由 `run_timers` 每帧触发
/// （先取出再释放锁，见 AGENTS.md）。`value` 在**触发（上升沿）那一刻**被读取并快照，延迟期间源
/// 变量变化不影响本次输出。`trigger` 未先回到 0 再置位前不会重复触发。
pub struct DelaySetProxy {
    trigger: String,
    delay_ms: u64,
    output: String,
    value: String, // 变量名：触发时读取其整型值作为快照
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
            if let Ok(v) = material::find_var(material, &self.handle_n) {
                let _ = material::set_string(v, &c);
            }
        }
    }

    /// 统一清理挂起的计时器（任何失败路径 / 被销毁时都会走这里，防止全局注册表残留）。
    /// 注意：清理时不访问材质（可能在销毁路径上已失效/悬垂），只操作自己的手柄。
    fn abort_current(&mut self) {
        if !self.current.is_empty() {
            material::abort_timer(&self.current);
            self.current.clear();
        }
    }
}
impl Drop for DelaySetProxy {
    fn drop(&mut self) {
        // 代理离开活动表/被销毁（bind 返回 Err 被移除、材质失效、插件卸载）时，
        // 确保取消挂起的计时器，防止其在全局 TIMERS 表里残留到期写坏内存。
        self.abort_current();
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
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        if let Err(e) = material::find_var(material, &self.output_n) {
            // 材质失效/变量缺失 → 从活动表移除，并清理挂起的计时器（统一走 abort_current）
            self.abort_current();
            return Err(e);
        }
        let t = material::get_int(material::find_var(material, &self.trigger_n)?)?;

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

        // 上升沿启动新计时器：触发时刻读取 value 变量作为**快照**写入计时器，
        // 到期后 run_timers 直接写快照（延迟期间 value 再变不影响本次输出）。
        if t != 0 && self.last_trigger == 0 && self.current.is_empty() {
            let v = material::get_int(material::find_var(material, &self.value_n)?)?;
            self.current = material::start_timer(material, self.output_n.clone(), v, self.delay_ms);
            self.write_handle(material, &self.current);
            #[cfg(debug_assertions)]
            {
                log(&format!(
                    "delay_set: timer {} started ({}ms), value={}, trigger={}",
                    self.current, self.delay_ms, v, t
                ));
            }
        }
        self.last_trigger = t;
        Ok(())
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
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        let t = material::get_int(material::find_var(material, &self.trigger_n)?)?;
        // handle 变量是字符串类型（UUID v4），用 get_string 读取
        let h = material::get_string(material::find_var(material, &self.handle_n)?).ok();
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
        Ok(())
    }
}

// ---------- l4nrp_random 的辅助类型 / 函数 ----------

/// 数值参数：既可以是字面量（`"500"`），也可以是材质变量名（`"$max_frame"`）。
///
/// 其它代理把 `min`/`max` 一律当变量名，写 `"min" "0"` 会查不到变量而报错；
/// 本代理先尝试按数字解析，解析不了才当变量名，因此两种写法都能用。
struct NumParam {
    name: String,
    c: CString,
    lit: Option<f32>,
}
impl NumParam {
    fn new(default: &str) -> Self {
        Self {
            name: default.to_string(),
            c: cstr_of(default),
            lit: default.trim().parse::<f32>().ok(),
        }
    }
    fn set(&mut self, value: &str) {
        self.lit = value.trim().parse::<f32>().ok();
        set_kv(&mut self.name, &mut self.c, value);
    }
    /// 解析当前值：字面量直接用；否则读材质变量，读不到返回 `fallback`。
    ///
    /// # Safety
    /// `material` 必须是有效的 `IMaterial*`。
    unsafe fn resolve(&self, material: *mut c_void, fallback: f32) -> f32 {
        if let Some(v) = self.lit {
            return v;
        }
        read_number(material, &self.c).unwrap_or(fallback)
    }
}

/// 把 `[0,1)` 的随机比例映射到 `[lo, hi]`；`integer` 为真时向下取整。
fn scale_unit(unit: f32, lo: f32, hi: f32, integer: bool) -> f32 {
    let v = lo + (hi - lo) * unit;
    if integer { v.floor() } else { v }
}

/// 读取材质变量的数值（浮点优先，整数兜底）。
///
/// 实测（materialsystem.dll `CMaterialVar`）：`SetFloatValue`(+0x0c) 与 `SetIntValue`(+0x10)
/// **都会同时同步两个字段** —— 前者 `MOVSS [this+0x0c]` 后再 `CVTTSS2SI` 写 `[this+0x08]`，
/// 后者 `MOV [this+0x08]` 后再 `CVTSI2SS` 写 `[this+0x0c]`。因此无论 VMT 里写 `"0"`（整型）
/// 还是 `"0.0"`（浮点），`GetFloatValue`(+0x6c) 都能读到正确数值，整数兜底实际只在
/// 变量类型异常时兜底。
///
/// # Safety
/// `material` 必须是有效的 `IMaterial*`。
unsafe fn read_number(material: *mut c_void, name: &CString) -> Option<f32> {
    let v = material::find_var(material, name).ok()?;
    if let Ok(f) = material::get_float(v) {
        return Some(f);
    }
    material::get_int(v).ok().map(|i| i as f32)
}

/// VMT 里的布尔值：`1` / `true` / `yes` / `on` 视为真（不区分大小写）。
fn kv_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// 进程级共享盐：整个游戏运行期间固定，用于 `$oneSkinPerMap` 语义。
///
/// 每个材质实例都用它播种，于是同一帧/同一地图内所有实例得到相同的随机比例。
/// 惰性初始化（首次 `reseed` 时取一次系统时间）。
fn shared_salt() -> u32 {
    static SALT: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *SALT.get_or_init(|| {
        let n = now_nanos();
        if n == 0 { 0x5BF0_3635 } else { n }
    })
}

/// 当前系统时间（纳秒与秒混合），作为熵来源。
fn now_nanos() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() ^ d.as_secs() as u32)
        .unwrap_or(0)
}

/// 为 `l4nrp_random` 播种：混合系统时间、材质实例地址、额外熵与重掷计数。
///
/// `material` 只当熵用（`addr()` 不解引用），因此静态物体（地图物件）也能拿到
/// 稳定的、每个实例不同的种子。`shared` 为真时**只用进程级共享盐**（见 [`shared_salt`]）：
/// 不掺时间、地址与计数，于是同一次游戏运行内所有实例、无论何时重掷，都得到同一个种子
/// → 同一个随机比例 → 同一张皮肤，即指南里的 `$oneSkinPerMap`。
fn reseed(material: *mut c_void, extra: u32, counter: u32, shared: bool) -> u64 {
    if shared {
        // 共享模式：只用进程级盐，**不掺**时间/地址/计数/额外熵 ——
        // 否则先重掷与后重掷的实例会分叉，`$oneSkinPerMap` 失效。
        return shared_salt() as u64;
    }
    let entropy =
        (now_nanos() as u64) ^ (material.addr() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    entropy.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (extra as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (counter as u64).wrapping_mul(0x1656_67B1_9E37_79F9)
}

/// l4nrp_random —— 稳定的伪随机数：按材质实例播种一次，之后只在 `trigger` 变化时重掷。
///
/// 与引擎自带的 `EntityRandom` 相比：
/// - **静态物体也能用**：不依赖实体，种子来自材质实例地址 + 系统时间 + 可选 `seed`；
/// - **帧间稳定**：只在播种/重掷时更新，不会每帧乱跳；
/// - **`min`/`max` 每帧重新读取**：上界变量（如探测出来的帧数）变大时结果按比例跟随，
///   不需要原版 RNG 代码里那一长串溢出/钳制代理。
///
/// 参数（键名不区分大小写）：
/// - `min` / `max`：取值范围，字面量或变量名，默认 `0` / `1`；`max < min` 时自动交换。
/// - `result`：输出变量，默认 `$random_result`。
/// - `gate`（可选）：门控变量，为 0 时本代理完全不动作（不重掷也不写出）。
///   典型用法是接「帧数探测完成」标志，等上界稳定后再选皮肤，避免加载瞬间连跳几帧。
/// - `trigger`（可选）：整型触发器变量，其值**发生变化**时重掷。
/// - `seed`（可选）：额外熵来源（浮点变量），重掷时混入。
/// - `unit`（可选）：外部 `[0,1)` 随机源变量（如 `EntityRandom`）。给了就用它当随机比例，
///   不再用内部 PRNG —— 需要「每个实体一个皮肤」时用它（`EntityRandom` 对同一实体稳定）。
/// - `shared`（可选）：`1` = 整个地图所有实例共用同一个结果（对应指南的 `$oneSkinPerMap`）。
/// - `integer`（可选）：`1` = 结果向下取整。默认仍写浮点（`$frame` 这类着色器变量是浮点，
///   取整只是为了落在整数帧上）。
/// - `write_int`（可选）：`1` = 用 `set_int` 写出（目标是真正的整型变量时用）。
pub struct RandomProxy {
    min: NumParam,
    max: NumParam,
    result: String,
    gate: String,
    trigger: String,
    seed: String,
    unit_var: String,
    integer: bool,
    write_int: bool,
    /// `$oneSkinPerMap` 语义：整个地图所有实例共用同一个随机结果
    shared: String,
    shared_n: CString,
    /// `shared` 写成字面量时直接定值；否则每帧读 `shared_n` 变量
    shared_lit: Option<bool>,
    result_n: CString,
    gate_n: CString,
    trigger_n: CString,
    seed_n: CString,
    unit_n: CString,
    /// PRNG（`rand::rngs::StdRng`，ChaCha12）：`None` = 尚未播种
    rng: Option<rand::rngs::StdRng>,
    /// 重掷计数，作为播种熵的一部分（保证每次重掷都换一个种子）
    reroll_count: u32,
    /// 当前随机比例 `[0,1)`：重掷时更新，每帧按当前 `[min,max]` 重新缩放
    unit: f32,
    last_trigger: i32,
    /// 上次写出的值，仅在 `debug_assertions` 下用于抑制重复日志
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    last_value: f32,
    last_shared: bool,
}
impl Default for RandomProxy {
    fn default() -> Self {
        Self {
            min: NumParam::new("0"),
            max: NumParam::new("1"),
            result: "$random_result".into(),
            gate: String::new(),
            trigger: String::new(),
            seed: String::new(),
            unit_var: String::new(),
            integer: false,
            write_int: false,
            shared: String::new(),
            shared_n: cstr_of(""),
            shared_lit: None,
            result_n: cstr_of("$random_result"),
            gate_n: cstr_of(""),
            trigger_n: cstr_of(""),
            seed_n: cstr_of(""),
            unit_n: cstr_of(""),
            rng: None,
            reroll_count: 0,
            unit: 0.0,
            last_trigger: 0,
            last_value: f32::NEG_INFINITY,
            last_shared: false,
        }
    }
}
impl RandomProxy {
    /// 用 `seed` 播种 `rand::rngs::StdRng`（ChaCha12），并取一个 `[0,1)` 随机比例。
    ///
    /// 采用 `rand` 而非自写 PRNG：算法经过充分验证、无自研风险；
    /// `StdRng` 是 `rand` 文档指定的「加密强度、跨版本稳定」选择。
    fn seed_and_roll(&mut self, seed: u64) {
        use rand::{RngExt, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        // `random::<f32>()` 落在 [0,1)
        self.unit = rng.random::<f32>();
        self.rng = Some(rng);
    }
}
impl Proxy for RandomProxy {
    fn apply_kv(&mut self, name: &str, value: &str) {
        match name.to_ascii_lowercase().as_str() {
            "min" | "min_var" | "from" | "low" => self.min.set(value),
            "max" | "max_var" | "to" | "high" => self.max.set(value),
            "result" | "result_var" | "output" | "target" => {
                set_kv(&mut self.result, &mut self.result_n, value)
            }
            "gate" | "enable" | "enabled" | "when" | "ready" => {
                set_kv(&mut self.gate, &mut self.gate_n, value)
            }
            "trigger" | "input" | "src" | "reroll" | "reroll_var" => {
                set_kv(&mut self.trigger, &mut self.trigger_n, value)
            }
            "seed" | "seed_var" | "entropy" | "salt" => {
                set_kv(&mut self.seed, &mut self.seed_n, value)
            }
            "unit" | "unit_var" | "source" | "random_source" => {
                set_kv(&mut self.unit_var, &mut self.unit_n, value)
            }
            "integer" | "int" | "floor" => self.integer = kv_truthy(value),
            "write_int" | "as_int" | "set_int" => self.write_int = kv_truthy(value),
            "shared" | "one_skin_per_map" | "global" | "same_for_all" => {
                // 允许写字面量（"1"）或变量名（"$oneSkinPerMap"）
                self.shared_lit = if value.trim().starts_with('$') {
                    None
                } else {
                    Some(kv_truthy(value))
                };
                set_kv(&mut self.shared, &mut self.shared_n, value);
            }
            _ => {}
        }
    }
    // 触发器/上下界都可能每帧变化
    fn per_frame(&self) -> bool {
        true
    }
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError> {
        if material.is_null() {
            return Err(PluginError::Material(MaterialError::InvalidMaterial));
        }
        let out = material::find_var(material, &self.result_n)?;

        // 门控：为 0 时完全不动作（保持上一次写出的值，避免加载瞬间连跳几帧）
        if !self.gate.is_empty() {
            match read_number(material, &self.gate_n) {
                Some(g) if g != 0.0 => {}
                _ => return Ok(()),
            }
        }

        // `shared` 可以是字面量或变量名；变量为 0 时按每实例随机
        let shared = match self.shared_lit {
            Some(v) => v,
            None if self.shared.is_empty() => false,
            None => read_number(material, &self.shared_n).unwrap_or(0.0) != 0.0,
        };

        // 随机比例：给了 `unit` 就用外部随机源（如 EntityRandom，对同一实体稳定），
        // 否则用内部 PRNG —— 首次 bind（rng == None）必然播种一次，之后仅在 trigger 变化时重掷。
        if !self.unit_var.is_empty() {
            let u = read_number(material, &self.unit_n).unwrap_or(0.0);
            self.unit = if u.is_finite() { u.clamp(0.0, 1.0) } else { 0.0 };
        }
        else {
            let mut reroll = self.rng.is_none();
            if !self.trigger.is_empty() {
                if let Some(t) = read_number(material, &self.trigger_n) {
                    let t = t as i32;
                    if t != self.last_trigger {
                        reroll = true;
                        self.last_trigger = t;
                    }
                }
            }
            // shared 打开/关闭时也要重掷，否则切换开关不生效
            if shared != self.last_shared {
                reroll = true;
                self.last_shared = shared;
            }
            if reroll {
                let extra = if self.seed.is_empty() {
                    0
                }
                else {
                    read_number(material, &self.seed_n).unwrap_or(0.0).to_bits()
                };
                self.reroll_count = self.reroll_count.wrapping_add(1);
                let seed = reseed(material, extra, self.reroll_count, shared);
                self.seed_and_roll(seed);
            }
        }

        let mut lo = self.min.resolve(material, 0.0);
        let mut hi = self.max.resolve(material, 1.0);
        if !lo.is_finite() {
            lo = 0.0;
        }
        if !hi.is_finite() {
            hi = 1.0;
        }
        if hi < lo {
            core::mem::swap(&mut lo, &mut hi);
        }
        let v = scale_unit(self.unit, lo, hi, self.integer);
        if self.write_int {
            material::set_int(out, v as i32)?;
        }
        else {
            material::set_float(out, v)?;
        }
        #[cfg(debug_assertions)]
        {
            if (v - self.last_value).abs() > EPS {
                let mat_name = material::get_name(material).unwrap_or_else(|_| "?".into());
                log(&format!(
                    "random[{}]: {}={v} in [{lo}, {hi}] (unit={})",
                    mat_name, self.result, self.unit
                ));
                self.last_value = v;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_unit_maps_range() {
        // unit=0 → lo；unit 接近 1 → 接近 hi
        assert_eq!(scale_unit(0.0, 5.0, 10.0, false), 5.0);
        assert_eq!(scale_unit(0.5, 0.0, 10.0, false), 5.0);
        assert_eq!(scale_unit(0.25, 0.0, 4.0, false), 1.0);
        // integer = 向下取整（$frame 必须落在整数帧上）
        assert_eq!(scale_unit(0.99, 0.0, 4.0, true), 3.0);
        assert_eq!(scale_unit(0.5, 0.0, 4.0, true), 2.0);
        // 负区间同样成立
        assert_eq!(scale_unit(0.5, -10.0, 10.0, false), 0.0);
    }

    #[test]
    fn kv_truthy_accepts_common_spellings() {
        for s in ["1", "true", "TRUE", "yes", "On", " 1 "] {
            assert!(kv_truthy(s), "{s} 应为真");
        }
        for s in ["0", "false", "no", "off", "", "$var"] {
            assert!(!kv_truthy(s), "{s} 应为假");
        }
    }

    #[test]
    fn num_param_accepts_literal_or_variable() {
        // 字面量：直接定值，不需要材质
        let lit = NumParam::new("42");
        assert_eq!(lit.lit, Some(42.0));
        // 变量名：无法解析成数字 → 留给 find_var（此处只验证解析结果）
        let var = NumParam::new("$frameLimit");
        assert_eq!(var.lit, None);
        assert_eq!(var.name, "$frameLimit");
        // 覆盖 set 会同时更新字面量与缓存名
        let mut p = NumParam::new("0");
        p.set("$max_frame");
        assert_eq!(p.lit, None);
        assert_eq!(p.name, "$max_frame");
    }

    #[test]
    fn seeded_rng_is_deterministic_and_in_range() {
        // 同一种子必须给出同一序列（保证帧间稳定）
        let mut a = RandomProxy::default();
        a.seed_and_roll(0x1234_5678);
        let mut b = RandomProxy::default();
        b.seed_and_roll(0x1234_5678);
        assert_eq!(a.unit, b.unit, "同种子应得到同一随机比例");

        // 不同种子应给出不同比例（抽样验证，避免偶然相等造成假阴性）
        let mut c = RandomProxy::default();
        c.seed_and_roll(0x8765_4321);
        assert_ne!(a.unit, c.unit, "不同种子不应得到同一比例");

        // [0,1) 映射恒在范围内
        let mut p = RandomProxy::default();
        for i in 0..1000u64 {
            p.seed_and_roll(i);
            assert!((0.0..1.0).contains(&p.unit), "unit={} 越界", p.unit);
        }
        assert!(p.rng.is_some(), "播种后应持有 PRNG");
    }

    #[test]
    fn reseed_is_nonzero_and_differs_per_instance() {
        let m1 = 0x1000usize as *mut c_void;
        let m2 = 0x2000usize as *mut c_void;
        let s1 = reseed(m1, 0, 1, false);
        let s2 = reseed(m2, 0, 1, false);
        // 不同材质实例应当拿到不同种子（静态物体也能各自随机）
        assert_ne!(s1, s2, "不同实例的种子不应相同");
        // 同一实例连续重掷也应换种子（否则 trigger 重掷无效）
        assert_ne!(s1, reseed(m1, 0, 2, false), "重掷应换种子");
        // shared 模式下同一进程内恒定 → 所有实例共用同一结果
        assert_eq!(reseed(m1, 0, 1, true), reseed(m2, 0, 99, true));
        // 且与实例无关、与计数无关（换地图才换皮肤）
        assert_eq!(reseed(m1, 7, 1, true), reseed(m2, 0, 5, true));
        assert_eq!(shared_salt(), shared_salt());
    }

    #[test]
    fn apply_kv_parses_documented_parameters() {
        let mut p = RandomProxy::default();
        p.apply_kv("MIN", "$frameLimit");
        p.apply_kv("max", "8");
        p.apply_kv("result", "$frame");
        p.apply_kv("integer", "1");
        p.apply_kv("shared", "$oneSkinPerMap");
        p.apply_kv("unknown_key", "whatever"); // 未知名应被忽略
        assert_eq!(p.min.name, "$frameLimit");
        assert_eq!(p.min.lit, None);
        assert_eq!(p.max.lit, Some(8.0));
        assert_eq!(p.result, "$frame");
        assert!(p.integer);
        assert_eq!(p.shared_lit, None, "以 $ 开头应视为变量名");
        assert_eq!(p.shared, "$oneSkinPerMap");

        // 字面量 shared 直接定值
        let mut q = RandomProxy::default();
        q.apply_kv("shared", "1");
        assert_eq!(q.shared_lit, Some(true));

        // unit 参数
        let mut r = RandomProxy::default();
        r.apply_kv("unit", "$entityUnit");
        assert_eq!(r.unit_var, "$entityUnit");
    }
}