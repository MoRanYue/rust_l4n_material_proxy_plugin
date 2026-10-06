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
- 实现 **L4N 插件接口 v2**（`IL4NPlugin`）：`GetInterfaceVersion()` 返回 `2`，
  虚表槽位 8（字节偏移 `0x20`）为 `RequestHudMenu`，提供游戏内 HUD 菜单，见 [`src/menu.rs`](src/menu.rs)

## 目录结构

| 文件 | 职责 |
|---|---|
| [`src/lib.rs`](src/lib.rs) | 插件入口：IL4NPlugin 虚表/实现、注册与安装时序 |
| [`src/engine.rs`](src/engine.rs) | IMaterialSystem 绑定 + `client.dll` 的 `CMaterialProxyDict` 工厂（`material_proxy_factory` / `add_material_proxy`） |
| [`src/kv.rs`](src/kv.rs) | `Proxy` trait 定义 + 内存可读性检查（`is_readable`） |
| [`src/material.rs`](src/material.rs) | 代理注册表、引擎侧 `ProxyObject`（5 槽 vtable）、KeyValues 参数解析、`OnBind` 驱动的每帧执行、延迟计时器 |
| [`src/menu.rs`](src/menu.rs) | L4N 接口 v2 的 HUD 菜单（`RequestHudMenu`）：标题、KV 菜单结构、`callback` 子菜单 |
| [`src/util.rs`](src/util.rs) | 通用小工具：`RelativeCompare`（f32 相对比较，容差 1e-6），供比较类代理复用 |
| [`src/expr.rs`](src/expr.rs) | 表达式求值器（无依赖）：数学 + 比较（`== != < <= > >=`）+ 逻辑（`&& \|\| !`），供 `l4nrp_math` / `l4nrp_logic` 使用 |

## L4N 插件接口 v2（HUD 菜单）

`l4n_plugin.h` 的接口版本 2 在 `IL4NPluginV1` 的 8 个虚函数之后**只新增了一个槽位**：
`virtual const char* RequestHudMenu(bool request_title)`，即虚表槽位 8 / 字节偏移 `0x20`。

### 调用链（`left4neko.dll` v2.43.1 反汇编实测）

1. **插件加载器 `FUN_100d30e0`**：遍历 `neko/plugins` 目录，对每个 DLL 调
   `LoadLibrary` + `GetProcAddress("GetL4NPluginInstance")`，取实例后调用虚表槽位 1
   `GetInterfaceVersion()`，把结果存进插件表。**表项 12 字节**：
   `{+0, +4 插件实例指针, +8 GetInterfaceVersion() 返回值}`。
2. **`PluginManager::BuildHudMenu`（`FUN_100d2970`）**：遍历插件表，`CMP [EDI+8], 0x2` →
   `version < 2` 的插件**直接跳过**；`version >= 2` 才 `PUSH 0x1` 调用槽位 8
   （`RequestHudMenu(true)`）。返回 `nullptr` → 该插件没有菜单入口，跳过；
   返回非空 → 以该字符串为标题建立插件菜单项（插件实例存入菜单项 payload `+0x48`）。
3. **菜单展开 `FUN_100d35a0`**：用户打开菜单时，从 payload `+0x48` 取插件实例，
   `PUSH 0x0` 再次调用槽位 8（`RequestHudMenu(false)`），把返回的字符串交给
   `FUN_10043930`（`tyti::vdf` 解析器），再由 `FUN_10048060` 逐条构建菜单项。
4. **`callback` 调用 `FUN_100494f0`**：`PUSH [payload+0x4c]`（user_data）→
   `CALL [payload+0x48]`（callback）→ `ADD ESP, 0x4`，即
   **`const char* __cdecl f(void* user_data)`**（调用方清栈）。返回值非空则作为子菜单
   KV 再次走 `FUN_10048060`（`param_3 = 1`，递归支持嵌套 callback）。

### 菜单条目类型（`FUN_10048060`，`param_3 = 1` 分支）

| 键 | 解析 | 激活行为 |
|---|---|---|
| `"cmd"` | `HudMenuItemCmdexec` | `FUN_10049300` 把值字符串交给引擎执行 |
| `"cvar"` | `HudMenuItemConVar` | `FUN_10049350`：`g_pCVar->FindVar(name)`（虚表 `+0x34`），取「当前值 + `delta`」并夹到 `[min, max]` 后写回 |
| `"callback"` | `CustomCommand::_CallbackEntry` | `FUN_100494f0` 调用函数指针，返回值作为子菜单 |
| `"user_data"` | 同上（可选） | 作为 `callback` 的参数传入 |

- `"cvar"` 的 `"delta"` 默认 `1.0`，`"min"` 默认 `0`，`"max"` 默认 `1.0`。
- 三者都没有的条目会被**静默忽略**（不产生菜单项）。
- `"callback"` / `"user_data"` 的值由 `std::stoul(str, &endptr, 0)` 解析：
  **base = 0 即自动识别进制，所以 `0x` 十六进制前缀有效**；
  格式非法会抛 C++ 异常（`"invalid stoul argument"` / `"stoul argument out of range"`），
  因此这两个字符串必须严格合法。

### KV 结构：必须恰好一个顶层对象

`FUN_10043930` 解析后数顶层条目 `uVar6 = (int)local_70 - (int)local_74 >> 2`：

- `uVar6 == 1` → `FUN_100471e0(*local_74)`，直接取用那**唯一**的顶层对象；
- `uVar6 >= 2` → 走另一条**未经实测**的合并分支。

