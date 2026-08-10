#![allow(dead_code)] // set_int/set_string 等 API 供插件开发者选用
//! 材质代理：代理注册表、KeyValues 解析、detour hook、D3D EndScene 每帧执行。
//! 详细机制 / 逆向依据 / 注意事项见项目根目录 AGENTS.md。

use core::ffi::{c_char, c_void, CStr};
use core::mem::transmute;
use std::ffi::CString;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use uuid::Uuid;

use crate::kv::{is_readable, Proxy};

// ---------- IMaterial / IMaterialVar 访问（L4D2 materialsystem.dll，逆向确认） ----------
const MAT_FIND_VAR: usize = 0x2c;
const MATVAR_SET_FLOAT: usize = 0x0c;
const MATVAR_SET_INT: usize = 0x10;
const MATVAR_SET_STRING: usize = 0x14;
const MATVAR_SET_VEC: usize = 0x30;
const MATVAR_SET_VEC_COMPONENT: usize = 0x64;
const MATVAR_GET_STRING: usize = 0x18;
const MATVAR_GET_INT: usize = 0x68;
const MATVAR_GET_FLOAT: usize = 0x6c;
const MATVAR_GET_VEC: usize = 0x70;

type GetNameFn = unsafe extern "thiscall" fn(*const c_void) -> *const c_char;
type FindVarFn = unsafe extern "thiscall" fn(*const c_void, *const c_char, *mut u8, i32) -> *mut c_void;
type SetFloatFn = unsafe extern "thiscall" fn(*const c_void, f32);
type SetIntFn = unsafe extern "thiscall" fn(*const c_void, i32);
type SetStringFn = unsafe extern "thiscall" fn(*const c_void, *const c_char);
type SetVecFn = unsafe extern "thiscall" fn(*const c_void, *const f32, i32);
type GetStringFn = unsafe extern "thiscall" fn(*const c_void) -> *const c_char;
type GetIntFn = unsafe extern "thiscall" fn(*const c_void) -> i32;
type GetFloatFn = unsafe extern "thiscall" fn(*const c_void) -> f32;
type GetVecFn = unsafe extern "thiscall" fn(*const c_void, *mut f32, i32);

/// 在 IMaterial 上查找 VMT 变量（如 "$color2" / "$result_var"）。返回 `IMaterialVar*` 或 null。
///
/// # Safety
/// `mat` 必须是有效的 `IMaterial*`。
pub unsafe fn find_var(mat: *mut c_void, name: &CStr) -> *mut c_void {
    if mat.is_null() {
        return core::ptr::null_mut();
    }
    let vft = *(mat as *const *const c_void);
    if vft.is_null() {
        return core::ptr::null_mut();
    }
    let f: FindVarFn = transmute(*((vft as *const usize).add(MAT_FIND_VAR / 4)));
    let mut found: u8 = 0;
    let v = f(mat, name.as_ptr(), &mut found, 1);
    if found != 0 {
        v
    } else {
        core::ptr::null_mut()
    }
}

/// 获取材质名称（VMT 文件名，如 `"vgui/common/l4d_spinner"`）。
///
/// 逆向确认（materialsystem `FUN_10002d50`：`MOV EDX,[ESI]; MOV EAX,[EDX]; MOV ECX,ESI; CALL EAX`
/// 返回值用于 `Warning("Material \"%s\"...")`）：`IMaterial::GetName` = **vtable +0x00**，
/// `thiscall(this=IMaterial*) -> const char*`。
/// # Safety
/// `mat` 必须是有效的 `IMaterial*`。
pub unsafe fn get_name(mat: *mut c_void) -> Option<String> {
    if mat.is_null() {
        return None;
    }
    let vft = *(mat as *const *const c_void);
    if vft.is_null() {
        return None;
    }
    let f: GetNameFn = transmute(*((vft as *const usize).add(0)));
    let p = f(mat);
    if p.is_null() {
        return None;
    }
    // 防坏指针（strlen 崩溃）
    if !crate::kv::is_readable(p as *const c_void) {
        return None;
    }
    Some(CStr::from_ptr(p).to_string_lossy().into_owned())
}

