# AGENTS.md

本文件面向 AI 代理与开发者，收录本项目**修改代码前必须了解的关键上下文、逆向依据与注意事项**。
源代码注释只保留必要的一行说明，详细内容统一在此维护，修改后请同步更新本文件。

## 项目概述

Rust 编写的 L4N 插件（cdylib，x86/Windows），用于在 Left 4 Dead 2 中**注册自定义材质代理
（material proxy）**：在 VMT 的 `"Proxies" { "代理名" { <参数> <值> } }` 块里写代理名，即可触发
Rust 回调，回调内可读写该材质的 VMT 变量（变色、比较运算等）。

- 每个代理是**一个独立 Rust struct**（参数相互隔离），实现 [`Proxy`](src/kv.rs) trait
- 注册用泛型 `material::register_proxy::<T>("代理名")`
- 导出入口 `GetL4NPluginInstance`（left4neko 调用），见 [`lib.rs`](src/lib.rs)
- 目标平台：`i686-pc-windows-msvc`（32 位），edition 2024，`cdylib` crate-type

## 目录结构

| 文件 | 职责 |
|---|---|
| [`src/lib.rs`](src/lib.rs) | 插件入口：IL4NPlugin 虚表/实现、演示代理定义、注册与 hook 安装时序 |
| [`src/engine.rs`](src/engine.rs) | IMaterialSystem 绑定 + `FUN_10002d50`（proxy 解析函数）地址获取 |
| [`src/kv.rs`](src/kv.rs) | `Proxy` trait 定义 + 内存可读性检查（`is_readable`） |
| [`src/material.rs`](src/material.rs) | 代理注册表、KeyValues 解析、detour hook、D3D EndScene 每帧执行 |
| [`src/util.rs`](src/util.rs) | 通用小工具：`RelativeCompare`（f32 相对比较，容差 1e-6），供比较类代理复用 |

## 逆向背景（为什么不能"正常"创建代理对象）

**left4neko 深 hook 了材质系统**（patch 了 `GetMaterialProxyFactory` 及 proxy 解析链）。
若插件创建自定义 `IMaterialProxy` 对象并放入材质 proxy 数组，left4neko 会把它按自己的
`CResultProxy` 布局处理而崩溃（`left4neko+0xE2AB8`：`FUN_100e2a60` 读坏指针）。

历史方案：
- v1–v3：链式 hook proxy factory 创建对象 → 崩溃
- v4：hook 引擎解析函数后透传原函数 → 原函数调用 left4neko `CreateProxy` 仍崩溃，且自解析
  KeyValues 布局遇到特殊材质（如 `Shadow`）会遍历越界
- **v5（当前）**：不创建代理对象，hook 引擎解析 `"Proxies"` 块的函数 → 稳定生效、不崩溃

## v5 实现机制

### 引擎真实 proxy 协议（逆向确认）

materialsystem.dll `FUN_10002d50`（RVA `0x2d50`）是引擎解析 VMT `"Proxies"` 块的函数，
`thiscall(this=CMaterial, param_1=材质 KeyValues)`：

- 内部 `FUN_10076020(kv,"Proxies",0)`（FindKey）找 `"Proxies"` 块；
- 遍历其子键，对每个代理名调 `ProxyFactory->CreateProxy(name)`（**factory vtable[0]**）；
- 创建后 `Init`（参数与 `material+0x80` 相关），成功则存入材质代理数组（`this+0x28`，计数
  `this+0x23`）；
- 引擎只在 `GetProxyFactory()` 非空且找到块时才做事，否则直接返回。

> left4neko 深 hook 了 `CreateProxy`（factory vtable[0]），传入未知名会按 `CResultProxy` 布局处理
> → 崩溃。因此我们的代理名**不能**落到这条路径，必须由本插件在引擎解析时拦截并摘除。

1. **hook `FUN_10002d50`**（materialsystem RVA `0x2d50`，`thiscall(this=CMaterial, param_1=材质 KeyValues)`）
   入口。入口 9 字节完整指令为 `55 8B EC 81 EC 08 04 00 00`（PUSH EBP; MOV EBP,ESP; SUB ESP,0x408），
   trampoline 复制 9 字节后 JMP 回 `+9`。