`FUN_100d35a0` 把该对象交给 `FUN_10048060`，后者遍历的是**它的子条目**，
并把对象自身的键名当作菜单标题。所以正确形状是「一个顶层对象 = 标题，其子条目 = 菜单项」，
与头文件示例一致。返回两个及以上顶层对象会落进未实测分支，故本插件只生成一个。

### 编码与 ABI 约定

- 返回的字符串必须是 **UTF-8**（`neko/config_template.vdf` 即纯 UTF-8 无 BOM）。
- `bool request_title` 在 MSVC x86 `thiscall` 下占**一个 32 位栈槽**，调用点实测为
  `PUSH 0x1` / `PUSH 0x0`，故 Rust 侧用 `u8` 接收。
- 返回的指针必须在 left4neko `strlen` + 解析期间保持有效：本插件返回 `static`/`OnceLock`
  里的 `CString` 指针，`callback` 的子菜单文本存进 `Mutex<Option<CString>>` 后返回其指针。

## 材质代理注册（v6：真正注册进引擎代理工厂）

**当前机制：把 `l4nrp_*` 直接注册进 `client.dll` 的 `CMaterialProxyDict`**，让引擎自己的 VMT
解析器去实例化它们 —— 不再 detour 引擎函数，也不再手动摘除 `"Proxies"` 链表节点。

### 工厂与注册 API（逆向确认）

| 项 | 位置 | 说明 |
|---|---|---|
| 工厂单例 getter | `client.dll` RVA `0x13ab40` | `__cdecl` 无参，惰性构造并返回 `&CMaterialProxyDict`（= `client+0x75e928`） |
| 工厂 vtable | `client.dll` RVA `0x576d94` | 恰 3 槽：`[0]` CreateProxy、`[1]` DeleteProxy、`[2]` AddProxy |
| `AddProxy` | vtable 槽 `[2]` | `__thiscall(this, const char* name, CreateFn create_fn)`，`RET 0x8` |

注册流程（`src/engine.rs` + `src/material.rs` 的 `register_engine_proxies`）：

1. `engine::material_proxy_factory()` 取单例；
2. 对注册表里每个代理名 `AddProxy(name, l4nrp_create)` —— 名字 `Box::leak` 故意泄漏
   （`AddProxy` 是否复制名字未验证）；`create_fn` 全部指向同一个 `l4nrp_create`；
3. 引擎解析 VMT `"Proxies"` 块时调 `CreateProxy(name)` → 得到我们的 `ProxyObject` → 调
   `Init(name, kv)` → 我们在 `Init` 里查 `REGISTRY`、`apply_kv` 注入参数、`bind` 一次；
   `per_frame()` 为真的代理**不再单独登记**，改由引擎后续的 `OnBind` 调用驱动（见下）。

**因此 `l4n_is_proxy_exist` 现在能查到 `l4nrp_*`**（实机验证过，见下）。

### 引擎解析链（`materialsystem.dll` RVA `0x2d50`，`FUN_10002d50`）

`thiscall(this=CMaterial, param_1=材质 KeyValues)`：

- `FUN_10076020(kv,"Proxies",0)`（FindKey）找 `"Proxies"` 块；
- 遍历其子键，对每个代理名调 `GetMaterialProxyFactory()->CreateProxy(name)`；
- **`Init` 的真实 ABI**（字节级确认，与 SDK 头不同）：

  ```
  10002df0  MOV ECX,[EDI]            ; proxy->vtable
  10002df2  MOV EDX,[ESI]            ; material->vtable
  10002df4  MOV EAX,[EDX+0x17c]      ; material vtable +0x17c → 名字字符串
  10002dfd  PUSH EBX                 ; 第 2 参 = KeyValues* 子键
  10002dfe  MOV ECX,ESI / CALL EAX   ; 取名字
  10002e05  PUSH EAX                 ; 第 1 参 = const char* 名字
  10002e08  MOV ECX,EDI / CALL EAX   ; proxy->Init(name, kv)
  10002e0c  TEST AL,AL               ; 返回 bool
  ```

  → **`bool __thiscall Init(const char* materialName, KeyValues* kv)`**，第 1 参是**名字字符串**
  而非 `IMaterial*`（`AnimatedTexture` 的 `Init` 以 `RET 0x8` 收尾，证实恰好 2 个栈参）。
- `Init` 返回 false 时引擎走 `factory->DeleteProxy(proxy)`；我们恒返回 true。
- 成功则存入材质代理数组（`this+0x28`，计数 `this+0x23`）—— 注意这两个字段由引擎在
  `ParseProxies` **尾段**才写入，`FindMaterial` 刚返回时读到的仍是 0（不是注册失败的证据）。

### 我们的 `ProxyObject`（`src/material.rs`）

**5 槽** vtable，对齐 `AnimatedTexture`（`client.dll` RVA `0x53cb48`，共 7 槽）**实测被引擎用到的前 5 槽**：

| 槽 | 字节偏移 | 我们的实现 | 语义 |
|---|---|---|---|
| 0 | `+0x00` | `po_init` | `Init(name, kv)`，`RET 0x8`，恒返回 1 |
| 1 | `+0x04` | `po_on_bind` | **per-frame 代理在此执行**（路线 A，见下） |
| 2 | `+0x08` | `po_release` | 置 `released` 标志 + 递增 `PENDING_RELEASE`，供 `purge_released` 回收 |
| 3 | `+0x0c` | `po_get_material` | 返回 `(*this).material` |
| 4 | `+0x10` | `po_scalar_deleting_dtor` | 标量删除析构（`RET 0x4`），与 `Release` 同样只置标志 |

