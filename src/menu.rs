//! L4N 插件接口 v2：HUD 菜单（`IL4NPlugin::RequestHudMenu`）。
//!
//! # 逆向依据（`left4neko.dll`，v2.43.1）
//!
//! - `PluginManager::BuildHudMenu`（`0x100d2970`）遍历插件表（每项 12 字节：
//!   `{+0, +4 实例指针, +8 GetInterfaceVersion() 返回值}`），对 `version >= 2`
//!   的插件 `PUSH 1` 后调用虚表槽位 8（字节偏移 `0x20`），即 `RequestHudMenu(true)`。
//!   **返回非空**即以该字符串为标题建立一个插件菜单项；返回 `nullptr` 则跳过该插件。
//! - 用户打开该菜单时 `FUN_100d35a0` 以 `PUSH 0` 再次调用槽位 8
//!   （`RequestHudMenu(false)`），把返回的字符串交给 `FUN_10043930`（`tyti::vdf` 解析器）
//!   解析，再由 `FUN_10048060` 逐条构建菜单项。
//! - 条目类型（`FUN_10048060` 的 `param_3 = 1` 分支）：
//!   - `"cmd"`：控制台命令，激活时由 `FUN_10049300` 交给引擎执行。
//!   - `"cvar"`：控制台变量，可带 `"delta"`（默认 `1.0`）、`"min"`、`"max"`；
//!     激活时由 `FUN_10049350` 取「当前值 + delta」并夹到 `[min, max]` 后写回。
//!   - `"callback"`：值为函数地址字符串（`std::stoul`，base 0 → 支持 `0x` 前缀），
//!     激活时由 `FUN_100494f0` 调用 `const char* __cdecl f(void* user_data)`。
//!   - `"user_data"`：可选，同上按地址字符串解析后作为 `callback` 的参数。
//!   - 三者都没有的条目会被静默忽略（不产生菜单项）。
//! - `callback` 的返回值：`nullptr` = 不展开子菜单；否则是子菜单的 KeyValues 文本，
//!   递归走同一条解析路径（所以子菜单里同样能用 `cmd`/`cvar`/`callback`）。
//! - 字符串一律 **UTF-8**（neko 自带的 `neko/config_template.vdf` 即纯 UTF-8 无 BOM）。
//!
//! # KV 结构：必须**恰好一个顶层对象**
//!
//! `FUN_10043930` 解析后会数顶层条目：`uVar6 = (int)local_70 - (int)local_74 >> 2;`
//! - `uVar6 == 1` → `FUN_100471e0(*local_74)`，直接取用那**唯一**的顶层对象；
//! - `uVar6 >= 2` → 走另一条合并分支。
//!
//! `FUN_100d35a0` 随后把该对象交给 `FUN_10048060`，后者遍历的是**它的子条目**，
//! 并把对象自身的键名当作菜单标题。因此正确的形状是「一个顶层对象 = 标题，
//! 其子条目 = 菜单项」，与头文件示例一致：
//!
//! ```text
//! "我的标题" {
//!     "项目1" { "cvar" "my_cvar_name" }
//! }
//! ```
//!
//! 返回两个及以上顶层对象会落进那条未经实测的分支，故本模块只生成一个。
//!
//! # 安全约定
//!
//! 本模块的代码运行在游戏主线程，且 crate 以 `panic = "abort"` 构建
//! （任何 panic 都会直接终止游戏进程），因此这里不使用任何会 panic 的
//! `unwrap()` / `expect()`；锁中毒时走 `into_inner()` 继续，而不是 `unwrap()`。

use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// 菜单标题（`RequestHudMenu(true)` 的返回值）。
///
/// 返回非空即向 left4neko 声明「本插件需要一个 HUD 菜单入口」，
/// 该字符串会成为插件菜单里的一个条目名。
static TITLE: &CStr = c"L4NRP 材质代理";

/// 传给菜单回调的 `user_data`（可选键的演示：回调里会把它原样打进日志）。
static CALLBACK_USER_DATA: &CStr = c"L4NRP HUD menu callback";

/// 记录标题是否已被索取过，用于「只打一次」的诊断日志。
static TITLE_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 菜单结构文本（只构造一次：其中内嵌回调地址，进程内恒定）。
static MENU_KV: OnceLock<CString> = OnceLock::new();

/// 回调返回的子菜单文本。每次调用重新生成并覆盖，
/// 以保证返回给 left4neko 的指针在其 `strlen` + 解析期间始终有效。
static CALLBACK_KV: Mutex<Option<CString>> = Mutex::new(None);

/// `RequestHudMenu` 的实现。
///
/// - `request_title != 0`：返回标题字符串（非空 = 需要菜单入口）。
/// - `request_title == 0`：返回菜单结构的 KeyValues 文本（UTF-8、NUL 结尾）。
///
/// 返回的指针均指向进程生命期内有效的静态数据。
pub unsafe fn request(request_title: u8) -> *const c_char {
    if request_title != 0 {
        if !TITLE_REQUESTED.swap(true, Ordering::SeqCst) {
            crate::log("RequestHudMenu(title): 已向 left4neko 注册 HUD 菜单入口");
        }
        TITLE.as_ptr()
    }
    else {
        menu_kv().as_ptr()
    }
}