/// 设置 IMaterialVar 的浮点值（等价 `SetFloatValue`，vtable +0x0c）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn set_float(var: *mut c_void, value: f32) {
    if var.is_null() {
        return;
    }
    let vft = *(var as *const *const c_void);
    if vft.is_null() {
        return;
    }
    let f: SetFloatFn = transmute(*((vft as *const usize).add(MATVAR_SET_FLOAT / 4)));
    f(var, value);
}

/// 设置 IMaterialVar 的整数值（等价 `SetIntValue`，vtable +0x10）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn set_int(var: *mut c_void, value: i32) {
    if var.is_null() {
        return;
    }
    let vft = *(var as *const *const c_void);
    if vft.is_null() {
        return;
    }
    let f: SetIntFn = transmute(*((vft as *const usize).add(MATVAR_SET_INT / 4)));
    f(var, value);
}

/// 设置 IMaterialVar 的字符串值（等价 `SetStringValue`，vtable +0x14）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn set_string(var: *mut c_void, value: &CStr) {
    if var.is_null() {
        return;
    }
    let vft = *(var as *const *const c_void);
    if vft.is_null() {
        return;
    }
    let f: SetStringFn = transmute(*((vft as *const usize).add(MATVAR_SET_STRING / 4)));
    f(var, value.as_ptr());
}

/// 设置 IMaterialVar 的向量值（等价 `SetVecValue(float*, n)`，vtable +0x30）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`；`values` 长度 >= `n`。
pub unsafe fn set_vec(var: *mut c_void, values: &[f32]) {
    if var.is_null() {
        return;
    }
    let vft = *(var as *const *const c_void);
    if vft.is_null() {
        return;
    }
    let f: SetVecFn = transmute(*((vft as *const usize).add(MATVAR_SET_VEC / 4)));
    f(var, values.as_ptr(), values.len() as i32);
}

/// 读取 IMaterialVar 的浮点值（等价 `GetFloatValue`，vtable +0x6c）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn get_float(var: *mut c_void) -> f32 {
    if var.is_null() {
        return 0.0;
    }
    let vft = *(var as *const *const c_void);
    if vft.is_null() {
        return 0.0;
    }
    let f: GetFloatFn = transmute(*((vft as *const usize).add(MATVAR_GET_FLOAT / 4)));
    f(var)
}

/// 读取 IMaterialVar 的字符串值（等价 `GetStringValue()`，vtable +0x18，thiscall 返回 `const char*`）。
///
/// 逆向依据（materialsystem.dll）：引用诊断字符串 `"CMaterialVar::GetStringValue: Unknown
/// material var type"` 的实现 `FUN_10019e70`（case 1: `return param_1[1]` 直接返回内部字符串
/// 指针）位于 vtable@0x1009d274 的 **+0x18** 槽位；该 vtable 起点由已验证偏移交叉确认
/// （+0x0c=SetFloat、+0x14=SetString、+0x6c=GetFloat、+0x70=GetVec）。顺带确认
/// `GetIntValue` = +0x68（`FUN_10019c60: return param_1[2]`）。
///
/// 返回字符串副本；坏指针/不可读内存返回 `None`（防 strlen 崩溃）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn get_string(var: *mut c_void) -> Option<String> {
    if var.is_null() {
        return None;
    }
    let vft = *(var as *const *const c_void);
    if vft.is_null() {
        return None;
    }
    let f: GetStringFn = transmute(*((vft as *const usize).add(MATVAR_GET_STRING / 4)));
    let p = f(var);
    if p.is_null() {
        return None;
    }
    if !crate::kv::is_readable(p as *const c_void) {
        return None;
    }
    Some(CStr::from_ptr(p).to_string_lossy().into_owned())
}

/// 读取 IMaterialVar 的整数值（等价 `GetIntValue()`，vtable +0x68，thiscall 返回 `int`）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn get_int(var: *mut c_void) -> i32 {
    if var.is_null() {
        return 0;
    }
    let vft = *(var as *const *const c_void);
    if vft.is_null() {
        return 0;
    }
    let f: GetIntFn = transmute(*((vft as *const usize).add(MATVAR_GET_INT / 4)));
    f(var)
}