> 只用 5 槽是**实测结论**（探针 v5 + v0.7.0 实机 E2E 均正常驱动），不是假设。`AnimatedTexture`
> 多出的槽 5（`0x1000e7e0` = `FLDZ; RET 0x4`，返 `0.0f`）与槽 6（`0x1000e7f0`，KV 风格 getter）
> **引擎从未在代理分发路径上调用过** —— 若将来出现未解释的崩溃，这是第一个要补的地方。

- **对象内存由 `Box::into_raw` 分配**（Rust 全局分配器）。**引擎从不释放我方对象** ——
  `FUN_10002ed0`（`materialsystem.dll`，代理数组拆除）只是倒序对每项调
  `factory->vtable[+1](proxy)`（`DeleteProxy` 槽），把释放交给代理自己；代理指针数组本身
  才由引擎用 `g_pMemAlloc` 释放。因此分配/释放两端都在本插件手里，不存在堆不匹配。
- **`po_release` 只打标记，绝不就地释放**：它由引擎在拆除循环内调用，那一刻引擎仍持有该对象
  （循环还要继续），就地 `Box::from_raw` 会让引擎继续触碰已释放内存。真正的释放由
  `purge_released()` 完成 —— 它先从 `LIVE` 摘除（**摘除这一步同时充当所有权判定**，
  所以引擎重复调用 `Release` / 标量删除析构也不会二次释放），再在**锁外** `Box::from_raw`。
- `LIVE` 表保存全部已创建对象指针；`PENDING_RELEASE` 是「已 `Release`、待回收」的计数，
  用于让 `po_on_bind`（每帧数百次调用）在无待回收项时**完全不碰 `LIVE` 锁**。

### 每帧执行：`OnBind` 驱动（路线 A，无 D3D hook）

`per_frame()` 为真的代理**直接在槽 1 `po_on_bind` 里执行** —— 引擎每次绑定该材质都会遍历
代理数组并调用 `proxy->vtable[+4](entity)`，这就是每帧驱动，无需任何 hook：

```rust
if this.is_null() { return; }
// 顺序要紧：purge_released() 会释放已 Release 的对象，若 this 在其中，
// 先 purge 再读 this 就是 use-after-free —— 所以先自判。
if (*this).released.load(Ordering::Acquire) { return; }
purge_released();
run_timers();
let Some(proxy) = (*this).proxy.as_mut() else { return; };
if !proxy.per_frame() { return; }
proxy.bind((*this).material);
```

- **不做按帧去重**：实测该槽对同一材质约 **4.68 次/帧**（8100 帧内 37916 次），一帧内会重复
  执行。全部 15 个代理都幂等（纯函数读-算-写，或基于边沿检测），因此重复执行等价 ——
  这正是不需要帧计数器、不需要 hook 的前提。
- **`run_timers()` 也挂在这里**：计时器精度取决于材质绑定频率；材质**未被绑定期间计时器不会
  到期**，会在下次绑定时立即补触发（`l4nrp_delay_set` 的语义因此是「绑定驱动的延迟」）。
- **两点快速路径**（`po_on_bind` 是热路径）：`PENDING_RELEASE == 0` 时 `purge_released` 直接
  返回；`TIMER_COUNT == 0` 时 `run_timers` 直接返回。绝大多数材质两者都是 0，热路径上
  **零锁开销**。

> **为什么最终能用 `OnBind`**：早期实测「`po_on_bind` 命中 0 次」是**测试材质从未被渲染**导致
> 的假阴性（当时用 `l4n/neko_tonemap` 做载体，而该材质从不进入引擎的代理分发列表）。
> 改用真实会被绑定的材质（`models/weapons/melee/crowbar`）后，计数稳定逐帧递增
> （实测 28995 帧 / 4828 帧两次运行，见「端到端实机验证记录」）。

> **旧结论已作废**：曾用 D3D9 `EndScene` hook（vtable 索引 42）驱动每帧，理由是「引擎从不调用
> 我方 `OnBind`，且材质 bind 频率低于帧率会漏帧」。前半句是上述假阴性；后半句在路线 A 下
> **不再是问题** —— 代理本就只对「正在被绑定的材质」有意义，未被绑定的材质不需要每帧更新。
> EndScene 路线还带来 DXVK / 设备重建耦合（`is_dxvk=true` 时尤其脆），已整体移除。

> **路线 B（未采用）**：改挂 `CMaterialSystem::BeginFrame`（vtable 槽 32 / 字节 `0x80`）或
> `EndFrame`（槽 37 / 字节 `0x94`）。比 EndScene 干净（脱离 D3D/DXVK，且 `BeginFrame`/`EndFrame`
> 是每帧恰好一次的语义），但仍是 vtable patch，且对未被绑定的材质做无谓计算。SDK 头已核对：
> `IMaterialSystem` 没有任何每帧回调或代理分发接口（`FrameSync` 系列在 `materialsystem.dll` 里
> 也不存在），所以「完全零 hook 的每帧全局回调」不存在。

### 与内置代理共存

`l4nrp_*` 现在是引擎眼里的**一等代理**，与原版 / L4N 内置代理（`Sine`/`Multiply`/
`AnimatedTexture`/`IT`/`BloodyHands` 等）走完全相同的路径，可自由混写在同一个 `"Proxies"` 块里。

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