2. **用引擎自己的 KeyValues 函数**处理 `"Proxies"` 块（不再自解析布局，避免 `Shadow` 式越界）：

   | 引擎函数 | 作用 | RVA |
   |---|---|---|
   | `FUN_10076020` | FindKey(kv, name, create) | `0x76020` |
   | `FUN_10075dc0` | FirstChild | `0x75dc0` |
   | `FUN_10075dd0` | NextSibling | `0x75dd0` |
   | `FUN_10075b90` | GetKeyName | `0x75b90` |

3. 命中注册表 → 创建 `Box<dyn Proxy>`，把 `"Proxies"` 块参数（键值对）经 `apply_kv` 注入，再
   `bind(material)`。
4. 处理我们的代理后把它从 `"Proxies"` 链表**摘除**，且**总是透传**原 `FUN_10002d50` —— 引擎只
   处理剩余原版/L4N 代理（不会再把我们的代理名传给 left4neko `CreateProxy` 而崩溃），**因此可与
   内置材质代理共存**（同一 `"Proxies"` 块里 `Sine`/`Multiply` 与 `l4nrp_*` 并存）。
   - 摘除链表节点：前驱 `m_pPeer(+0x1c) = 当前.m_pPeer`；若为首子键则 `proxies.m_pSub(+0x20) =
     当前.m_pPeer`，并把当前节点 `m_pPeer` 置空防残留。

## KeyValues 真实布局（实测，非 SDK2013 假设）

| 偏移 | 字段 |
|---|---|
| `+0x00` | `m_iKeyName`（symbol） |
| `+0x04` | **值字符串指针 `const char*`**（实测确认，非 SDK 假设的 symbol！） |
| `+0x1c` | `m_pPeer`（兄弟链表） |
| `+0x20` | `m_pSub`（子链表） |
| `+0x24` | `m_pChain` |

> 旧注释里 `+0x08/+0x0c/+0x10`（SDK2013 假设）是**错的**；`+0x04` 也是**值字符串指针**而非
> `m_iValue(symbol)`。曾误当 symbol 传给 `KeyValuesSystem::GetStringForSymbol` 导致返回坏指针
> → `strlen` 崩溃，现已直接读字符串指针。

## IMaterial / IMaterialVar vtable 偏移（L4D2 materialsystem.dll，逆向确认）

- `IMaterial::GetName` = `+0x00`（thiscall 返回材质名 `const char*`，插件里用 `material::get_name(mat)
  -> Option<String>`）
- `IMaterial::FindVar` = `+0x2c`，签名 `FindVar(const char*, bool* found, i32)`
- `IMaterialVar`：
  - `SetFloatValue` `+0x0c`、`SetIntValue` `+0x10`、`SetStringValue` `+0x14`
  - `SetVecValue(float*, n)` `+0x30`、`SetVecComponentValue` `+0x64`
  - `GetStringValue` `+0x18`、`GetIntValue` `+0x68`、`GetFloatValue` `+0x6c`、`GetVecValue` `+0x70`
  - `GetVecValueInternal` `+0x74`（返回内部 `float*`，即 `this+3`）、`VectorSize` `+0x78`（返回分量数）
  - **没有 `GetVecComponentValue`（读单分量）槽位** —— 只有写单分量的 `SetVecComponentValue(+0x64)`；
    读单分量用 `get_vec` 取下标，或 `GetVecValueInternal` 返回指针后读 `[n]`（插件提供
    `material::get_vec_component(var, index)` 封装前者）。

> **`GetStringValue` = `+0x18` 的逆向依据**（`ghidra_matvar_getstring*.txt`）：引用诊断字符串
> `"CMaterialVar::GetStringValue: Unknown material var type"` 的实现 `FUN_10019e70`（case 1:
> `return param_1[1]` 直接返回内部字符串指针）位于 vtable@0x1009d274 的 **+0x18** 槽位；该 vtable
> 起点由已验证偏移交叉确认（+0x0c=SetFloat、+0x14=SetString、+0x6c=GetFloat、+0x70=GetVec）。
> 顺带确认 `GetIntValue` = `+0x68`（`FUN_10019c60: return param_1[2]`）。