/// 读取 IMaterialVar 的向量值（等价 `GetVecValue(float*, n)`，vtable +0x70）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn get_vec(var: *mut c_void, out: &mut [f32; 3]) {
    if var.is_null() {
        return;
    }
    let vft = *(var as *const *const c_void);
    if vft.is_null() {
        return;
    }
    let f: GetVecFn = transmute(*((vft as *const usize).add(MATVAR_GET_VEC / 4)));
    f(var, out.as_mut_ptr(), 3);
}

/// 读取 IMaterialVar 向量的第 `index` 个分量（float）。
///
/// 注意：L4D2 `IMaterialVar` **没有** `GetVecComponentValue`（读单分量）槽位，只有写单分量的
/// `SetVecComponentValue(+0x64)`；读单分量需经 `GetVecValue(+0x70)` 读全向量后取下标
/// （或 `GetVecValueInternal(+0x74)` 返回内部 `float*` 后读 `[n]`）。此函数封装前者。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn get_vec_component(var: *mut c_void, index: usize) -> f32 {
    let mut o = [0.0f32; 3];
    get_vec(var, &mut o);
    if index < 3 {
        o[index]
    } else {
        0.0
    }
}

// ---------- 代理注册表（泛型） ----------
struct RegEntry {
    name: &'static str,
    new_proxy: fn() -> Box<dyn Proxy>,
}
static REGISTRY: Mutex<Vec<RegEntry>> = Mutex::new(Vec::new());

/// 注册一个自定义材质代理：VMT `"Proxies"` 里出现该名字时创建 `P` 实例、
/// 注入 `"Proxies"` 块参数（`apply_kv`）并调用 `bind(material)`。
pub fn register_proxy<P: Proxy + Default + 'static>(name: &'static str) -> bool {
    let mut reg = REGISTRY.lock().unwrap();
    if reg.iter().any(|e| e.name == name) {
        return false;
    }
    reg.push(RegEntry {
        name,
        new_proxy: || Box::new(P::default()),
    });
    true
}

pub fn proxy_registered(name: &str) -> bool {
    REGISTRY.lock().unwrap().iter().any(|e| e.name == name)
}

pub fn registered_names() -> Vec<String> {
    REGISTRY.lock().unwrap().iter().map(|e| e.name.to_string()).collect()
}

// ---------- 每帧执行（D3D EndScene 触发） ----------
struct ActiveProxy {
    // 唯一 id：同一材质可注册多个 per-frame 代理（各自独立条目，参数隔离）
    id: u64,
    material: *mut c_void,
    proxy: Box<dyn Proxy>,
}
// material 指针仅在渲染线程内使用，手动标记 Send 以放入 Mutex
unsafe impl Send for ActiveProxy {}
static ACTIVE: Mutex<Vec<ActiveProxy>> = Mutex::new(Vec::new());
static NEXT_ACTIVE_ID: AtomicU64 = AtomicU64::new(1);

/// 注册需每帧执行的代理（材质加载时调用）。
/// 同一材质可注册多个不同代理（各自独立条目）；**不能按材质去重**，否则同一材质上
/// 多个 per-frame 代理只会保留最后一个（曾导致 `l4nrp_print_variable` 被
/// `l4nrp_is_in_range` 替换而不执行，见 AGENTS.md 已知问题）。
pub fn register_active(material: *mut c_void, proxy: Box<dyn Proxy>) {
    let mut a = ACTIVE.lock().unwrap();
    let id = NEXT_ACTIVE_ID.fetch_add(1, Ordering::Relaxed);
    a.push(ActiveProxy { id, material, proxy });
}