> **`SetFloatValue` / `SetIntValue` 会同时同步两个字段**（字节级实测，`materialsystem.dll`）：
> `SetFloatValue`(+0x0c) 在 `MOVSS [this+0x0c], XMM0` 之后执行 `CVTTSS2SI` 并把整数写进
> `[this+0x08]`；`SetIntValue`(+0x10) 在 `MOV [this+0x08], EDI` 之后执行 `CVTSI2SS` 并把浮点写进
> `[this+0x0c]`（`GetIntValue`(+0x68) 读 `[this+0x08]`、`GetFloatValue`(+0x6c) 读 `[this+0x0c]`，
> 二者都不做类型转换）。
> **实践含义**：VMT 里写 `"0"`（整型）还是 `"0.0"`（浮点）**都不影响 `get_float` 读取**，
> 变量类型只在「按字符串读」时才有区别。`l4nrp_random` 的 `read_number` 因此以
> `get_float` 为主、`get_int` 仅作异常兜底。

## 每帧执行

每帧驱动已改为 **`OnBind` 驱动（路线 A，零 hook）**，详见上文「每帧执行：`OnBind` 驱动」小节。

> **v0.7.0 起不再有 D3D9 `EndScene` hook**，也不再有 `ACTIVE` 活动表与 `run_active_proxies`。
> 每帧入口是 `ProxyObject` 的槽 1 `po_on_bind`（由引擎在每次绑定该材质时调用），
> `per_frame()` 的判断也随之从「登记进活动表」改为**在 `po_on_bind` 内按需判断**。
> 历史记录（EndScene hook 的 D3D9 `__stdcall` 约定等）保留在本文「历史方案」节。

## 线程 / 锁 / 内存安全注意事项

- **`LIVE`（存活对象登记表）与 `PENDING_RELEASE`（待回收计数）**：
  `l4nrp_create` 在创建时登记进 `LIVE`；`po_release` 只置 `released` 标志并递增
  `PENDING_RELEASE`；`purge_released()`（由 `po_on_bind` 在热路径上调用）负责摘除与释放。
- **`po_on_bind` 内语句顺序是安全关键**：必须**先**判 `(*this).released` 再调
  `purge_released()` —— 后者可能把 `this` 释放掉，反过来就是 use-after-free。
- **绝不持有锁调用代理 `bind`**：`purge_released` 先在锁内摘出待回收列表、**出锁后**才
  `Box::from_raw`；`run_timers` 同样先在锁内提取到期计时器、出锁后再执行。
  若持锁期间 `bind` 经引擎回调再次加锁，会造成 `std::sync::Mutex` 重入死锁（表现为"无响应"）。
- **`bind` 返回 `Err` = 材质失效/所需变量缺失** → 记一条日志后跳过本次；`OnBind` 路径下
  不再有活动表可摘除（代理的生命周期完全由引擎的 `Release` 决定）。
- **读指针前先 `is_readable` 检查**（`VirtualQuery`）：防止 `strlen`/`CStr` 读到坏指针崩溃。
  `is_readable` 复用 32 位 `MEMORY_BASIC_INFORMATION` 布局（`Mbi32`，x86）。
- **堆归属**：`ProxyObject` 由 Rust 全局分配器（`Box::into_raw`）分配，也由 Rust
  （`purge_released` 的 `Box::from_raw`）释放 —— **引擎从不释放我方对象**（见上文），
  所以不存在「Rust 分配 / MSVC 释放」的堆不匹配。
- **两条零锁快速路径**：`PENDING_RELEASE == 0` 时 `purge_released` 直接返回；
  `TIMER_COUNT == 0` 时 `run_timers` 直接返回。这两条计数只作快路径判据，
  真值始终以 `LIVE` / `TIMERS` 为准。
- **`Mutex` 一律用 `unwrap_or_else(|e| e.into_inner())`**：`panic = "abort"` 下 `.unwrap()` 的
  poison panic 会直接让游戏崩溃。**v0.7.0 已把 `REGISTRY` / `LIVE` / `TIMERS` 全部改为
  `unwrap_or_else`**（历史遗留已清理）。

## 裸指针创建约定（Strict Provenance）