> `FindVar` 只对**已在 VMT 声明**的变量有效（引擎只为声明过的变量创建 `IMaterialVar`）。

## 每帧执行（D3D9 EndScene hook）

原版代理每帧 `OnBind`，而我们的代理若只在材质加载时触发一次，对依赖每帧输入的持续计算会"无效"。
因此插件 **hook D3D9 `EndScene`（vtable 索引 42）**：把 `per_frame() == true` 的代理注册到活动表，
每帧对已注册材质再次执行 `bind`。

> **D3D9 COM 方法为 `__stdcall`**（逆向依据：`ghidra_d3d.txt`，left4neko Present hook
> `LAB_100a8b90`：this 从栈取、`RET n` 清栈），故 EndScene hook 用 `extern "system"`（x86 即 stdcall），
> **不能用 thiscall**。

## 线程 / 锁 / 内存安全注意事项

- **Mutex 重入死锁**：`run_active_proxies` 必须**先复制 (material, proxy 指针) 并释放 `ACTIVE` 锁**，
  再逐个 `bind`。若持有锁期间 `bind` 经引擎回调（如创意工坊 Mod 刷新材质 → `apply_proxies` →
  `register_active`）再次对 `ACTIVE` 加锁，会造成 `std::sync::Mutex` 重入死锁（表现为"无响应"）。
- **`ActiveProxy` 手动 `unsafe impl Send`**：`material` 指针仅在渲染线程内传递与使用（不跨线程拥有）。
- **`bind` 返回 `false` = 材质失效/所需变量缺失** → 调用方从活动表移除该代理（防止悬垂 + 引擎
  `FindVar` 警告刷屏）。
- **读指针前先 `is_readable` 检查**（`VirtualQuery`）：防止 `strlen`/`CStr` 读到坏指针崩溃。
  `is_readable` 复用 32 位 `MEMORY_BASIC_INFORMATION` 布局（`Mbi32`，x86）。
- **Detour hook**：`hook_function` 改写目标前 5 字节为 `E9 rel32`（近 JMP）并保存原字节；
  `make_trampoline` 复制目标前 `patch_len` 字节到 `VirtualAlloc` 分配的可执行内存再 JMP 回
  `target+patch_len`；`uninstall` 用保存的 5 字节还原入口。

## 代理编写约定

- 每个代理独立 struct（参数隔离），实现 [`Proxy`](src/kv.rs) trait：
  - `apply_kv(name, value)`：`"Proxies"` 块参数填充（参数名不区分大小写）
  - `bind(material)`：读写材质 VMT 变量；返回 `false` 表示材质失效/变量缺失，从活动表移除
  - `per_frame()`：是否每帧执行（依赖每帧变化的输入，如 `Sine` 输出，应覆写为 `true`）

trait 定义（[`src/kv.rs`](src/kv.rs)）：

```rust
pub trait Proxy: Send {
    fn apply_kv(&mut self, name: &str, value: &str);          // VMT "Proxies" 块参数填充
    unsafe fn bind(&mut self, material: *mut c_void) -> bool; // 触发动作；返回 false = 材质失效/变量缺失
    fn per_frame(&self) -> bool { false }                     // 是否每帧执行（持续计算）
}
```

> `bind` 返回 `false` 时，`run_active_proxies` 会从**活动表移除**该条目 —— 防止材质被引擎
> 卸载/替换后活动表持有悬垂指针导致 "No such variable" 刷屏或崩溃。

注册（泛型，见 [`src/lib.rs`](src/lib.rs) `try_bind_and_install`）：

```rust
material::register_proxy::<ColorRampProxy>("l4nrp_color_ramp");
material::register_proxy::<LogPulseProxy>("l4nrp_log_pulse");
material::register_proxy::<ForceRedProxy>("l4nrp_force_red");
material::register_proxy::<DoesEqualProxy>("l4nrp_does_equal");
material::register_proxy::<CompareProxy>("l4nrp_compare");
material::register_proxy::<IsInRangeProxy>("l4nrp_is_in_range");
material::register_proxy::<PrintVariable>("l4nrp_print_variable");
```