/// D3D 每帧回调：对所有活动代理执行 `bind`（读当前材质变量 → 计算 → 写回）。
/// 注意：须先复制指针并释放 `ACTIVE` 锁再逐个 `bind`，避免锁重入死锁（见 AGENTS.md）。
pub fn run_active_proxies() {
    // 先触发到期的计时器（l4nrp_delay_set 生成的）
    run_timers();
    let items: Vec<(u64, *mut c_void, *mut Box<dyn Proxy>)> = {
        let mut a = ACTIVE.lock().unwrap();
        a.iter_mut().map(|e| (e.id, e.material, &mut e.proxy as *mut Box<dyn Proxy>)).collect()
    };
    // bind 返回 false = 材质失效/变量缺失 → 从活动表移除该条目
    let mut stale: Vec<u64> = Vec::new();
    for (id, m, p) in items {
        let ok = unsafe { (&mut *p).bind(m) };
        if !ok {
            stale.push(id);
        }
    }
    if !stale.is_empty() {
        let mut a = ACTIVE.lock().unwrap();
        a.retain(|e| !stale.contains(&e.id));
    }
}

// ---------- 计时器注册表（l4nrp_delay_set 生成 / l4nrp_delay_abort 中断） ----------
struct ActiveTimer {
    handle: String, // UUID v4 字符串手柄
    material: *mut c_void,
    output_n: CString, // 到期后把 value 变量整型值写入此变量
    value_n: CString,  // 值来源变量名
    end: Instant,
}
// material 指针仅在渲染线程内使用，手动标记 Send 以放入 Mutex
unsafe impl Send for ActiveTimer {}
static TIMERS: Mutex<Vec<ActiveTimer>> = Mutex::new(Vec::new());

/// 启动一个计时器：`delay_ms` 后（由 `run_timers` 每帧触发）把 `value` 变量整型值写入
/// `output`。返回 **UUID v4 字符串手柄**，供中断/查询。
pub fn start_timer(material: *mut c_void, output_n: CString, value_n: CString, delay_ms: u64) -> String {
    let h = Uuid::new_v4().to_string();
    let mut ts = TIMERS.lock().unwrap();
    ts.push(ActiveTimer {
        handle: h.clone(),
        material,
        output_n,
        value_n,
        end: Instant::now() + std::time::Duration::from_millis(delay_ms),
    });
    h
}

/// 按手柄（UUID 字符串）中断计时器。返回是否找到并移除。
pub fn abort_timer(handle: &str) -> bool {
    let mut ts = TIMERS.lock().unwrap();
    let before = ts.len();
    ts.retain(|t| t.handle != handle);
    ts.len() != before
}

/// 指定手柄的计时器是否仍在运行。
pub fn timer_active(handle: &str) -> bool {
    TIMERS.lock().unwrap().iter().any(|t| t.handle == handle)
}

/// 每帧触发到期的计时器：把 `value` 变量整型值写入 `output` 并移除。
/// 先取出到期项再释放锁执行，避免持锁调用引擎（见 AGENTS.md 锁注意事项）。
fn run_timers() {
    let mut fired: Vec<ActiveTimer> = Vec::new();
    {
        let now = Instant::now();
        let mut ts = TIMERS.lock().unwrap();
        let mut i = 0;
        while i < ts.len() {
            if now >= ts[i].end {
                fired.push(ts.remove(i));
            } else {
                i += 1;
            }
        }
    }
    for t in fired {
        // 防悬垂：材质已不可读（被引擎卸载/替换）则丢弃该计时器，避免解引用坏指针崩溃
        if !is_readable(t.material as *const c_void) {
            continue;
        }
        unsafe {
            let out = find_var(t.material, &t.output_n);
            if !out.is_null() {
                let val = get_int(find_var(t.material, &t.value_n));
                set_int(out, val);
            }
        }
    }
}

// ---------- 引擎 KeyValues 函数（materialsystem.dll RVA） ----------
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleA(lp: *const u8) -> *mut c_void;
}

type FindKeyFn = unsafe extern "thiscall" fn(*mut c_void, *const c_char, u8) -> *mut c_void;
type FirstChildFn = unsafe extern "fastcall" fn(*mut c_void) -> *mut c_void;
type NextSiblingFn = unsafe extern "fastcall" fn(*mut c_void) -> *mut c_void;
type GetKeyNameFn = unsafe extern "fastcall" fn(*mut c_void) -> *const c_char;

fn ms_base() -> usize {
    unsafe { GetModuleHandleA(b"materialsystem.dll\0".as_ptr()) as usize }
}

unsafe fn ms_fn(rva: usize) -> usize {
    let b = ms_base();
    if b == 0 {
        0
    } else {
        b + rva
    }
}

