#![allow(dead_code)] // set_int/set_string 等 API 供插件开发者选用
//! 材质代理：代理注册表、KeyValues 解析、detour hook、D3D EndScene 每帧执行。
//! 详细机制 / 逆向依据 / 注意事项见项目根目录 AGENTS.md。

use core::ffi::{c_char, c_void, CStr};
use core::mem::transmute;
use std::collections::HashMap;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::sync::LazyLock;
use std::time::Instant;

use uuid::Uuid;
use windows::core::s;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect};

use crate::error::{MaterialError, PluginError};
use crate::kv::Proxy;

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

/// 取 COM 对象 vtable 上第 `slot_off` 字节处的函数指针。
///
/// 用 `&raw const` 取裸指针（而非 `&` 引用 + `as`）读取：`this` 是引擎对象，
/// 我们只借用它的 vtable 内存、不构造引用，避免为外来指针凭空造出引用。
///
/// # Safety
/// `this` 必须是有效对象指针，且 `slot_off` 为该对象 vtable 内合法的函数槽偏移。
unsafe fn vtable_slot(this: *const c_void, slot_off: usize) -> usize {
    let vft: *const usize = *(&raw const *this).cast::<*const usize>();
    *vft.add(slot_off / 4)
}

/// 对象首字段（vtable 指针）是否为空。`true` = 坏对象，调用方应报 `UnexpectedInstance`。
///
/// # Safety
/// `this` 必须可读（至少首字段所在内存可读）。
unsafe fn vtable_is_null(this: *const c_void) -> bool {
    (*(&raw const *this).cast::<*const c_void>()).is_null()
}

/// 在 IMaterial 上查找 VMT 变量（如 "$color2" / "$result_var"）。返回 `IMaterialVar*` 或 null。
///
/// # Safety
/// `mat` 必须是有效的 `IMaterial*`。
pub unsafe fn find_var(mat: *mut c_void, name: &CStr) -> Result<*mut c_void, PluginError> {
    if mat.is_null() {
        return Err(PluginError::Material(MaterialError::InvalidMaterial));
    }
    if vtable_is_null(mat) {
        return Err(PluginError::Material(MaterialError::UnexpectedInstance));
    }
    let f: FindVarFn = transmute(vtable_slot(mat, MAT_FIND_VAR));
    let mut found = 0;
    let v = f(mat, name.as_ptr(), &mut found, 1);
    if found != 0 {
        Ok(v)
    }
    else {
        Err(PluginError::Material(MaterialError::VariableNotFound(
            Box::from(name.to_string_lossy().into_owned())
        )))
    }
}

/// 获取材质名称（VMT 文件名，如 `"vgui/common/l4d_spinner"`）。
///
/// 逆向确认（materialsystem `FUN_10002d50`：`MOV EDX,[ESI]; MOV EAX,[EDX]; MOV ECX,ESI; CALL EAX`
/// 返回值用于 `Warning("Material \"%s\"...")`）：`IMaterial::GetName` = **vtable +0x00**，
/// `thiscall(this=IMaterial*) -> const char*`。
/// # Safety
/// `mat` 必须是有效的 `IMaterial*`。
pub unsafe fn get_name(mat: *mut c_void) -> Result<String, PluginError> {
    if mat.is_null() {
        return Err(PluginError::Material(MaterialError::InvalidMaterial));
    }
    if vtable_is_null(mat) {
        return Err(PluginError::Material(MaterialError::UnexpectedInstance));
    }
    let f: GetNameFn = transmute(vtable_slot(mat, 0));
    let p = f(mat);
    if p.is_null() {
        return Err(PluginError::Material(MaterialError::UnexpectedInstance));
    }
    crate::kv::test_readable(p.cast())?;

    Ok(CStr::from_ptr(p).to_string_lossy().into_owned())
}

/// 设置 IMaterialVar 的浮点值（等价 `SetFloatValue`，vtable +0x0c）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn set_float(var: *mut c_void, value: f32) -> Result<(), PluginError> {
    if var.is_null() {
        return Err(PluginError::Material(MaterialError::InvalidMaterial));
    }
    if vtable_is_null(var) {
        return Err(PluginError::Material(MaterialError::UnexpectedInstance));
    }
    let f: SetFloatFn = transmute(vtable_slot(var, MATVAR_SET_FLOAT));
    f(var, value);

    Ok(())
}

