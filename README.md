# rust_l4n_material_proxy_plugin

L4N 插件：**在 Rust 中注册自定义材质代理（material proxy）**。在 VMT 的 `"Proxies"` 块里写代理名，即可触发插件回调，回调内可读写该材质的 VMT 变量（变色、比较运算、打印变量等）。

代理是**直接注册进引擎的材质代理工厂**（`client.dll` 的 `CMaterialProxyDict`）的，与 `Sine`/`Multiply`/`AnimatedTexture` 等内置代理同级：引擎自己解析 VMT 并实例化它们，`l4n_is_proxy_exist` 也能查到。

本插件实现了 **L4N 插件接口 v2**（`IL4NPlugin`），除材质代理外还提供一个 **HUD 菜单入口**（见[游戏内 HUD 菜单](#游戏内-hud-菜单)）。

对于添加的材质代理详情，见 [`Example.vmt`](Example.vmt)。

## 示例文件

| 文件 | 说明 |
|---|---|
| [`Example.vmt`](Example.vmt) | 全部内置代理的参数用法演示 |
| [`Example_RNG.vmt`](Example_RNG.vmt) | **随机皮肤**：改写自 Steam 指南 [RNG in 10 seconds (for modders)](https://steamcommunity.com/sharedfiles/filedetails/?id=2389582650) 的 ⚪️ 版，指南里约 40 个代理 → `l4nrp_random` 1 个 + 帧数探测 3 个 |
| [`Example_RNG_Map.vmt`](Example_RNG_Map.vmt) | 同上 🔶 版（地图静态物件 / 按玩家位置换皮肤） |
| [`Example_RNG_Entity.vmt`](Example_RNG_Entity.vmt) | 同上 🔵 版（幸存者等实体，尸体与生前皮肤一致） |

### 随机皮肤：指南原版 vs 本插件

Steam 指南 [RNG in 10 seconds](https://steamcommunity.com/sharedfiles/filedetails/?id=2389582650) 用 `AnimatedTexture` 数出 VTF 的帧数，再用 `CurrentTime` / `EntityRandom` / `PlayerProximity` / `ConVar` 等十几个引擎代理做溢出钳制与重掷，中间变量三十多个。本插件提供 `l4nrp_random`，把「随机」那部分整体替换为一个代理：

| 环节 | 指南原版 | 本插件 |
|---|---|---|
| 帧数探测 | `AnimatedTexture` + 3 个 `LessOrEqual` + `Add` | `AnimatedTexture`（**必须保留**，只有它能数出 VTF 有几帧）+ `l4nrp_math` ×2 + `l4nrp_logic` ×1 |
| 随机 | `EntityRandom` + `CurrentTime` + `PlayerProximity` | `l4nrp_random` |
| 溢出钳制 | `Subtract`/`LessOrEqual` 绕一圈夹进 `[0,帧数)` | `min`/`max` 直接缩放 |
| 重掷信号 | `ConVar` 读 `cl_buy_favorite_nowarn` | `trigger`（可选） |
| 每地图固定一个皮肤 | `$prepareStore`/`$randSYSStored`/`$randSYSInject` 三变量比对 | `shared`（可选） |
| 静态物件（地图道具） | 必须换用 🔶 版，否则加载即崩溃 | 无需换文件（不依赖实体） |

#### 帧数探测为什么能压成表达式

指南那串 `LessOrEqual` 看着绕，是因为它其实是**二选一（select）而不是布尔**：

```text
LessOrEqual:  srcVar1 <= srcVar2 ? LessEqualVar : greaterVar
```

只有当两个分支都能化简成 0/1 时，它才等价于一条逻辑表达式。帧数探测恰好满足，三件事各写成一行：

| 原版 | 改写 |
|---|---|
| 3 个 `LessOrEqual`（含 `$zero`/`$one` 常量变量） | `l4nrp_logic`：`(countingFinished \|\| checkMax <= 0) && maxFrame > 0` |
| `Add` 求运行最大值 | `l4nrp_math`：`max(maxFrame, checkMax)` |
| `Add` 求帧数上界 | `l4nrp_math`：`maxFrame + 1 + invisibleIsAlsoASkinHere` |

- `maxFrame` 是运行最大值，`countingFinished` 是**闩锁**：`maxFrame > 0` 才可能置位，置位后靠 `||` 自己保持。
- 组合条件**必须加括号**：`&&` 的优先级低于 `<=` / `>`，`a || b <= 0 && c > 0` 会被解析成 `a || ((b <= 0) && (c > 0))`。
- `l4nrp_logic` 用 `set_int` 写 0/1，而引擎的 `SetIntValue` 会同时把 `(float)v` 填进浮点槽（`GetFloatValue` 读的就是那里），所以它的输出能被 `l4nrp_math`、`l4nrp_random` 的 `gate` 精确读回。
- 等价性有单元测试兜底：`src/expr.rs` 的 `rng_frame_probe_matches_engine_chain` 逐帧跑完整动画循环，断言改写版与指南原版链输出完全一致。

## 安装

1. 构建插件（目标 `i686-pc-windows-msvc`）：

   ```powershell
   cargo build --release
   ```

2. 将生成的 DLL 复制到游戏安装目录的 `bin/neko/plugins`：

   ```powershell
   Copy-Item target/i686-pc-windows-msvc/release/rust_l4n_material_proxy_plugin.dll `
     "E:\SteamLibrary\steamapps\common\Left 4 Dead 2\bin\neko\plugins\"
   ```

   > 必须是**与 `left4neko.dll` 同目录的 `neko/plugins`**（即游戏安装目录下的`bin/neko/plugins`）。

3. 启动游戏，left4neko 会自动遍历并加载该目录下的所有插件 DLL。

> 本插件要求 **L4N ≥ 2.34.0**（该版本引入 `l4nplugin` 插件系统）与 `l4n_plugin.h` 的 **接口版本 2**。
> 插件通过 `GetInterfaceVersion()` 返回 `2` 来声明自己支持 `RequestHudMenu`；
> 若你的 L4N 版本较旧（接口版本 1），left4neko 会跳过 HUD 菜单部分，材质代理功能不受影响。

## VMT 用法

在材质的 VMT 文件 `"Proxies"` 块里写上代理名，以及可选参数：

```text
// rainbow.vmt
"UnlitGeneric" {
    "$basetexture" "vgui/common/l4d_spinner"
    "$color" "[1 1 1]"

    "$speed" "0.5"
    "$offset" "0"

    "$time" "0"
    "$theta" "0"
    "$r" "0"
    "$g" "0"
    "$b" "0"
    "Proxies" {
        "CurrentTime" {
            "resultVar" "$time"
        }
        "l4nrp_math" {
            "expr" "$time * $speed"
            "result_var" "$theta"
        }
        "l4nrp_math" {
            "expr" "0.5 + 0.5 * sin($theta)"
            "result_var" "$r"
        }
        "l4nrp_math" {
            "expr" "0.5 + 0.5 * sin($theta + 2 * pi() / 3)"
            "result_var" "$g"
        }
        "l4nrp_math" {
            "expr" "0.5 + 0.5 * sin($theta + 4 * pi() / 3)"
            "result_var" "$b"
        }
        "l4nrp_vec3" {
            "src_x" "$r"
            "src_y" "$g"
            "src_z" "$b"
            "result_var" "$color"
        }
        "l4nrp_print_variable" {
            "var" "$color"
            "type" "vector"
        }
    }
}
```

### 内置代理及参数

| 代理名 | 参数（键 → 默认） | 行为 |
|---|---|---|
| `l4nrp_does_equal` | `src_a`→`$src_var_1`、`src_b`→`$src_var_2`、`result`→`$result_var` | 相等则写 result=1 否则 0（每帧） |
| `l4nrp_compare` | `src_a`→`$src_var_1`、`src_b`→`$src_var_2`、`result`→`$result_var` | 三态比较：a==b→0、a>b→1、a<b→-1（相对容差 1e-6，每帧） |
| `l4nrp_is_in_range` | `src`→`$src_var`、`min`→`$min_var`、`max`→`$max_var`、`result`→`$result_var` | 在 [min,max] 内写 result=1 否则 0（每帧） |
| `l4nrp_str_concat` | `src_a`→`$src_var_1`、`src_b`→`$src_var_2`、`result`→`$result_var` | 拼接两个字符串变量：src_a+src_b → result（每帧） |
| `l4nrp_str_replace` | `src`→`$src_var`、`search`→""、`replace`→""、`result`→`$result_var` | 把 src 中所有 search 替换为 replace 写入 result（search/replace 以 `$` 开头当作变量名，否则字面量；每帧） |
| `l4nrp_str_slice` | `src`→`$src_var`、`src_start`→`$src_start`、`src_len`→`$src_len`、`result`→`$result_var` | 从 src 字符串按**字符**（非字节，避免 UTF-8 边界 panic）切片：src_start 为起始字符位置（越界 clamp 到 `[0, 长度]`），src_len 为截取字符数（`<=0` 得空串，超出剩余则到末尾），结果写 result（每帧） |
| `l4nrp_str_len` | `src`→`$src_var`、`result`→`$result_var` | 计算 src 字符串的**字符**长度（非字节计数）并写入 result（整型，每帧） |
| `l4nrp_vmt_name` | `result`→`$result_var` | 把材质自身文件名（`material::get_name`，即 VMT 去掉 `.vmt` 后的路径，如 `"Example"`）写入 result（字符串，一次性，非每帧） |
| `l4nrp_vec3` | `src_x`→`$src_x`、`src_y`→`$src_y`、`src_z`→`$src_z`、`result`→`$result_var` | 把 3 个浮点变量作为分量组成三维向量写入 result（每帧） |
| `l4nrp_math` | `expr`→`"0"`、`result`→`$result_var` | 计算数学表达式（四则/幂/括号/函数；`$var` 或 `var` 读材质已定义变量），结果写入 result（每帧） |
| `l4nrp_logic` | `expr`→`"0"`、`result`→`$result_var` | 计算逻辑表达式（比较 `== != < <= > >=` 与逻辑 `&& \|\| !`，非 0 视为真；另有 `in_range`/`in_range_exclusively` 范围函数），结果写 result（整型 0/1，每帧） |
| `l4nrp_random` | `min`→`0`、`max`→`1`、`result`→`$random_result`；可选 `gate`、`trigger`、`seed`、`unit`、`shared`、`integer`、`write_int` | 稳定的伪随机数：按材质实例播种一次（**静态物体也能用**），把 `[0,1)` 比例缩放到 `[min,max]` 写入 result（每帧） |
| `l4nrp_delay_set` | `trigger`→`$trigger_var`、`delay`→`1000`、`output`→`$result_var`、`value`→`$value_var`、`handle`→""（可选） | 检测 trigger（整型）非 0 上升沿后启动计时器，延迟 delay 毫秒把 value 变量的整型值**触发时快照**写入 output（延迟期间 value 变化不影响本次输出）；handle 写出 UUID v4 计时器手柄（字符串类型，无计时器写空字符串，每帧） |
| `l4nrp_delay_abort` | `trigger`→`$trigger_var`、`handle`→`$timer_handle` | trigger 非 0 时中断 handle 变量指定的计时器（每帧） |
| `l4nrp_print_variable` | `var`→`$var`、`type`→`float`（`float`/`int`/`vector`/`string`） | 每帧读取变量并打印 |

### 注意事项

- 代理只能读写**已在 VMT 声明**的变量（引擎只为声明过的变量创建变量对象）。若想让代理写入某个变量，请先在 VMT 顶层声明它，例如 `"$result_var" "0"`。
- 插件与**原版/L4N 内置材质代理共存**：`l4nrp_*` 是引擎眼里的普通材质代理，同一个 `"Proxies"` 块里 `Sine`/`Multiply`/`AnimatedTexture`/`IT`/`BloodyHands` 等与 `l4nrp_*` 可自由混写。
- `l4nrp_random` 的 `min`/`max`/`shared` 既可以写字面量（`"0"`）也可以写变量名（`"$frameLimit"`）；其它代理的 `min`/`max` 一律按变量名处理。
- `l4nrp_math` 与 `l4nrp_logic` 的表达式里，`$var` 与 `var` 等价；未声明的变量按 `0` 处理（不报错）。

## 游戏内 HUD 菜单

插件实现了接口版本 2 新增的 `IL4NPlugin::RequestHudMenu`，因此在 L4N 主菜单（默认按 `\` 打开）里会多出一个 **「L4NRP 材质代理」** 条目。进入后可以看到：

| 菜单项 | 类型 | 行为 |
|---|---|---|
| 重新加载所有材质 | `cmd` | 执行 `mat_reloadallmaterials`，改完 VMT 后无需重启即可看到效果 |
| 检查代理是否生效 | `cmd` | 执行 `l4n_is_proxy_exist l4nrp_math`，在控制台确认代理已挂上引擎 |
| 菜单横向偏移 | `cvar` | 以步长 `10` 调整 `l4n_hudmenu_offset_x`（范围 `-500` ~ `500`），用来挪动 HUD 菜单位置 |
| 已注册代理… | `callback` | 展开子菜单，逐个列出本插件已注册的全部材质代理，点任意一项即可用 `l4n_is_proxy_exist` 单独检查它 |

菜单的实现与逆向依据见 [`src/menu.rs`](src/menu.rs) 顶部文档注释。

### 自定义菜单

`RequestHudMenu(false)` 需要返回一段 Valve KeyValues 文本，形状是**恰好一个顶层对象 = 菜单标题，其子条目 = 菜单项**：

```text
"我的标题" {
    "项目1" { "cmd" "my_command" }
    "项目2" { "cvar" "my_cvar" "delta" "1" "min" "0" "max" "10" }
    "带子菜单的项目" { "callback" "0x12345678" "user_data" "0x87654321" }
}
```

- 三种条目类型：`"cmd"`（执行控制台命令）、`"cvar"`（调整控制台变量，可选 `delta`/`min`/`max`）、`"callback"`（调用函数）。
- `"callback"` 的值是函数地址字符串（支持 `0x` 十六进制前缀），签名为 `const char* __cdecl f(void* user_data)`；返回 `nullptr` 表示不展开子菜单，否则返回子菜单的 KeyValues 文本（可递归嵌套）。
- `"user_data"` 可选，作为 `callback` 的参数原样传入。
- 文本必须是 **UTF-8** 编码（neko 自带的 `neko/config_template.vdf` 即纯 UTF-8 无 BOM）。
- 注意：`callback`/`user_data` 的地址字符串由 left4neko 用 `std::stoul(..., base = 0)` 解析，格式非法会抛 C++ 异常，所以务必保证是合法的数字字符串。

## 日志 / 验证

插件输出 `l4n_material_proxy_plugin.log`（当前工作目录），同时在引擎控制台以 `[l4n-proxy]` 前缀输出。启动游戏进主菜单后，日志里应能看到代理注册、命中与生效记录：

```
[l4n-proxy] registered proxies: ["l4nrp_does_equal", "l4nrp_compare", ...]
[l4n-proxy] registered 15 proxies into CMaterialProxyDict
[l4n-proxy] Successfully installed
[l4n-proxy] RequestHudMenu(title): 已向 left4neko 注册 HUD 菜单入口
[l4n-proxy] RequestHudMenu(structure): callback=0x........ user_data=0x........
[l4n-proxy] 菜单回调被调用（user_data = "L4NRP HUD menu callback"）
[l4n-proxy] 菜单回调：已注册 15 个代理
[l4n-proxy] engine proxy: 'l4nrp_math' Init material=0x........ per_frame=true
[l4n-proxy] engine proxy: Release
[l4n-proxy] is_in_range[vgui/common/l4d_spinner]: $src_var=0.5000 in [$min_var=0.0000,$max_var=1.0000] -> $result_var2=1
[l4n-proxy] print_variable[vgui/common/l4d_spinner]: $var=0.5000 (float)
```

> `engine proxy: '...' Init ...` 每出现一次，就代表引擎在自己的 VMT 解析过程中实例化了一个 `l4nrp_*` 代理；
> 若 VMT 里写错了代理名，引擎会在控制台警告 `proxy not found`（本插件不会再拦截这类名字）。

## 开发者：如何新增一个代理

插件采用 Rust trait + 泛型注册架构，新增代理只需：

1. 新建一个 struct 实现 [`Proxy`](src/kv.rs) trait：`apply_kv` 填参数、`bind` 执行动作、`per_frame` 决定是否每帧执行；
2. 在 [`lib.rs`](src/lib.rs) 里用 `material::register_proxy::<T>("代理名")` 注册；
3. 在 VMT 的 `"Proxies"` 块里写上该代理名即可。

注册表会在 `OnModuleLoaded("client")` 时整体通过 `AddProxy` 写进引擎的 `CMaterialProxyDict`，
所以新增代理不需要改任何引擎侧的地址或 hook —— 加一行 `register_proxy` 就够了。

## 实现说明

- 代理是**引擎原生代理**：插件在 `OnModuleLoaded("client")` 时取到 `client.dll` 的
  `CMaterialProxyDict` 单例，为每个 `l4nrp_*` 调一次 `AddProxy(name, createFn)`；
  之后引擎解析 VMT 时会调 `createFn` 得到插件的代理对象，再调它的 `Init(name, kv)` 注入参数。
- **每帧执行靠引擎自己的 `OnBind`（零 hook）**：`per_frame()` 为真的代理直接在代理对象的
  `OnBind` 槽里执行 —— 引擎每次绑定该材质都会遍历代理数组并调用它。实机实测同一材质约
  **4.68 次/帧**，15 个代理全部幂等，所以不需要帧计数也不需要 hook。
  **v0.7.0 起不再 hook D3D9 `EndScene`**（旧方案会与 DXVK / 设备重建耦合）。
- 逆向依据（工厂地址、`Init` 真实 ABI、`ProxyObject` vtable 布局、堆归属处理等）见
  [`AGENTS.md`](AGENTS.md)。