/// 遍历代理名子键里的参数子键，逐个调用 `apply_kv`。
/// # Safety
/// `node` 为 `"Proxies"` 块里某个代理名子键。
unsafe fn parse_proxy_params(proxy: &mut dyn Proxy, node: *mut c_void) {
    let first_child: FirstChildFn = transmute(ms_fn(0x75dc0));
    let next_sib: NextSiblingFn = transmute(ms_fn(0x75dd0));
    let get_name: GetKeyNameFn = transmute(ms_fn(0x75b90));
    let mut arg = first_child(node);
    let mut guard = 0;
    while !arg.is_null() && guard < 32 {
        guard += 1;
        let name_ptr = get_name(arg);
        // +0x04 是值字符串指针（const char*），见 AGENTS.md
        let val_ptr = *(arg.add(0x04) as *const *const c_char);
        if !name_ptr.is_null() && !val_ptr.is_null() && is_readable(val_ptr as *const c_void) {
            let name = CStr::from_ptr(name_ptr).to_string_lossy().into_owned();
            let val = CStr::from_ptr(val_ptr).to_string_lossy().into_owned();
            proxy.apply_kv(&name, &val);
        }
        arg = next_sib(arg);
    }
}

/// 用引擎 KeyValues 函数处理材质的 `"Proxies"` 块。
///
/// 返回 `true` 表示命中并处理了已注册代理（hook 据此决定透传策略，见 AGENTS.md）；
/// 返回 `false` 表示无需拦截。
///
/// # Safety
/// `material` 为有效 `IMaterial*`；`kv` 为材质 KeyValues 根。
unsafe fn apply_proxies(material: *mut c_void, kv: *mut c_void) -> bool {
    if kv.is_null() {
        return false;
    }
    let find_key: FindKeyFn = transmute(ms_fn(0x76020));
    let first_child: FirstChildFn = transmute(ms_fn(0x75dc0));
    let next_sib: NextSiblingFn = transmute(ms_fn(0x75dd0));
    let get_name: GetKeyNameFn = transmute(ms_fn(0x75b90));
    if find_key as usize == 0 || first_child as usize == 0 || next_sib as usize == 0 || get_name as usize == 0 {
        crate::log("apply_proxies: engine KV functions unavailable (materialsystem not loaded?)");
        return false;
    }

    // 用引擎 FindKey 找 "Proxies" 块（create=0，不创建）
    let proxies = find_key(kv, c"Proxies".as_ptr(), 0);
    if proxies.is_null() {
        return false;
    }

    // 遍历 "Proxies" 子键：处理我们注册的代理并把它从链表摘除（透传时引擎只处理剩余代理）
    // 链表：proxies.m_pSub(+0x20) 首子键，节点 m_pPeer(+0x1c) 兄弟
    let mut handled = false;
    let mut prev: *mut c_void = core::ptr::null_mut();
    let mut cur = first_child(proxies);
    let mut guard = 0;
    while !cur.is_null() && guard < 64 {
        guard += 1;
        let name_ptr = get_name(cur);
        let mut is_ours = false;
        if !name_ptr.is_null() {
            let name = CStr::from_ptr(name_ptr).to_string_lossy().into_owned();
            let reg = REGISTRY.lock().unwrap();
            if let Some(e) = reg.iter().find(|e| e.name == name) {
                let mut proxy = (e.new_proxy)();
                parse_proxy_params(&mut *proxy, cur);
                crate::log(&format!(
                    "apply_proxies: MATCH '{}' material=0x{:x}",
                    name,
                    material as usize
                ));
                if proxy.per_frame() {
                    // 持续计算：先执行一次并注册到活动表（D3D 每帧再执行）
                    let _ = proxy.bind(material);
                    register_active(material, proxy);
                    crate::log(&format!("apply_proxies: '{}' registered per-frame", name));
                } else {
                    // 一次性：仅材质加载时执行
                    let _ = proxy.bind(material);
                }
                handled = true;
                is_ours = true;
            }
        }
        let next = next_sib(cur);
        if is_ours {
            // 摘除当前节点：前驱.m_pPeer = 当前.m_pPeer；若为首子键则 proxies.m_pSub = 当前.m_pPeer
            if !prev.is_null() {
                *(prev.add(0x1c) as *mut *mut c_void) = next;
            } else {
                *(proxies.add(0x20) as *mut *mut c_void) = next;
            }
            *(cur.add(0x1c) as *mut *mut c_void) = core::ptr::null_mut(); // 防残留
        } else {
            prev = cur;
        }
        cur = next;
    }
    handled
}