/// 设置 IMaterialVar 的整数值（等价 `SetIntValue`，vtable +0x10）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn set_int(var: *mut c_void, value: i32) -> Result<(), PluginError> {
    if var.is_null() {
        return Err(PluginError::Material(MaterialError::InvalidMaterial));
    }
    if vtable_is_null(var) {
        return Err(PluginError::Material(MaterialError::UnexpectedInstance));
    }
    let f: SetIntFn = transmute(vtable_slot(var, MATVAR_SET_INT));
    f(var, value);

    Ok(())
}

/// 设置 IMaterialVar 的字符串值（等价 `SetStringValue`，vtable +0x14）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn set_string(var: *mut c_void, value: &CStr) -> Result<(), PluginError> {
    if var.is_null() {
        return Err(PluginError::Material(MaterialError::InvalidMaterial));
    }
    if vtable_is_null(var) {
        return Err(PluginError::Material(MaterialError::UnexpectedInstance));
    }
    let f: SetStringFn = transmute(vtable_slot(var, MATVAR_SET_STRING));
    f(var, value.as_ptr());

    Ok(())
}

/// 设置 IMaterialVar 的向量值（等价 `SetVecValue(float*, n)`，vtable +0x30）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`；`values` 长度 >= `n`。
pub unsafe fn set_vec(var: *mut c_void, values: &[f32]) -> Result<(), PluginError> {
    if var.is_null() {
        return Err(PluginError::Material(MaterialError::InvalidMaterial));
    }
    if vtable_is_null(var) {
        return Err(PluginError::Material(MaterialError::UnexpectedInstance));
    }
    let f: SetVecFn = transmute(vtable_slot(var, MATVAR_SET_VEC));
    f(var, values.as_ptr(), values.len() as i32);

    Ok(())
}

/// 读取 IMaterialVar 的浮点值（等价 `GetFloatValue`，vtable +0x6c）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn get_float(var: *mut c_void) -> Result<f32, PluginError> {
    if var.is_null() {
        return Err(PluginError::Material(MaterialError::InvalidMaterial));
    }
    if vtable_is_null(var) {
        return Err(PluginError::Material(MaterialError::UnexpectedInstance));
    }
    let f: GetFloatFn = transmute(vtable_slot(var, MATVAR_GET_FLOAT));

    Ok(f(var))
}

/// 读取 IMaterialVar 的字符串值（等价 `GetStringValue()`，vtable +0x18，thiscall 返回 `const char*`）。
///
/// 逆向依据（materialsystem.dll）：引用诊断字符串 `"CMaterialVar::GetStringValue: Unknown
/// material var type"` 的实现 `FUN_10019e70`（case 1: `return param_1[1]` 直接返回内部字符串
/// 指针）位于 vtable@0x1009d274 的 **+0x18** 槽位；该 vtable 起点由已验证偏移交叉确认
/// （+0x0c=SetFloat、+0x14=SetString、+0x6c=GetFloat、+0x70=GetVec）。顺带确认
/// `GetIntValue` = +0x68（`FUN_10019c60: return param_1[2]`）。
///
/// 返回字符串副本；坏指针/不可读内存返回 `Err`（防 strlen 崩溃）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn get_string(var: *mut c_void) -> Result<String, PluginError> {
    if var.is_null() {
        return Err(PluginError::Material(MaterialError::InvalidMaterial));
    }
    if vtable_is_null(var) {
        return Err(PluginError::Material(MaterialError::UnexpectedInstance));
    }
    let f: GetStringFn = transmute(vtable_slot(var, MATVAR_GET_STRING));
    let p = f(var);
    crate::kv::test_readable(p.cast())?;

    Ok(CStr::from_ptr(p).to_string_lossy().into_owned())
}