创建裸指针**优先用 `&raw const` / `&raw mut`**（[Rust 文档「Common ways to create raw pointers」](https://doc.rust-lang.org/std/primitive.pointer.html#common-ways-to-create-raw-pointers)
第 1/3 条），**不要**写 `&x as *const T` / `&mut x as *mut T`。`lib.rs` 顶部已开启
`#![warn(clippy::ref_as_ptr)]` 把这条规则钉死（该 lint 属 pedantic、默认关闭）。

按来源分四类处理：

| 指针来源 | 写法 |
|---|---|
| 本地变量 / 结构体字段 | `&raw const x`、`&raw mut x`（可对未对齐字段、packed struct 取址） |
| `Vec`/`Box` 等容器元素 | `&raw mut e.proxy`；容器自身用 `as_ptr()` / `as_mut_ptr()` |
| 来自 C / Win32 API | 直接用返回值（如 `HMODULE`、`VirtualAlloc` 的 `*mut c_void`），**全程保持指针类型** |
| 外部裸地址（模块基址+RVA、函数指针转整数） | 见下 |

**Strict Provenance 的关键是"指针全程不降级为整数"**，而非换个函数：

- 需要数值地址时用 `.addr()`（丢弃 provenance，但语义明确），不要写 `p as usize`；
- `materialsystem.dll` 各引擎函数地址由 `ms_fn(rva)` 返回 **`*const u8`**（`HMODULE` 基址
  `.add(rva)` 指针算术），不再经 `usize` 中转 —— `HMODULE` 本身就是模块映射基址（Rust 文档
  第 4 条「Get it from C」），自带 provenance；
- `engine::material_proxy_factory()` 取到的工厂对象是引擎给的 `*mut c_void`（来自
  `__cdecl` getter 的返回值），**全程保持指针类型**；`add_material_proxy` 从工厂首字段读出
  vtable、再读槽位 2 得到函数地址，该地址以 `usize` 取出后 `transmute` 成函数指针
  （函数指针槽位没有 `&raw` 来源，这是必需的例外）。

> **`with_exposed_provenance` 目前已无使用**：v6 起不再 detour 引擎函数，没有"从 `E9 rel32`
> 解码出外部地址"的场景了。它与 `.addr()` 同属 strict provenance 家族（1.84 稳定），用途是
> "地址来自外部、无法回溯来源"；若将来重新引入 detour 才需要它。

> **不要用 `transmute` 把空指针变成函数指针再判空**：函数指针不可为 null，
> `(fn_ptr as *const ()).is_null()` 恒为 `false`（clippy 会报
> `fn_to_numeric_cast`/`fn_null_check`）。必须先对**源指针**判空再 `transmute`
> （见 `parse_proxy_params` / `engine::add_material_proxy`）。

验证手段（lint 已改名为 `implicit_provenance_casts`，仍是 unstable）：

```powershell
$env:RUSTC_BOOTSTRAP="1"
$env:RUSTFLAGS='-Zcrate-attr=feature(strict_provenance_lints) -W implicit_provenance_casts'
cargo build --release   # 当前 0 命中
```

## 代理编写约定

- 每个代理独立 struct（参数隔离），实现 [`Proxy`](src/kv.rs) trait：
  - `apply_kv(name, value)`：`"Proxies"` 块参数填充（参数名不区分大小写）
  - `bind(material)`：读写材质 VMT 变量；返回 `Err` 表示材质失效/变量缺失，本次跳过并记日志
  - `per_frame()`：是否每帧执行（依赖每帧变化的输入，如 `Sine` 输出，应覆写为 `true`）

trait 定义（[`src/kv.rs`](src/kv.rs)）：

```rust
pub trait Proxy: Send {
    fn apply_kv(&mut self, name: &str, value: &str);          // VMT "Proxies" 块参数填充
    unsafe fn bind(&mut self, material: *mut c_void) -> Result<(), PluginError>; // 触发动作；返回 Err = 材质失效/变量缺失
    fn per_frame(&self) -> bool { false }                     // 是否每帧执行（持续计算）
}
```

> `bind` 返回 `Err` 时，`po_on_bind` 只记一条日志并跳过本次 —— 代理实例的生命周期完全由引擎
> 的 `Release` 决定，不再有"从活动表移除"这一步（没有活动表了）。

注册（泛型，见 [`src/lib.rs`](src/lib.rs) `try_bind_and_install`）：

```rust
material::register_proxy::<DoesEqualProxy>("l4nrp_does_equal");
material::register_proxy::<CompareProxy>("l4nrp_compare");
material::register_proxy::<IsInRangeProxy>("l4nrp_is_in_range");
material::register_proxy::<PrintVariable>("l4nrp_print_variable");
material::register_proxy::<StrConcatProxy>("l4nrp_str_concat");
material::register_proxy::<StrReplaceProxy>("l4nrp_str_replace");
material::register_proxy::<StrSliceProxy>("l4nrp_str_slice");
material::register_proxy::<StrLenProxy>("l4nrp_str_len");
material::register_proxy::<VmtName>("l4nrp_vmt_name");
material::register_proxy::<MathProxy>("l4nrp_math");
material::register_proxy::<LogicProxy>("l4nrp_logic");
material::register_proxy::<RandomProxy>("l4nrp_random");
material::register_proxy::<Vec3Proxy>("l4nrp_vec3");
material::register_proxy::<DelaySetProxy>("l4nrp_delay_set");
material::register_proxy::<DelayAbortProxy>("l4nrp_delay_abort");
```

> **整数结果用 `SetInt`**：比较类代理（`does_equal` / `compare` / `is_in_range`）输出 0/1（`compare`
> 为 -1/0/1）的结果变量用 `material::set_int` 写入（输入比较仍用 `get_float`）；`PrintVariable`
> 用 `get_int` / `get_string` 读取 int / string 类型变量。
>
> **字符串读写用 `SetString`/`GetString`**：`str_concat`（`src_a`+`src_b`→`result`）与
> `str_replace`（`src` 里把 `search` 全替换为 `replace` →`result`）用 `material::get_string` 读、
> `material::set_string` 写（均 `per_frame`，每次先复制指针再释放锁，见上）。`str_replace` 的
> `search`/`replace` 若以 `$` 开头当作变量名读取，否则当作字面字符串。
>
> **`l4nrp_str_slice` 字符串切片**：`src`（`get_string` 读）**按字符**（非字节，避免 UTF-8 边界
> panic）截取切片：`src_start`（整型，`get_int`，起始字符位置，越界 clamp 到 `[0, 长度]`）起，
> `src_len`（整型，`get_int`，截取字符数，`<=0` 得空串，超出剩余则到末尾）→`result`
> （`set_string`，每帧）。
>
> **`l4nrp_str_len` 字符串长度**：`src`（`get_string` 读）**按字符**（非字节）计数后写入
> `result`（整型，`set_int`，每帧）。
>
> **`l4nrp_vmt_name` 材质名**：材质加载时把 `material::get_name(this)`（VMT 去掉 `.vmt` 后的路径，
> 如 `"Example"`）写入 `result`（`set_string`，一次性，非 `per_frame`）。
>
> **`l4nrp_math` 数学表达式**：表达式由 [`src/expr.rs`](src/expr.rs) `eval_math` 求值
> （无第三方依赖），**仅支持数学**：`+ - * / % ^`、括号、一元负号与常用函数
> （`sin/cos/sqrt/abs/min/max/clamp/pow/lerp/...`），**不支持**比较/逻辑运算符；表达式里的
> `$name` 或 `name` 经 `get_float` 读取该材质已声明变量（未定义按 0.0），结果写 `result`（每帧）。
>
> **`l4nrp_logic` 逻辑表达式**：由 [`src/expr.rs`](src/expr.rs) `eval_logic` 求值，在数学运算之上
> 增加比较（`==` `!=` `<` `<=` `>` `>=`）与逻辑（`&&` `||` `!`，非 0 视为真），另有范围函数
> `in_range(v,min,max)`（含端点）/ `in_range_exclusively(v,min,max)`（不含端点）；表达式里的
> `$name` 或 `name` 经 `get_float` 读取变量（未定义按 0.0），**结果写 `result`（整型 0/1，用
> `set_int`）**，每帧重算。
>
> **用表达式替换 `LessOrEqual` 串（帧数探测）**：引擎的 `LessOrEqual` 是**二选一（select）**
> 而非布尔 —— `srcVar1 <= srcVar2 ? LessEqualVar : greaterVar`，输出的是两个值之一。只有两个分支
> 都能化简成 0/1 时才等价于一条 `l4nrp_logic` 表达式。指南 RNG 的帧数探测恰好满足，三行改写：
> `l4nrp_math "max(maxFrame, checkMax)" → $maxFrame`；
> `l4nrp_logic "(countingFinished || checkMax <= 0) && maxFrame > 0" → $countingFinished`（闩锁）；
> `l4nrp_math "maxFrame + 1 + invisibleIsAlsoASkinHere" → $frameLimit`。
> **组合条件必须加括号**：`&&` 优先级低于 `<=`/`>`，漏括号会静默改变语义。
> 互读依据（[`materialsystem.dll`](src/material.rs) 反编译）：`SetIntValue`(+0x10) 会把
> `(float)v` 同时填进浮点槽 `+0xc`，而 `GetFloatValue`(+0x6c) 读的正是 `+0xc` —— 所以
> `l4nrp_logic` 写出的 0/1 能被 `l4nrp_math` / `l4nrp_random` 的 `gate` 精确读回（反向 float→int 截断）。
> 等价性由 [`src/expr.rs`](src/expr.rs) 的 `rng_frame_probe_matches_engine_chain` 单测逐帧兜底。
>
> **`l4nrp_delay_set` 延迟置位**：检测 `trigger`（整型，`get_int`）非 0 的**上升沿**后，向全局
> 计时器注册表（[`material.rs`](src/material.rs) `start_timer`）申请一个 **UUID v4 字符串手柄**
> （`uuid` crate `Uuid::new_v4()`）并计时 `delay` 毫秒；`value` 变量（整型，`get_int`）在**触发（上升沿）
> 那一刻读取并快照**，到期由 `run_timers` 每帧用 `set_int` 把该**快照值**写入 `output`（延迟期间源变量
> 再变不影响本次输出）。可选 `handle` 变量写出当前手柄（字符串类型，无计时器写空字符串），供
> `l4nrp_delay_abort` 等代理中断。上升沿触发（需先回 0 再置位才重复触发）。
>
> 生命周期：计时器为全局注册表条目，与代理实例解耦。`DelaySetProxy` 实现了 `Drop` —— 代理实例被销毁时
> （引擎调 `Release` → `purge_released` 回收；或插件卸载）调用 `abort_current` 取消挂起的计时器，
> 防止其在全局 `TIMERS` 表残留、到期访问已失效材质。
> `bind` 返回 `Err` **不再**销毁代理（v0.7.0 起没有活动表），只跳过本次执行及其后的计时器驱动的输出。
>
> **`l4nrp_delay_abort` 中断计时器**：`trigger`（整型，`get_int`）非 0 时，读取 `handle` 变量
> （字符串类型，`get_string`）指定的 UUID 手柄并调用 `material::abort_timer` 中断对应计时器
> （幂等，无该计时器则无操作）。
>
> **`l4nrp_random` 随机数**：把 `[0,1)` 随机比例缩放到 `[min,max]` 写入 `result`（每帧）。
> `min`/`max`/`shared` **既可写字面量也可写变量名**（其它代理的 `min`/`max` 一律按变量名）；
> 其余参数：`gate`（0 时不动作）、`trigger`（整型，值变化时重掷）、`seed`（额外熵）、
> `unit`（外部 `[0,1)` 随机源，如 `EntityRandom`，给了就不走内部 PRNG）、
> `integer`（结果向下取整）、`write_int`（用 `set_int` 写出）。
> PRNG 用 **`rand` crate 的 `rand::rngs::StdRng`**（ChaCha12，`seed_from_u64` 播种），
> 种子由 `reseed()` 混合系统时间 + 材质实例地址（仅当熵用，不解引用）+ `seed` 变量 + 重掷计数。
> **`shared` 语义（对应指南的 `$oneSkinPerMap`）**：`reseed` 在 shared 模式下**只用进程级
> `shared_salt()`**（`OnceLock` 取一次系统时间），不掺时间/地址/计数/额外熵 —— 否则先重掷与
> 后重掷的实例会分叉。因此同一进程内所有实例、任何时刻重掷都得到同一皮肤。
> 与引擎 `EntityRandom` 的差异：**静态物体也能用**（不依赖实体）、帧间稳定（不每帧乱跳）、
> `min`/`max` 每帧重读（上界变大时结果按比例跟随，省掉原版 RNG 那一长串溢出/钳制代理）。

- **CString 缓存**：`per_frame` 代理在 struct 里缓存变量名 `CString`（`cstr_of` 构造），避免每帧
  反复堆分配；`apply_kv` 更新变量名时用辅助函数 `set_kv(&mut dst, &mut c, value)` 同步重建缓存
  （位于 [`src/lib.rs`](src/lib.rs)，可复用）。
- 代理见 [`src/lib.rs`](src/lib.rs)：`DoesEqualProxy` / `CompareProxy` / `IsInRangeProxy` /
  `PrintVariable` / `StrConcatProxy` / `StrReplaceProxy` / `StrSliceProxy` / `StrLenProxy` / `VmtName` /
  `MathProxy` / `LogicProxy` / `RandomProxy` / `Vec3Proxy` / `DelaySetProxy` / `DelayAbortProxy`。
- 完整 VMT 用法示例见项目根目录 [`Example.vmt`](Example.vmt)（涵盖全部 15 个代理及其参数）；
  随机皮肤的三份改写示例见 [`Example_RNG.vmt`](Example_RNG.vmt) / [`Example_RNG_Map.vmt`](Example_RNG_Map.vmt) /
  [`Example_RNG_Entity.vmt`](Example_RNG_Entity.vmt)。

## 构建 / 部署 / 验证

```powershell
cargo build --release   # 目标 i686-pc-windows-msvc
cargo test --lib        # 表达式求值器单元测试（[`src/expr.rs`](src/expr.rs)，宿主目标可跑）
```

- 部署：复制 DLL 到游戏安装目录 `bin/neko/plugins`（必须与 `left4neko.dll` 同目录的 `neko/plugins`，
  **不是**工作区里的 `bin/neko/plugins`）。left4neko 用 `std::filesystem::directory_iterator` 遍历
  该目录所有 DLL 并调用 `GetL4NPluginInstance`。
- 日志：`l4n_material_proxy_plugin.log`（当前工作目录，**每次游戏运行（插件加载）首次写日志前
  自动清空**）+ 引擎控制台 `[l4n-proxy]` 前缀输出（tier0.dll `Msg`，fallback `OutputDebugStringA`）。
- 安装时序：`OnModuleLoaded("client")` 或 D3D 首帧（`on_d3d_device_created`）时调用
  `try_bind_and_install`（幂等，成功安装后不再重复）；必须先等 `materialsystem.dll` 加载。

## 已知问题

- v6.0：**代理注册无法撤销**。`CMaterialProxyDict` 只有 CreateProxy / DeleteProxy / AddProxy
  三个槽位，**没有"按名字删除"的接口**，插件析构时**没有任何东西可以还原**（v0.7.0 起连 D3D
  `EndScene` vtable 槽也不再改写，`dtor` 只记一条日志），引擎侧的代理名字条目会留到进程结束。
  名字字符串也是 `Box::leak` 故意泄漏的（`AddProxy` 是否复制名字未验证），泄漏量 = 15 个短字符串，
  可忽略。
- v6.1：HUD 菜单 `"cvar"` 条目的**夹取方向未完全确定**。`FUN_10049350` 的越界分支在反编译里呈现为
  `if ((*(float*)(param_1+0x4c) + 1e-06 < local_30) || (local_30 < *(float*)(param_1+0x48) - 1e-06)) local_30 = *(float*)(param_1+0x4c);`
  —— `+0x48` / `+0x4c` 哪个是 `min`、哪个是 `max`，以及越界时夹到哪一端，未能完全确定。
  因此 `"菜单横向偏移"` 条目的实际效果是「每次点击把 `l4n_hudmenu_offset_x` 朝某个方向调整」，
  极端情况下可能直接落到 `±500` 边界（可见但无害、可恢复）。
  要精确验证需在实机里点击该菜单项并观察 convar 的实际变化方向。
- v6.2：**`material+0x23` / `material+0x28` 的读取时机**。这两个字段（代理计数 / 代理数组）
  由引擎在 `ParseProxies` **尾段**才写入；在 `FindMaterial` 刚返回时读到的仍是 0 / 空。
  排查问题时不要把它当成"注册失败"的证据。
- v6.3：**已修复（v0.7.0）**。曾有一批锁用 `.unwrap()`（`panic = "abort"` 下 poison panic 会直接
  崩溃游戏），现已把 `REGISTRY` / `LIVE` / `TIMERS` 全部改为 `unwrap_or_else(|e| e.into_inner())`。
  新代码必须沿用该写法。

## 历史方案（v1–v5，已废弃）

以下机制**已全部被 v6 取代**，保留记录以免重走弯路：

- **v1–v3**：链式 hook proxy factory 创建对象 → 崩溃。
- **v4**：hook 引擎解析函数后透传原函数 → 原函数调用 left4neko `CreateProxy` 仍崩溃，且自解析
  KeyValues 布局遇到特殊材质（如 `Shadow`）会遍历越界。
- **v5**：**detour `materialsystem.dll` RVA `0x2d50`（`FUN_10002d50`）**，在引擎解析 `"Proxies"`
  块时拦截我们的代理名、用引擎自己的 KeyValues 函数（`0x76020` FindKey / `0x75dc0` FirstChild /
  `0x75dd0` NextSibling / `0x75b90` GetKeyName）解析参数，然后**把节点从链表摘除**再透传原函数。
  可用但不彻底：`l4n_is_proxy_exist` 查不到（代理不在引擎表里），且 `uninstall` 只能还原入口字节。

> **旧结论已作废**：曾认为"left4neko 深 hook 材质系统，自定义 `IMaterialProxy` 会被按
> `CResultProxy` 布局处理而崩溃"（曾引 `left4neko+0xE2AB8`）。该结论**无法复现**：
> `left4neko+0xE2AB8` 不是函数入口（`create_function` → `body_size=1`），真实入口
> `FUN_100e2a68` 反编译后**无任何 `CResultProxy` 布局证据**；`list_globals(ResultProxy)` 为 0 条。
> 实机探针（v5/v6）已证明自定义 `IMaterialProxy` 能安全注册并被引擎正常驱动。
>
> 另外，"left4neko 改写了 `VClient016->vtbl[0x128]`" 是**真的**（运行时该槽 =
> `left4neko+0xe4ab0`），但它只影响 `l4n_is_proxy_exist` 的查询路径：
> `FUN_100e4ab0` 先查 left4neko 自己的 `IMaterialProxyDict`（单例 getter `FUN_100c2df0`），
> **未命中再回退到引擎原生的 `client+0xa9570`**（= `GetProxyFactory()->CreateProxy(name)`）。
> 所以只要注册进了 `CMaterialProxyDict` 且 `createFn` 返回非空，`l4n_is_proxy_exist` 就能查到。

## 端到端实机验证记录

### v6：注册进引擎代理工厂

用临时探针插件 `l4n_probe.dll`（只观察、不注册）+ 测试材质 `l4n/l4nrp_e2e_test.vmt` 在真实游戏中验证：

| 验证项 | 结果 |
|---|---|
| 注册 | 工厂 `m_nProxies` 67 → **82**（+15）；`CreateProxy("l4nrp_math")` 等 6 个名字全部返回非空 |
| 不存在的名字 | `CreateProxy("__l4nrp_no_such_proxy__")` → `0x0`（对照正确） |
| `l4n_is_proxy_exist` | 复刻其路径，6 个 `l4nrp_*` 全部返回非空（= 会打印 `exist!`） |
| 引擎实例化 | 插件日志 `engine proxy: 'l4nrp_math' Init material=0x... per_frame=true` × 24 |
| 生命周期 | `engine proxy: Release` 30+ 次；`purge_released` 正常回收（存活数 18→17→16…） |
| 每帧驱动（旧） | 当时由 D3D9 `EndScene` hook 驱动：测试 VMT 的 `$e2eWatch` 与帧计数严格同步递增 |
| 稳定性 | 多次运行无崩溃 |

### v0.7.0：`OnBind` 驱动（零 hook）

**不再使用探针** —— 直接用发布版插件 + addon VPK 覆盖一个真实会被绑定的材质
（`models/weapons/melee/crowbar`；原材质在 `pak01` 内，addon VPK 优先级最高故能覆盖）：

| 验证项 | 结果 |
|---|---|
| 注册 | 15 个代理全部注册，日志 `registered 15 proxies into CMaterialProxyDict` |
| 引擎实例化 | `Init material=` 8 行，两个材质：`l4n/neko_tonemap`（`$l4nrpWt`）与 `crowbar`（`$l4nrpCb`） |
| **每帧驱动** | `models/weapons/melee/crowbar` 上的 `$l4nrpCb` 由 `l4nrp_math { "expr" "l4nrpCb + 1" }` 自增，**完全由 `po_on_bind` 驱动、零 D3D hook**：两次运行分别递增到 **28995** 与 **4828** |
| 无 D3D hook | 日志 `OnD3DDeviceCreated device=... is_dxvk=true (no D3D hook: OnBind-driven)`，运行中不触碰任何 D3D vtable 槽 |
| 错误计数 | `bind failed` = 0、`not in registry` = 0、`Release` = 0、`panic`/`error`/`assert`/`overflow` = 0 |
| 稳定性 | 无崩溃；两次运行均在数十秒后因 Steam 认证失败退出（`No Steam logon`，非崩溃），无新 `.mdmp`、无 WER 事件 |
| 覆盖生效 | `console.log` 出现 `Error: Material "concrete/concrete_ext_15" uses unknown shader "L4nrpVpkMarkerShader"` → 证实 addon VPK 成功覆盖 `pak01` 材质 |

> 验证手法（v0.7.0）：
> ```powershell
> $root='E:\SteamLibrary\steamapps\common\Left 4 Dead 2'
> Start-Process -FilePath "$root\left4dead2.exe" -ArgumentList @('-steam','-language','schinese',
>   '-heapsize','2097151','-novid','-nojoy','-noforcemaccel','-noforcemspd','-noforcemparms',
>   '+exec','autoexec.cfg','-condebug','+map','c1m1_hotel') -WorkingDirectory $root -PassThru
> ```
> **不要加 `-insecure`**（会卡在 `Engine Error` 模态框）；`-WorkingDirectory $root` 使插件日志
> 落在游戏根目录 `l4n_material_proxy_plugin.log`。等 50–60 s 后检查 `MainWindowTitle` 是否为
> `Left 4 Dead 2 - Direct3D 9`（崩溃时是「哇袄！游戏爆炸了!」）。
> **诊断读数必须与"测试材质确实被加载"同时成立才有意义** —— 曾因测试材质未被加载
> （`l4n/neko_tonemap` 从不进入代理分发列表）而看到 `po_on_bind` 命中 0 次，误判为"每帧驱动没跑"。
> 让测试材质生效的正确做法是 **addon VPK**（`left4dead2/addons/*.vpk` 优先级高于 pak01 与 dlc），
> 而不是 loose `left4dead2/materials/`（会输给 pak01/dlc VPK）。