// ---------- detour hook（FUN_10002d50） ----------
#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualProtect(lp: *mut c_void, dw_size: usize, fl_new: u32, lp_old: *mut u32) -> i32;
    fn VirtualAlloc(lp: *mut c_void, dw_size: usize, fl_type: u32, fl_prot: u32) -> *mut c_void;
}
const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;

// 原始 FUN_10002d50（trampoline）与 hook 目标
static mut ORIGINAL_PROXY_PARSE: usize = 0;
static mut HOOKED_TARGET: usize = 0;
static mut HOOKED_SAVED: [u8; 5] = [0; 5];

/// 改写 `target` 前 5 字节为 `E9 rel32`（近 JMP 到 `replacement`）。保存原字节。
unsafe fn hook_function(target: usize, replacement: usize) -> bool {
    core::ptr::copy_nonoverlapping(
        target as *const u8,
        core::ptr::addr_of_mut!(HOOKED_SAVED) as *mut u8,
        5,
    );
    let mut old_prot: u32 = 0;
    if VirtualProtect(target as *mut c_void, 5, PAGE_EXECUTE_READWRITE, &mut old_prot) == 0 {
        return false;
    }
    let rel = replacement.wrapping_sub(target + 5) as i32;
    *(target as *mut u8) = 0xE9;
    *((target + 1) as *mut i32) = rel;
    let mut tmp: u32 = 0;
    let _ = VirtualProtect(target as *mut c_void, 5, old_prot, &mut tmp);
    HOOKED_TARGET = target;
    true
}

/// 生成 trampoline：复制 `target` 前 `patch_len` 字节到可执行内存，再 JMP 回 `target+patch_len`。
unsafe fn make_trampoline(target: usize, patch_len: usize) -> usize {
    let mem = VirtualAlloc(core::ptr::null_mut(), 64, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) as usize;
    if mem == 0 {
        return 0;
    }
    core::ptr::copy_nonoverlapping(target as *const u8, mem as *mut u8, patch_len);
    *((mem + patch_len) as *mut u8) = 0xE9;
    let rel = (target + patch_len).wrapping_sub(mem + patch_len + 5) as i32;
    *((mem + patch_len + 1) as *mut i32) = rel;
    mem
}

/// hook `FUN_10002d50`（materialsystem RVA 0x2d50）。入口 9 字节完整指令，trampoline 复制 9 字节。
pub unsafe fn install(parse_addr: usize) -> bool {
    if parse_addr == 0 {
        return false;
    }
    let original_entry: [u8; 9] = [0x55, 0x8B, 0xEC, 0x81, 0xEC, 0x08, 0x04, 0x00, 0x00];
    let mut head: [u8; 5] = [0; 5];
    core::ptr::copy_nonoverlapping(parse_addr as *const u8, head.as_mut_ptr(), 5);
    if head[0] == 0xE9 {
        // 链式接管：FUN_10002d50 已被其它插件（如 rust_l4n_node_texture_plugin）detour，
        // 解析入口 E9 rel32 得到先加载者 hook 作为下一跳，再把自己的 hook patch 到入口。
        // 两个插件各自处理自己的代理，形成 hook 链，避免 trampoline 复制对方 jmp 崩溃。
        let rel = i32::from_le_bytes([head[1], head[2], head[3], head[4]]);
        let next = parse_addr.wrapping_add(5).wrapping_add(rel as usize);
        ORIGINAL_PROXY_PARSE = next;
        crate::log(&format!(
            "install: FUN_10002d50 already hooked at 0x{:x}, chaining detour",
            next
        ));
        return hook_function(parse_addr, proxy_parse_hook as *const () as usize);
    }
    let mut cur: [u8; 9] = [0; 9];
    core::ptr::copy_nonoverlapping(parse_addr as *const u8, cur.as_mut_ptr(), 9);
    if cur != original_entry {
        crate::log("install: FUN_10002d50 entry unexpected, skipping install");
        return false;
    }
    let tramp = make_trampoline(parse_addr, 9);
    if tramp == 0 {
        return false;
    }
    ORIGINAL_PROXY_PARSE = tramp;
    if !hook_function(parse_addr, proxy_parse_hook as *const () as usize) {
        return false;
    }
    true
}

