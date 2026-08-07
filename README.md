# rust_l4n_material_proxy_plugin

L4N 插件：**在 Rust 中注册自定义材质代理（material proxy）**。在 VMT 的 `"Proxies"` 块里写代理名，
即可触发插件回调，回调内可读写该材质的 VMT 变量（变色、比较运算、打印变量等）。

对于添加的材质代理详情，见[`Example.vmt`](/Example.vmt)。

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

   > 必须是**与 `left4neko.dll` 同目录的 `neko/plugins`**（即游戏安装目录下的
   > `bin/neko/plugins`），不是项目里的 `bin/neko/plugins`。

3. 启动游戏，left4neko 会自动遍历并加载该目录下的所有插件 DLL。

## VMT 用法

在材质的 VMT 文件 `"Proxies"` 块里写上代理名，以及可选参数：

```text
// some_material.vmt
"UnlitGeneric"
{
    "$basetexture" "vgui/common/l4d_spinner"
    "$color"       "[1 1 1]"
    "$src_var_1"   "1.0"        // does_equal 输入
    "$src_var_2"   "1.0"
    "$result_var"  "0"
    "Proxies"
    {
        "l4nrp_does_equal"
        {
            "src_a"   "$src_var_1"
            "src_b"   "$src_var_2"
            "result"  "$result_var"
        }
        "l4nrp_print_variable"
        {
            "src_var" "$result_var"
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
| `l4nrp_print_variable` | `var`→`$var`、`type`→`float`（`float`/`int`/`vector`） | 每帧读取变量并打印 |

### 注意事项

- 代理只能读写**已在 VMT 声明**的变量（引擎只为声明过的变量创建变量对象）。若想让代理写入某个
  变量，请先在 VMT 顶层声明它，例如 `"$result_var" "0"`。
- 插件与**原版/L4N 内置材质代理共存**：同一个 `"Proxies"` 块里 `Sine`/`Multiply`/`Sequence`（L4N的材质代理） 与 `l4nrp_*`
  可同时使用。

## 日志 / 验证

插件输出 `l4n_material_proxy_plugin.log`（当前工作目录），同时在引擎控制台以 `[l4n-proxy]`
前缀输出。启动游戏进主菜单后，日志里应能看到代理注册、命中与生效记录：

```
[l4n-proxy] registered proxies: ["l4nrp_color_ramp", ...]
[l4n-proxy] proxy parse hook installed ...
[l4n-proxy] apply_proxies: MATCH 'l4nrp_force_red' material=0x..
[l4n-proxy] force_red: "$color" = (1,0,0) (should appear red)
[l4n-proxy] is_in_range[vgui/common/l4d_spinner]: $src_var=0.5000 in [$min_var=0.0000,$max_var=1.0000] -> $result_var2=1
[l4n-proxy] print_variable[vgui/common/l4d_spinner]: $var=0.5000 (float)
```

## 开发者：如何新增一个代理

插件采用 Rust trait + 泛型注册架构，新增代理只需：

1. 新建一个 struct 实现 [`Proxy`](src/kv.rs) trait：`apply_kv` 填参数、`bind` 执行动作、
   `per_frame` 决定是否每帧执行；
2. 在 [`lib.rs`](src/lib.rs) 里用 `material::register_proxy::<T>("代理名")` 注册；
3. 在 VMT 的 `"Proxies"` 块里写上该代理名即可。

详细的接口定义、编写约定与实现注意事项见 [`AGENTS.md`](AGENTS.md)。