/// 构造（并缓存）菜单结构文本。
fn menu_kv() -> &'static CString {
    MENU_KV.get_or_init(|| {
        // 回调地址与 user_data 地址都以 `0x` 十六进制字符串写进 KV，
        // left4neko 侧用 `std::stoul(..., base = 0)` 解析回 32 位地址。
        let callback: unsafe extern "C" fn(*mut c_void) -> *const c_char = on_menu_callback;
        let callback_addr = (callback as *const ()).addr();
        let user_data_addr = CALLBACK_USER_DATA.as_ptr().addr();

        let kv = format!(
            "\"{title}\" {{\n\
             \t\"重新加载所有材质\" {{ \"cmd\" \"mat_reloadallmaterials\" }}\n\
             \t\"检查代理是否生效\" {{ \"cmd\" \"l4n_is_proxy_exist l4nrp_math\" }}\n\
             \t\"菜单横向偏移\" {{ \"cvar\" \"l4n_hudmenu_offset_x\" \"delta\" \"10\" \"min\" \"-500\" \"max\" \"500\" }}\n\
             \t\"已注册代理…\" {{ \"callback\" \"0x{callback_addr:x}\" \"user_data\" \"0x{user_data_addr:x}\" }}\n\
             }}\n",
            title = TITLE.to_string_lossy(),
        );
        crate::log(&format!(
            "RequestHudMenu(structure): callback=0x{callback_addr:x} user_data=0x{user_data_addr:x}"
        ));
        CString::new(kv).unwrap_or_default()
    })
}

/// 菜单回调：`const char* __cdecl f(void* user_data)`。
///
/// 打印已注册的代理，并返回一个子菜单——每个代理一条
/// `l4n_is_proxy_exist <name>`，点击即可在控制台确认该代理真的挂上了引擎。
/// 返回 `nullptr` 表示不展开子菜单。
unsafe extern "C" fn on_menu_callback(user_data: *mut c_void) -> *const c_char {
    if user_data.is_null() {
        crate::log("菜单回调被调用（user_data = null）");
    }
    else {
        let text = CStr::from_ptr(user_data.cast::<c_char>()).to_string_lossy();
        crate::log(&format!("菜单回调被调用（user_data = \"{text}\"）"));
    }

    let names = crate::material::registered_names();
    if names.is_empty() {
        crate::log("菜单回调：当前没有已注册的代理，不展开子菜单");
        return core::ptr::null();
    }
    crate::log(&format!("菜单回调：已注册 {} 个代理", names.len()));

    let mut kv = format!("\"已注册代理（{}）\" {{\n", names.len());
    for name in &names {
        kv.push_str(&format!(
            "\t\"{name}\" {{ \"cmd\" \"l4n_is_proxy_exist {name}\" }}\n"
        ));
    }
    kv.push_str("}\n");

    let Ok(text) = CString::new(kv) else {
        crate::log("菜单回调：子菜单文本含 NUL 字节，已跳过");
        return core::ptr::null();
    };
    // 锁中毒时继续用内层数据：本 crate 以 panic = "abort" 构建，不会留下中毒的锁，
    // 但这里依然不使用 `unwrap()`，避免任何 panic 路径。
    let mut slot = CALLBACK_KV.lock().unwrap_or_else(|e| e.into_inner());
    *slot = Some(text);
    match slot.as_ref() {
        Some(s) => s.as_ptr(),
        None => core::ptr::null(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 标题必须是合法 C 字符串且非空——返回非空才意味着「本插件需要菜单入口」。
    #[test]
    fn title_is_non_empty() {
        assert!(!TITLE.to_bytes().is_empty());
    }

    /// 菜单结构文本必须能被 `CString` 接受（无内嵌 NUL），且含有全部四类条目。
    #[test]
    fn menu_kv_is_well_formed() {
        let kv = menu_kv().to_string_lossy().into_owned();
        assert!(kv.contains("\"cmd\" \"mat_reloadallmaterials\""));
        assert!(kv.contains("\"cmd\" \"l4n_is_proxy_exist l4nrp_math\""));
        assert!(kv.contains("\"cvar\" \"l4n_hudmenu_offset_x\""));
        assert!(kv.contains("\"callback\" \"0x"));
        assert!(kv.contains("\"user_data\" \"0x"));
        // 大括号必须配平，否则 tyti::vdf 解析会失败。
        assert_eq!(kv.matches('{').count(), kv.matches('}').count());
        assert_eq!(kv.matches('"').count() % 2, 0);
    }

    /// 标题索取路径返回标题本身，结构索取路径返回 KV 文本，两者都非空。
    #[test]
    fn request_dispatches_on_flag() {
        let title = unsafe { request(1) };
        let structure = unsafe { request(0) };
        assert!(!title.is_null());
        assert!(!structure.is_null());
        assert_ne!(title, structure);
        assert_eq!(unsafe { CStr::from_ptr(title) }, TITLE);
        assert!(unsafe { CStr::from_ptr(structure) }
            .to_string_lossy()
            .contains('{'));
    }
}