/// 还原被 hook 的函数入口。进程退出前可调用。
pub unsafe fn uninstall() {
    let target = core::ptr::addr_of!(HOOKED_TARGET).read();
    if target != 0 {
        let mut old_prot: u32 = 0;
        if VirtualProtect(target as *mut c_void, 5, PAGE_EXECUTE_READWRITE, &mut old_prot) != 0 {
            core::ptr::copy_nonoverlapping(
                core::ptr::addr_of!(HOOKED_SAVED) as *const u8,
                target as *mut u8,
                5,
            );
            let mut tmp: u32 = 0;
            let _ = VirtualProtect(target as *mut c_void, 5, old_prot, &mut tmp);
        }
    }
}

// ---------- D3D9 EndScene 每帧 hook（执行活动代理） ----------
// D3D9 COM 方法为 __stdcall，故 hook 用 extern "system"（见 AGENTS.md）
static mut ORIGINAL_ENDSCENE: usize = 0;

type EndSceneFn = unsafe extern "system" fn(*mut c_void) -> i32;

unsafe extern "system" fn endscene_hook(this: *mut c_void) -> i32 {
    // 每帧：先执行活动代理，再透传原 EndScene
    run_active_proxies();
    let orig: EndSceneFn = transmute(ORIGINAL_ENDSCENE);
    orig(this)
}

/// patch D3D9 `IDirect3DDevice9::EndScene`（vtable 索引 42），每帧先执行活动代理再透传原函数。
pub unsafe fn install_d3d_endscene(device: *mut c_void) -> bool {
    if device.is_null() {
        return false;
    }
    let vft = *(device as *const *const usize);
    if vft.is_null() {
        return false;
    }
    let slot: *mut usize = vft.add(42) as *mut usize;
    let orig = *slot;
    let hook_addr = endscene_hook as *const () as usize;
    if orig == 0 || orig == hook_addr {
        return false;
    }
    let mut old: u32 = 0;
    if VirtualProtect(slot as *mut c_void, 4, PAGE_EXECUTE_READWRITE, &mut old) == 0 {
        return false;
    }
    ORIGINAL_ENDSCENE = orig;
    *slot = hook_addr;
    let mut t: u32 = 0;
    let _ = VirtualProtect(slot as *mut c_void, 4, old, &mut t);
    crate::log(&format!(
        "D3D EndScene hook installed (device=0x{:x})",
        device as usize
    ));
    true
}

/// proxy 解析 hook：处理 `"Proxies"` 块（命中注册表则创建代理 + 注入参数 + bind），
/// 并总是透传原函数（我们的代理已摘除，引擎只处理剩余内置代理，见 AGENTS.md）。
unsafe extern "thiscall" fn proxy_parse_hook(this: *mut c_void, kv: *mut c_void) {
    // this = CMaterial，kv = 材质 KeyValues
    let handled = apply_proxies(this, kv);
    if handled {
        crate::log(&format!("apply_proxies: 0x{:x} handled", this as usize));
    }
    // 总是透传（我们的代理已摘除，可与内置代理共存）。
    // 加固：链式下一跳（trampoline 或先加载者 hook）无效时安全返回，避免调用坏指针。
    let orig_ptr = core::ptr::addr_of!(ORIGINAL_PROXY_PARSE).read();
    if orig_ptr == 0 || !crate::kv::is_readable(orig_ptr as *const c_void) {
        return;
    }
    let orig: unsafe extern "thiscall" fn(*mut c_void, *mut c_void) = transmute(orig_ptr);
    orig(this, kv);
}