> 注意：`l4nrp_color_ramp` / `l4nrp_log_pulse` / `l4nrp_force_red` 三个演示代理目前仅在
> `#[cfg(debug_assertions)]`（debug 构建）下注册，release 构建不包含（见 [`src/lib.rs`](src/lib.rs)
> `try_bind_and_install`）。
>
> **整数结果用 `SetInt`**：比较类代理（`does_equal` / `compare` / `is_in_range`）输出 0/1（`compare`
> 为 -1/0/1）的结果变量用 `material::set_int` 写入（输入比较仍用 `get_float`）；`PrintVariable`
> 用 `get_int` / `get_string` 读取 int / string 类型变量。

- **CString 缓存**：`per_frame` 代理在 struct 里缓存变量名 `CString`（`cstr_of` 构造），避免每帧
  反复堆分配；`apply_kv` 更新变量名时用辅助函数 `set_kv(&mut dst, &mut c, value)` 同步重建缓存
  （位于 [`src/lib.rs`](src/lib.rs)，可复用）。
- 演示代理见 [`src/lib.rs`](src/lib.rs)：`ColorRampProxy` / `LogPulseProxy` / `ForceRedProxy` /
  `DoesEqualProxy` / `CompareProxy` / `IsInRangeProxy` / `PrintVariable`。
- 完整 VMT 用法示例见项目根目录 [`Example.vmt`](Example.vmt)（涵盖全部 7 个代理及其参数）。

## 构建 / 部署 / 验证

```powershell
cargo build --release   # 目标 i686-pc-windows-msvc
```

- 部署：复制 DLL 到游戏安装目录 `bin/neko/plugins`（必须与 `left4neko.dll` 同目录的 `neko/plugins`，
  **不是**工作区里的 `bin/neko/plugins`）。left4neko 用 `std::filesystem::directory_iterator` 遍历
  该目录所有 DLL 并调用 `GetL4NPluginInstance`。
- 日志：`l4n_material_proxy_plugin.log`（当前工作目录）+ 引擎控制台 `[l4n-proxy]` 前缀输出
  （tier0.dll `Msg`，fallback `OutputDebugStringA`）。
- 安装时序：`OnModuleLoaded("client")` 或 D3D 首帧（`on_d3d_device_created`）时调用
  `try_bind_and_install`（幂等，成功安装后不再重复）；必须先等 `materialsystem.dll` 加载。

## 已知问题

- v5.1：处理我们的代理后从 `"Proxies"` 链表**摘除**再透传，因此可与原版/L4N 材质代理共存（同一
  `"Proxies"` 块里既有 `Sine`/`Multiply` 等内置代理，也有 `l4nrp_*`）。摘除只改一次材质 KeyValues
  树（仅移除我们的代理节点），对引擎其余逻辑无影响。
- v5.2：`per_frame()` 代理注册到活动表并在 D3D EndScene 每帧执行。EndScene 回调里**先复制指针再
  释放 `ACTIVE` 锁**后才执行 `bind`（避免持锁期间 `bind` 内部重新入锁导致 Mutex 重入死锁 ——
  表现为游戏无响应）。材质被引擎销毁后活动表条目可能**悬垂**（暂未做清理）；若切换地图/重载材质
  后崩溃，请将该材质改用一次性代理，或后续增加材质析构回调清理活动表。
- v5.3：`Proxy::bind` 返回 `bool`；材质被引擎卸载/替换（活动表持有悬垂指针）或所需输出变量缺失时
  返回 `false`，`run_active_proxies` 会从活动表**移除失效条目**，避免 "No such variable" 刷屏 /
  崩溃。
- v5.4：`register_active` **不能按材质去重**。曾按 `material` 去重（同一材质已有则替换），导致同一
  材质上注册多个 per-frame 代理时只有最后一个生效（如 `l4nrp_print_variable` 被
  `l4nrp_is_in_range` 替换而不执行，表现为"没有每帧输出"）。现改为每条目分配唯一 `id`，同一材质
  可挂多个独立代理；失效条目按 `id` 移除。