/// 读取 IMaterialVar 的整数值（等价 `GetIntValue()`，vtable +0x68，thiscall 返回 `int`）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn get_int(var: *mut c_void) -> Result<i32, PluginError> {
    if var.is_null() {
        return Err(PluginError::Material(MaterialError::InvalidMaterial));
    }
    if vtable_is_null(var) {
        return Err(PluginError::Material(MaterialError::UnexpectedInstance));
    }
    let f: GetIntFn = transmute(vtable_slot(var, MATVAR_GET_INT));

    Ok(f(var))
}

/// 读取 IMaterialVar 的向量值（等价 `GetVecValue(float*, n)`，vtable +0x70）。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn get_vec(var: *mut c_void, out: &mut [f32; 3]) -> Result<(), PluginError> {
    if var.is_null() {
        return Err(PluginError::Material(MaterialError::InvalidMaterial));
    }
    if vtable_is_null(var) {
        return Err(PluginError::Material(MaterialError::UnexpectedInstance));
    }
    let f: GetVecFn = transmute(vtable_slot(var, MATVAR_GET_VEC));
    f(var, out.as_mut_ptr(), 3);

    Ok(())
}

/// 读取 IMaterialVar 向量的第 `index` 个分量（float）。
///
/// 注意：L4D2 `IMaterialVar` **没有** `GetVecComponentValue`（读单分量）槽位，只有写单分量的
/// `SetVecComponentValue(+0x64)`；读单分量需经 `GetVecValue(+0x70)` 读全向量后取下标
/// （或 `GetVecValueInternal(+0x74)` 返回内部 `float*` 后读 `[n]`）。此函数封装前者。
/// # Safety
/// `var` 必须是 `find_var` 返回的有效 `IMaterialVar*`。
pub unsafe fn get_vec_component(var: *mut c_void, index: usize) -> Result<f32, PluginError> {
    let mut o = [0.0f32; 3];
    get_vec(var, &mut o)?;
    if index < 3 {
        Ok(o[index])
    } else {
        Err(PluginError::Material(MaterialError::VectorAccessOutOfBound(index, 3)))
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

// ---------- 引擎侧代理对象（真正注册进 CMaterialProxyDict 的 IMaterialProxy） ----------
// 逆向依据（详见 AGENTS.md「引擎原生代理注册」）：
//   * `client.dll` 的 `CMaterialProxyDict`（vtable RVA 0x576d94）只有 3 个槽位：
//     `[0]` CreateProxy、`[1]` DeleteProxy、`[2]` AddProxy；
//   * `materialsystem.dll` 的 VMT 解析函数 `FUN_10002d50` 遍历 `"Proxies"` 子键，
//     对每个键调用 `CreateProxy(名字)`，成功后调用 `Init(IMaterial*, KeyValues*)`；
//     只有 `Init` 返回 false 才调用 `DeleteProxy` 回收；
//   * 材质卸载时引擎调用 `Release()`（`AnimatedTexture::Release` 即自调 `vtable[0x10](1)` 自删）。
// 因此本插件把自己注册成引擎原生代理，不再 detour 引擎解析函数，也不再从
// `"Proxies"` 块里摘除节点 —— 引擎自己会实例化并驱动我们。
//
// vtable 形状对齐 `AnimatedTexture`（client.dll RVA 0x53cb48，7 槽）的前 5 槽；
// 实测（探针 v5）5 槽即可被引擎正常驱动：Init / OnBind / Release / GetMaterial / 标量删除析构。

#[repr(C)]
struct ProxyVtable {
    init: unsafe extern "thiscall" fn(*mut ProxyObject, *mut c_void, *mut c_void) -> u8,
    on_bind: unsafe extern "thiscall" fn(*mut ProxyObject, *mut c_void),
    release: unsafe extern "thiscall" fn(*mut ProxyObject),
    get_material: unsafe extern "thiscall" fn(*mut ProxyObject) -> *mut c_void,
    scalar_deleting_dtor: unsafe extern "thiscall" fn(*mut ProxyObject, u32),
}

/// 交给引擎的 `IMaterialProxy` 实例（每个 VMT 代理实例一个）。
///
/// 由 `l4nrp_create`（`extern "C" fn() -> *mut c_void`）用 Rust 堆分配；引擎只使用首字段的
/// vtable 指针并调用虚函数，`proxy` 里装着真正的计算逻辑。
/// 生命周期：`Release()` 只置 `released` 标记，由 `run_active_proxies` 每帧
/// `purge_released` 统一回收（先摘 `ACTIVE` 再 `Box::from_raw`，避免悬垂）。
#[repr(C)]
struct ProxyObject {
    vtable: *const ProxyVtable,
    material: *mut c_void,
    /// 引擎已调用 `Release()`。
    released: AtomicBool,
    /// 本对象的代理实例；注册表未命中时保持 `None`（空转但不崩）。
    proxy: Option<Box<dyn Proxy>>,
}
// 对象在渲染线程创建、材质卸载时（主线程）Release，指针仅在裸指针语义下传递
unsafe impl Send for ProxyObject {}

/// 所有存活对象的登记表（用于每帧回收已 `Release` 的对象）。
struct LiveList(Vec<*mut ProxyObject>);
unsafe impl Send for LiveList {}
static LIVE: Mutex<LiveList> = Mutex::new(LiveList(Vec::new()));

static PROXY_VTABLE: ProxyVtable = ProxyVtable {
    init: po_init,
    on_bind: po_on_bind,
    release: po_release,
    get_material: po_get_material,
    scalar_deleting_dtor: po_scalar_deleting_dtor,
};

/// 引擎 `CMaterialProxyDict` 的 createFn：无参，返回新建的代理对象（失败返回空）。
///
/// 用 Rust 堆分配（而非 client.dll 的 `operator new`）：分配与释放两端都在本插件手里
/// （`l4nrp_create` / `purge_released`），不存在跨分配器不匹配。
unsafe extern "C" fn l4nrp_create() -> *mut c_void {
    let obj = Box::new(ProxyObject {
        vtable: &raw const PROXY_VTABLE,
        material: core::ptr::null_mut(),
        released: AtomicBool::new(false),
        proxy: None,
    });
    let p = Box::into_raw(obj);
    LIVE.lock().unwrap_or_else(|e| e.into_inner()).0.push(p);
    p.cast::<c_void>()
}

/// 从 KeyValues 节点取键名（引擎 `GetKeyName`，materialsystem RVA 0x75b90）。
/// 引擎传给 `Init` 的 `kv` 就是该代理自己的节点，其键名即代理名。
unsafe fn kv_key_name(node: *mut c_void) -> Option<String> {
    let get_name_p = ms_fn(0x75b90);
    if get_name_p.is_null() || node.is_null() {
        return None;
    }
    let get_name: GetKeyNameFn = transmute(get_name_p);
    let p = get_name(node);
    if p.is_null() {
        return None;
    }
    Some(CStr::from_ptr(p).to_string_lossy().into_owned())
}

unsafe extern "thiscall" fn po_init(
    this: *mut ProxyObject,
    material: *mut c_void,
    kv: *mut c_void,
) -> u8 {
    if this.is_null() {
        return 1;
    }
    (*this).material = material;
    let name = kv_key_name(kv).unwrap_or_default();
    // 先在锁内取出构造器，再释放锁（构造过程会碰 KeyValues / 材质变量，不宜持锁）
    let new_proxy = {
        let reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
        reg.iter().find(|e| e.name == name).map(|e| e.new_proxy)
    };
    let Some(new_proxy) = new_proxy else {
        // 不在注册表里（理论上不会发生）：接受但空转，避免引擎走 DeleteProxy 释放路径
        crate::log(&format!("engine proxy: '{name}' not in registry, accepted as no-op"));
        return 1;
    };
    let mut proxy = new_proxy();
    parse_proxy_params(&mut *proxy, kv);
    let per_frame = proxy.per_frame();
    let bind_err = proxy.bind(material).err();
    (*this).proxy = Some(proxy);
    crate::log(&format!(
        "engine proxy: '{name}' Init material=0x{:x} per_frame={per_frame}",
        material.addr()
    ));
    if let Some(e) = bind_err {
        crate::log(&format!("engine proxy: '{name}' initial bind failed: {e}"));
    }
    if per_frame {
        register_active(this, material);
    }
    1
}

/// 引擎在材质每次绑定时调用。本插件仍在 D3D `EndScene` 里驱动 per-frame 代理
/// （`l4nrp_delay_set` 的计时器需要每帧心跳，不能只在材质被渲染时才跑），故此处空转。
unsafe extern "thiscall" fn po_on_bind(_this: *mut ProxyObject, _entity: *mut c_void) {
    // 刻意留空：per-frame 代理仍由 D3D `EndScene` hook 驱动（见 `run_active_proxies`），
    // `l4nrp_delay_set` 的计时器依赖那个每帧心跳。引擎只在材质 bind 时调本槽，
    // 用它驱动会漏帧，故不在此处执行 bind。
}

/// 引擎在材质卸载时调用：只打标记，实际回收交给 `run_active_proxies`（见 `ProxyObject`）。
unsafe extern "thiscall" fn po_release(this: *mut ProxyObject) {
    if this.is_null() {
        return;
    }
    if !(*this).released.swap(true, Ordering::AcqRel) {
        crate::log("engine proxy: Release");
    }
}

unsafe extern "thiscall" fn po_get_material(this: *mut ProxyObject) -> *mut c_void {
    if this.is_null() {
        core::ptr::null_mut()
    }
    else {
        (*this).material
    }
}

/// 标量删除析构：引擎正常路径不会调用（我们总是让 `Init` 返回 true），
/// 真被调用时同样只打标记，交给每帧回收，避免二次释放。
unsafe extern "thiscall" fn po_scalar_deleting_dtor(this: *mut ProxyObject, _flags: u32) {
    po_release(this);
}

/// 把所有已注册代理登记进 `client.dll` 的 `CMaterialProxyDict`，返回登记个数。
///
/// 代理名用 `Box::leak` 永久持有：`AddProxy` 是否复制名字未经验证，
/// 泄漏十几个短字符串换取「绝不悬垂」。
///
/// # Safety
/// 必须在 `client.dll` 已加载后调用（`OnModuleLoaded("client")`）。
pub unsafe fn register_engine_proxies() -> Result<usize, PluginError> {
    let factory = crate::engine::material_proxy_factory()?;
    let names: Vec<&'static str> = {
        let reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
        reg.iter().map(|e| e.name).collect()
    };
    let create_fn = (l4nrp_create as *const ()).cast::<c_void>();
    let mut n = 0usize;
    for name in names {
        let Ok(cname) = CString::new(name) else {
            crate::log(&format!("engine proxy: '{name}' contains NUL, skipped"));
            continue;
        };
        let leaked: &'static CStr = Box::leak(cname.into_boxed_c_str());
        match crate::engine::add_material_proxy(factory, leaked, create_fn) {
            Ok(()) => n += 1,
            Err(e) => crate::log(&format!("engine proxy: AddProxy('{name}') failed: {e}")),
        }
    }
    Ok(n)
}

// ---------- 每帧执行（D3D EndScene 触发） ----------
struct ActiveProxy {
    // 唯一 id：同一材质可注册多个 per-frame 代理（各自独立条目，参数隔离）
    id: u64,
    /// 拥有该条目的引擎侧代理对象（`l4nrp_create` 分配，`Release` 后由 purge 回收）。
    owner: *mut ProxyObject,
    material: *mut c_void,
}
// material 指针仅在渲染线程内使用，手动标记 Send 以放入 Mutex
unsafe impl Send for ActiveProxy {}
static ACTIVE: Mutex<Vec<ActiveProxy>> = Mutex::new(Vec::new());
static NEXT_ACTIVE_ID: AtomicU64 = AtomicU64::new(1);

/// 登记一个需每帧执行的代理（引擎 `Init` 里调用，`owner` 即该引擎代理对象）。
/// 同一材质可注册多个不同代理（各自独立条目）；**不能按材质去重**，否则同一材质上
/// 多个 per-frame 代理只会保留最后一个（曾导致 `l4nrp_print_variable` 被
/// `l4nrp_is_in_range` 替换而不执行，见 AGENTS.md 已知问题）。
unsafe fn register_active(owner: *mut ProxyObject, material: *mut c_void) {
    let mut a = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    let id = NEXT_ACTIVE_ID.fetch_add(1, Ordering::Relaxed);
    a.push(ActiveProxy { id, owner, material });
}

/// 回收已被引擎 `Release` 的代理对象：先从 `ACTIVE` 摘除条目，再释放内存。
/// 锁序固定为 `LIVE → ACTIVE`（`register_active` 只取 `ACTIVE`，不反向）。
fn purge_released() {
    let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    let mut keep: Vec<*mut ProxyObject> = Vec::with_capacity(live.0.len());
    for &p in live.0.iter() {
        if p.is_null() {
            continue;
        }
        if unsafe { (*p).released.load(Ordering::Acquire) } {
            {
                let mut a = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
                a.retain(|e| e.owner != p);
            }
            // SAFETY: 指针来自 `l4nrp_create` 的 `Box::into_raw`；本函数在本帧 items 快照
            // 之前运行，且已摘除全部指向它的 ACTIVE 条目 —— 此刻无人使用。
            unsafe { drop(Box::from_raw(p)) };
        }
        else {
            keep.push(p);
        }
    }
    live.0 = keep;
}

/// D3D 每帧回调：先回收引擎已 `Release` 的对象，再对所有活动代理执行 `bind`
/// （读当前材质变量 → 计算 → 写回）。
/// 注意：须先复制指针并释放 `ACTIVE` 锁再逐个 `bind`，避免锁重入死锁（见 AGENTS.md）。
pub fn run_active_proxies() {
    // 先触发到期的计时器（l4nrp_delay_set 生成的）
    run_timers();
    // 回收引擎已 Release 的对象（必须在生成 items 快照之前，避免用到已释放内存）
    purge_released();
    let items: Vec<(u64, *mut c_void, *mut ProxyObject)> = {
        let a = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        a.iter().map(|e| (e.id, e.material, e.owner)).collect()
    };
    // bind 返回 Err = 材质失效/变量缺失 → 从活动表移除该条目
    let mut stale: Vec<u64> = Vec::new();
    for (id, m, owner) in items {
        if owner.is_null() {
            stale.push(id);
            continue;
        }
        // SAFETY: owner 由 `purge_released` 保证存活（已 Release 的在本帧开始时被摘除/释放）
        let obj = unsafe { &mut *owner };
        let Some(proxy) = obj.proxy.as_mut() else {
            stale.push(id);
            continue;
        };
        if let Err(e) = unsafe { proxy.bind(m) } {
            crate::log(&format!(
                "material '{}' error: {}",
                unsafe { get_name(m) }.unwrap_or("?".into()), e
            ));
            stale.push(id);
        }
    }
    if !stale.is_empty() {
        let mut a = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        a.retain(|e| !stale.contains(&e.id));
    }
}

// ---------- 计时器注册表（l4nrp_delay_set 生成 / l4nrp_delay_abort 中断） ----------
struct ActiveTimer {
    // 注：handle 不存于此（作为 HashMap 的 key，见 TIMERS）
    material: *mut c_void,
    output_n: CString, // 到期后把 value 写入此变量
    value: i32,        // 触发时快照的整型值（语义：到期写入的是触发时刻的值）
    end: Instant,
}
// material 指针仅在渲染线程内使用，手动标记 Send 以放入 Mutex
unsafe impl Send for ActiveTimer {}
/// 计时器注册表：key = UUID v4 **字符串**手柄（生成时 `to_string` 一次，之后 `abort`/查询
/// 直接按字符串匹配，无需再解析）。`LazyLock` 作懒初始化（`HashMap::new` 非常量可构造）。
static TIMERS: LazyLock<Mutex<HashMap<String, ActiveTimer>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 启动一个计时器：`delay_ms` 后（由 `run_timers` 每帧触发）把 `value`（触发时快照的**整型值**）
/// 写入 `output`。返回 **UUID v4 字符串手柄**，供中断/查询。
///
/// 语义说明：`value` 在**启动（触发）时**由调用方读取并快照，到期写入的是那一刻的值；
/// 延迟期间源变量再变化不影响本次输出。
pub fn start_timer(material: *mut c_void, output_n: CString, value: i32, delay_ms: u64) -> String {
    let h = Uuid::new_v4().to_string();
    let mut ts = TIMERS.lock().unwrap();
    ts.insert(h.clone(), ActiveTimer {
        material,
        output_n,
        value,
        end: Instant::now() + std::time::Duration::from_millis(delay_ms),
    });
    h
}

/// 按手柄（UUID v4 字符串）中断计时器。返回是否找到并移除。
pub fn abort_timer(handle: &str) -> bool {
    TIMERS.lock().unwrap().remove(handle).is_some()
}

/// 指定手柄的计时器是否仍在运行。
pub fn timer_active(handle: &str) -> bool {
    TIMERS.lock().unwrap().contains_key(handle)
}

/// 每帧触发到期的计时器：把触发时快照的整型值 `value` 写入 `output` 并移除。
/// 先取出到期项再释放锁执行，避免持锁调用引擎（见 AGENTS.md 锁注意事项）。
fn run_timers() {
    // extract_if：持锁消费掉到期条目，key/value 所有权移出（零复制），
    // 无 Key 克隆、无中间集合分配。语句结束即释放锁，随后在锁外执行引擎写入。
    let now = Instant::now();
    let fired: Vec<ActiveTimer> = TIMERS.lock().unwrap()
        .extract_if(|_, t| now >= t.end)
        .map(|(_, t)| t)
        .collect();
    for t in fired {
        // 防悬垂：材质已不可读（被引擎卸载/替换）则丢弃该计时器，避免解引用坏指针崩溃
        if crate::kv::test_readable(t.material).is_err() {
            continue;
        }
        unsafe {
            match find_var(t.material, &t.output_n) {
                Ok(out) => match set_int(out, t.value) {
                    Ok(()) => (),
                    Err(e) => crate::log(&e.to_string()),
                },
                Err(e) => crate::log(&e.to_string()),
            }
        }
    }
}

type FirstChildFn = unsafe extern "fastcall" fn(*mut c_void) -> *mut c_void;
type NextSiblingFn = unsafe extern "fastcall" fn(*mut c_void) -> *mut c_void;
type GetKeyNameFn = unsafe extern "fastcall" fn(*mut c_void) -> *const c_char;

/// materialsystem.dll 模块基址。
///
/// `HMODULE` 本身就是模块映射基址（来自 C，见 Rust 文档「4. Get it from C」），
/// 自带 provenance，可直接做指针算术，无需降级成 `usize`。
fn ms_base() -> *const u8 {
    unsafe {
        GetModuleHandleA(s!("materialsystem.dll"))
            .map(|h| h.0.cast::<u8>().cast_const())
            .unwrap_or(core::ptr::null())
    }
}

/// 引擎函数地址 = 模块基址 + RVA（指针算术，全程不经过 `usize`）。
/// 模块未加载时返回空指针，调用方必须先判空。
unsafe fn ms_fn(rva: usize) -> *const u8 {
    let b = ms_base();
    if b.is_null() {
        core::ptr::null()
    } else {
        b.add(rva)
    }
}

/// 遍历代理名子键里的参数子键，逐个调用 `apply_kv`。
/// # Safety
/// `node` 为 `"Proxies"` 块里某个代理名子键。
unsafe fn parse_proxy_params(proxy: &mut dyn Proxy, node: *mut c_void) {
    // 先取入口指针并判空，再 transmute（函数指针不可为 null，不能靠 transmute 后判空）
    let first_child_p = ms_fn(0x75dc0);
    let next_sib_p = ms_fn(0x75dd0);
    let get_name_p = ms_fn(0x75b90);
    if first_child_p.is_null() || next_sib_p.is_null() || get_name_p.is_null() {
        return;
    }
    let first_child: FirstChildFn = transmute(first_child_p);
    let next_sib: NextSiblingFn = transmute(next_sib_p);
    let get_name: GetKeyNameFn = transmute(get_name_p);
    let mut arg = first_child(node);
    let mut guard = 0;
    while !arg.is_null() && guard < 32 {
        guard += 1;
        let name_ptr = get_name(arg);
        // +0x04 是值字符串指针（const char*），见 AGENTS.md
        let val_ptr = *(&raw const *arg).byte_add(0x04).cast::<*const c_char>();
        if !name_ptr.is_null() && !val_ptr.is_null() && crate::kv::test_readable(val_ptr.cast()).is_ok() {
            let name = CStr::from_ptr(name_ptr).to_string_lossy().into_owned();
            let val = CStr::from_ptr(val_ptr).to_string_lossy().into_owned();
            proxy.apply_kv(&name, &val);
        }
        arg = next_sib(arg);
    }
}

// ---------- D3D9 EndScene 每帧 hook（执行活动代理） ----------
// D3D9 COM 方法为 __stdcall，故 hook 用 extern "system"（见 AGENTS.md）
/// 原 EndScene 函数地址（来自 D3D vtable 槽，是外部函数指针，无 `&raw` 来源）。
static mut ORIGINAL_ENDSCENE: usize = 0;
/// 被我们改写的 vtable 槽（带 provenance，用于 `uninstall` 还原）。
static mut HOOKED_ENDSCENE_SLOT: *mut usize = core::ptr::null_mut();

type EndSceneFn = unsafe extern "system" fn(*mut c_void) -> i32;

unsafe extern "system" fn endscene_hook(this: *mut c_void) -> i32 {
    // 每帧：先执行活动代理，再透传原 EndScene
    run_active_proxies();
    let orig: EndSceneFn = transmute((&raw const ORIGINAL_ENDSCENE).read());
    orig(this)
}

/// patch D3D9 `IDirect3DDevice9::EndScene`（vtable 索引 42），每帧先执行活动代理再透传原函数。
pub unsafe fn install_d3d_endscene(device: *mut c_void) -> Result<(), PluginError> {
    if device.is_null() {
        return Err(PluginError::Unexpected(Box::from("Direct3D device not ready")));
    }
    // COM 对象首字段是 vtable 指针；用 `&raw const` 读取，不构造引用
    let vft: *const usize = *(&raw const *device).cast::<*const usize>();
    if vft.is_null() {
        return Err(PluginError::Unexpected(Box::from("Direct3D device not ready")));
    }
    // vtable 槽存的是函数地址（外部函数指针，无 `&raw` 来源）：读写全程用 usize，
    // 不把它当成可解引用的指针来回 cast。
    let slot: *mut usize = vft.add(42).cast_mut();
    let orig = *slot;
    let hook_addr = (endscene_hook as *const ()).addr();
    if orig == 0 || orig == hook_addr {
        return Err(PluginError::Unexpected(Box::from("IDirect3DDevice9::EndScene does not exist")));
    }
    let mut old = std::mem::zeroed();
    VirtualProtect(slot.cast(), 4, PAGE_EXECUTE_READWRITE, &mut old)?;
    ORIGINAL_ENDSCENE = orig;
    HOOKED_ENDSCENE_SLOT = slot;
    *slot = hook_addr;
    let mut t = std::mem::zeroed();
    VirtualProtect(slot.cast(), 4, old, &mut t).ok();

    Ok(())
}

/// 还原 `EndScene` vtable 槽（插件析构时调用）。
///
/// 引擎原生代理注册（`CMaterialProxyDict::AddProxy`）**无法撤销**：该工厂只有
/// CreateProxy / DeleteProxy / AddProxy 三个槽位，没有删除名字的接口；已实例化的
/// 代理对象也由引擎按材质生命周期调用 `Release()` 回收（见 `purge_released`）。
/// 因此这里只还原我们自己 patch 的 D3D vtable 槽。
pub unsafe fn uninstall() {
    let slot = (&raw const HOOKED_ENDSCENE_SLOT).read();
    if slot.is_null() {
        return;
    }
    let orig = (&raw const ORIGINAL_ENDSCENE).read();
    let mut old = std::mem::zeroed();
    VirtualProtect(slot.cast(), 4, PAGE_EXECUTE_READWRITE, &mut old).ok();
    *slot = orig;
    let mut t = std::mem::zeroed();
    let _ = VirtualProtect(slot.cast(), 4, old, &mut t);
    HOOKED_ENDSCENE_SLOT = core::ptr::null_mut();
}
