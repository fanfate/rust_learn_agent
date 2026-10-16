# my_agent · 学习计划

> 建立：2026-10-16　｜　目标：**一边深入 Rust，一边真正理解 Agent 是怎么运作的**
> 上游文档：`../my_api/auth_refactor_progress.md`（收工复盘见其 §3.4；本项目路线源自其 **§五 / §六**）
> 约定：**代码自己写**，本文件只负责"别漏"和"别想错"——不含可复制的完整实现。

---

## 〇、当前状态

- ✅ 项目已 `cargo init`（`edition = "2024"`，`src/main.rs` 还是模板）
- ✅ 前置能力已在 `my_api` 练过，**V2 阶段直接复用**：Axum 路由/中间件 · SQLx + 迁移 · JWT 鉴权 · 错误分层 · 集成测试
- ✅ 本机工具链：`rustc 1.99.1` / `cargo 1.99.1`
- ⚠️ **前提已变更（2026-10-16）**：**放弃本地模型**（本机无 NVIDIA GPU、无 llama.cpp），**改用在线模型**——API key 已有。
  ⇒ `my_api` doc §5.1 那句"本地模型条件已具备"**作废**；§5.3 第 2 条"先云端验证再切本地"也随之简化（全程云端）。
- ✅ **配置方式已定案（2026-10-16）**：`.env`（不入库）+ `.env.example`（空壳入库），三变量
  `LLM_BASE_URL` / `LLM_MODEL` / `LLM_API_KEY`，用 `dotenvy` 读取 —— 详见 **§3.3.1**。
- ⏳ **唯一待定项**：用哪家 API / 什么模型名 → 只需填 `.env` 的两个值，**代码不用改**（这正是三变量抽象的目的）。
- ✅ **scratch 目录已定位（2026-10-16）**：`my_agent/scratch/`，语义是 **agent 的沙箱根**，不是"测试目录"—— 详见 **§3.5**。
- ✅ **`src/config.rs` 已成型（2026-10-16，**四版迭代**）**：`ConfigError`（thiserror，3 变体 `Missing` / `NotUnicode` / `Empty`）
  + 私有 `get_env` / `format_url` / `get_map` + `Config`（**手写 `Debug` 把 key 打 `"***"`**）+ `from_env()` / `from_map()`。
  `cargo check --all-targets` **零代码告警**（只剩 `target/` 增量锁文件噪音）。
  ⚠️ **仍留一处结构问题**：`from_env` / `from_map` 两条取值链从未交汇 → 见 **§3.3.2**。
- ⏳ `main.rs` 仍是模板（`fn main() -> anyhow::Result<()>`）。**下一个动作 = §3.9**：先发最朴素的 chat 请求、打印原始 JSON。

---

## 一、为什么要做这个（双目标，以及它们为什么不冲突）

### 1.1 两个目标是同一件事

| 这个项目**真在练 Rust** | 换 Python / TS 写是同一件事 |
|---|---|
| `Tool` trait 动态分发（`Box<dyn Tool>` + 注册表） | 调 LLM 的 HTTP 请求 |
| **serde tagged enum 表达工具参数**（最体现 Rust 表达力） | prompt 拼装 |
| 错误类型设计（**工具失败也要回灌给模型**） | JSON Schema 描述 |
| `Arc` / 共享状态（工具注册表、会话状态） | 流式 chunk 的字符串处理 |
| 取消与超时（`CancellationToken` + `tokio::select!`） | |
| async 生命周期、跨 await 的借用 | |

### 1.2 关键论据：Agent 引擎**不是 ML**

它没有矩阵运算、没有 CUDA、没有训练循环 —— 全是 **HTTP 协议 + 异步编排 + 类型建模 + 错误处理**。
也就是说，**Rust 生态最弱的那块（数据科学 / 深度学习）在这里根本不参与**，你正好落在它最强的那块：

- tagged enum 让「未知工具」在**编译期**就不可能出现（Python 是运行时查不到才报错）
- `match` 穷尽匹配 role / 状态，漏一个分支编译不过
- `Result` 把「工具失败了」变成类型，而不是被忽略的异常

### 1.3 顺便：语言选择不是劣势

"agent 都用 TS 写"只对了一半 —— **TS 主导应用 / 框架 / UI / 分发层，引擎层反而 Rust 强势**：

| 项目 | 语言 | 备注 |
|---|---|---|
| **OpenAI Codex CLI** | **Rust ≈95.7%** | 2025-04 首发是 **React + TS + Node**，两个月后宣布 "going native"；`codex-rs/` ≈70 crates，`codex-cli/` 只剩 npm 薄壳 |
| **Block Goose** | **Rust ≈58% + TS ≈34%** | TS 那份**只做桌面 UI**；已捐给 Linux Foundation AAIF |
| Claude Code / Cline / Vercel AI SDK | TS | 应用层 |
| Aider / OpenHands | Python | 研究 / 脚本层 |

Codex CLI 重写的三条理由**全是引擎层理由**：① 零依赖安装（Node v22+ 是硬依赖）② **无 GC 停顿**（长时运行的 agent 累积 history / 工具结果 / diff，堆持续膨胀）③ **原生沙箱**（想直接调 macOS Seatbelt / Linux Landlock，而非 FFI shim）。第三条对**本项目 §3.5 的安全边界**尤其对味。

### 1.4 唯一的代价：迭代速度

Python 改一行 prompt 直接跑；Rust 要等编译，还要跟所有权较劲。这段摩擦是**练习成本**，不是设计缺陷。
**实用对策**：把"发现设计"和"实现设计"分开 —— 想快速试一种循环变体时，先用 Python 打个草稿，想法确认后再用 Rust 写一遍。别让编译错误变成探索的瓶颈。

---

## 二、路线 V0 → V3

| 版本 | 范围 | 交付物 | 退出标准 |
|---|---|---|---|
| **V0** | 无框架 · 非流式 · CLI · 单文件约 300 行 | 一个能用的命令行 agent | 见 §三·3.6（含 **dogfooding 一次真实修复**） |
| **V1** | 手写 SSE 流式 + 取消/超时 + 上下文管理 | 可 Ctrl-C 中断、不撑爆上下文的版本 | 流式分片能正确跨 chunk 累积 `tool_calls` |
| **V2** | 接回 `my_api`：会话落 sqlite + Axum 包一层 | `POST /chat` + SSE / WebSocket | 复用 `CurrentUser` 做到会话按用户隔离 |
| **V3** | MCP · 多后端抽象 | — | 到这一步才真正知道抽象该划在哪 |

**当前唯一在做的：V0。** V1 之后的事不提前设计。

---

## 三、V0 详细

### 3.1 范围

**做**：
- 单文件 `main.rs`（约 300 行），CLI，**非流式**，**不用任何 agent 框架**
- **4 个工具**：`list_files` / `read_file` / `write_file` / `run_command`
- 核心循环：`Vec<Message>` → `POST /v1/chat/completions` → `tool_calls` 非空则逐个执行、结果 push 回 messages → 继续；为空则输出文本、结束
- 安全边界（§3.5）——**不是可选项**

**不做（明确留到后面）**：流式、持久化、多会话、多后端抽象、MCP、重试/限流、UI。

### 3.2 要自己定的三个类型决定（只列选项，不给结论）

这三处是这个项目**最值钱的 Rust 练习点**，值得先想清楚再动手：

1. **`Message` 怎么建模？**
   `struct Message { role: Role, content: ..., tool_calls: ... }` 还是 tagged `enum Message`？两种都能走到终点 —— **各自在序列化、匹配、默认值上的代价是什么？**
2. **工具参数怎么建模？**
   `enum ToolCall`（每个变体带自己的参数结构）还是 `struct ToolCall { name: String, args: serde_json::Value }`？
   后者好写，前者才是 doc §5.2 里"最能体现 Rust 表达力"的那一条。**先想清楚一个问题：「未知工具」该在编译期挡掉，还是运行期挡掉？**
3. **`LlmClient` 用哪种分发？**
   `trait` + 泛型（静态分发）还是 `Box<dyn LlmClient>`（动态分发）？—— 这正好对上 doc §5.2 的「**trait object 动态分发**」。

### 3.2.1 先把这条想清楚：`impl` / `dyn` / `Box` **不是三个并列选项**（2026-10-16）

§3.2 ③ 问"`LlmClient` 用哪种分发"，答案取决于**两个正交维度**，别当成三选一：

- **分派方式**：`impl Trait`（泛型单态化 → 静态分派、可内联）↔ `dyn Trait`（虚表 → 动态分派）
- **所有权**：`&T`（借）↔ `Box<T>` / `T`（拥有）

四格都有名字：`&impl Fn(..)` · `&dyn Fn(..)` · `impl Fn(..)`（按值）· `Box<dyn Fn(..)>`。
**`Box` 那个字母解决的是"所有权"，不是"分派"。** 判据只有一句话：

> 先问「**我需要拥有它吗**」。需要（要存进 struct 字段 / `Vec`、要当返回值从函数里吐出来）才谈 `Box`；
> 不需要，就只在 `impl` 与 `dyn` 之间选。

⚠️ **`Box<dyn Trait>` 省略时默认就是 `+ 'static`**（对象生命周期默认规则）——实测报错原文：

```
note: due to object lifetime defaults, `Box<dyn Fn(&str) -> Option<String>>`
      actually means `Box<(dyn Fn(&str) -> Option<String> + 'static)>`
```

后果：想借一个局部 `HashMap` 当测试夹具会直接 `E0597`，必须 `move` 拷进去、或手写 `+ '_`。
而**借引用（`&impl` / `&dyn`）时闭包类型无需 `'static`** —— 这也正是 §3.3.2 里 `from_map` 能收 `&impl Fn` 的原因。

代价对照（实测）：`impl` 走单态化，**每多一种闭包类型就多一份机器码**（两个不同闭包类型调同一个泛型函数，
`--emit=llvm-ir` 里出现 **2 个 `define`**）；`dyn` 只编译一份。函数体小时忽略不计；
**函数体大、且调用方闭包种类多**，才值得换 `&dyn`（注意：`&dyn` 同样不需要 `Box`）。

> **映射到本项目**：§3.7 那句"把 LLM 调用藏在一个 trait 后面"——若只是**调用**它，`&dyn LlmClient` 甚至泛型就够；
> `Box<dyn LlmClient>` 只在**要把它存进结构体字段**时才必要。反之 §1.1 里 `Box<dyn Tool>` 的**工具注册表**
> 确实需要拥有（要放进 `Vec`），那里 `Box` 是对的。**同一个 `dyn`，两个场合两个答案。**

### 3.3 依赖（版本已实测核实，非记忆）

```toml
[dependencies]
tokio = { version = "1.53", features = ["rt-multi-thread", "macros", "time"] }
reqwest = { version = "0.13", features = ["json"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
anyhow = "1.0"          # 应用层错误收口
thiserror = "2.0"       # 领域错误（工具失败那种）分层用
dotenvy = "0.15"        # 读 .env 拿 API key
clap = { version = "4.6", features = ["derive"] }   # CLI 参数
```

V0 后半段再加：`tracing` + `tracing-subscriber`（结构化日志）。
流式相关的 `eventsource-stream` / `futures-util` **留到 V1**。

> ⚠️ **`reqwest 0.13` 的 TLS 与 0.12 不同，注意这个坑**（已查 crates.io 上的 feature 表）：
> `default-tls = ["rustls"]`，而 `rustls = ["__rustls-aws-lc-rs", ...]` ⇒ **默认走 rustls + aws-lc-rs**，
> 后者在 Windows 上可能要求 **CMake / NASM** 才能编过。
> 如果 `cargo build` 卡在 TLS 依赖上，**换系统 TLS 后端**（Windows = schannel，零额外工具链）：
> ```toml
> reqwest = { version = "0.13", default-features = false, features = ["json", "native-tls"] }
> ```
> （`rustls-tls` 这个 feature 名**在 0.13 里已不存在**，别照抄 0.12 的写法。）
> 选型原则与 `jsonwebtoken` 那次一致：**优先无 C 工具链的方案**。

### 3.3.1 配置：`.env` + `.env.example`（2026-10-16 定案）

成熟且通用的一套（12-factor 的本地化写法，Hello-Agents 也是这么做的）：

| 文件 | 入库 | 内容 |
|---|---|---|
| `.env` | ❌ **必须 gitignore** | 真实值（含 key） |
| `.env.example` | ✅ **入库** | **空壳 / 占位**，只列变量名 —— 它本身就是"需要配哪些"的文档 |
| 真实环境变量 | — | CI / 部署时直接注入，**优先级高于 `.env`** |

**三个变量（够用，不再加）**：

```
LLM_BASE_URL=https://<host>/v1     # ← 建议含 /v1，见下面约定 ①
LLM_MODEL=<model-name>
LLM_API_KEY=sk-...
```

⚠️ **两个约定要先定死，否则后面全是隐性耦合**：

1. **`LLM_BASE_URL` 是否含 `/v1`？** —— 建议**含**（示例里就带上），**代码里只往后拼 `/chat/completions`**。
   把"厂商差异"全部压进环境变量，代码只认一种形状。（否则 `base_url` 尾斜杠的有无、`/v1` 的有无，会变成三个 case 的分支。）
2. **读取顺序**：`dotenvy::dotenv().ok()` —— 用 **`.ok()` 而不是 `?`**：有真实环境变量时没有 `.env` 也能跑。
   `dotenvy` 默认**不覆盖**已存在的环境变量 ⇒ 天然形成"**真实环境变量 > `.env`**"。

**建议的落地形状**：启动时把三变量读进一个 `Config` 结构体，**缺哪个就带名字报错**（`anyhow::Context`），
**不要拖到发请求时才 `unwrap`**。这是 V0 第一个真实的 Rust 小练习：**依赖在边界处一次性校验**。
（顺手一条：**永远不要把 key 打进日志**。）

> ⚠️ **第一件该做的事**：`.gitignore` 里**原本没有 `.env`** —— 一旦 `git add .` 就永久进历史。
> **已代为加上**（同时加了 `scratch/`）。key 泄漏是这类项目最常见的翻车方式。

### 3.3.2 `config.rs` 的四版演化与遗留决策（2026-10-16）

实际写下来的过程比预想曲折，**四版的差异本身就是这一节最值钱的部分**：

| 版 | 改了什么 | 为什么 |
|---|---|---|
| 1 | `ConfigError::Missing` + `get_env` + `get_map` + `from_env` / `from_map` | 骨架 |
| 2 | 加 `Empty` / `NotUnicode` 变体；`match` 拆开 `VarError` 两态；加 `format_url` | 空值原本会**放行到发请求才 401**；"非 UTF-8"与"未设置"被揉成一种 |
| 3 | 加 `impl Display for Config`，把 key 打 `"***"` | ← **没解决问题**，见下 |
| 4 | 去掉 `#[derive(Debug)]`，**手写 `Debug`**；变体名 `NotAllowed` → `NotUnicode` | ✔ 这才是对的 |
| 5 | 新增**无字段**的两态 `ValueError` 作为注入边界；`get_env` 改返回它；`from_env` 委托 `from_map` | 方案 C 落地，两条链收敛（撞到 HRTB 坑，见下） |

#### ⚠️ 版 3 的教训：`Display` 不能代替 `Debug`

`Display` 与 `Debug` 是**两个独立 trait、两个独立调用点**。给 `Config` 加 `impl Display` 把 key 打 `"***"`，
`{:?}` 照样原样打印真 key —— 实测（把结构原样搬出来跑）：

```
Display  Config { ..., llm_api_key: "***" }
Debug    Config { ..., llm_api_key: "sk-REAL-..." }   ← 漏
```

而**日志生态里最常用的是 `{:?}`**：日志宏惯例、`dbg!`、`assert_eq!` 失败信息、`unwrap` / `expect` 对 `E` 的
`Debug` 要求，全走 `Debug`；`{}` 反而少见。⇒ **"绝不打 key 进日志"要遮的是 `Debug`。**
版 4 的手写 `Debug` 是正解；更彻底的做法是把 key 包成 `Secret(String)`，两个 impl 都遮 `"***"`，
以后别的结构体带 key 直接复用。

#### ✅ `format_url` 不是死代码（AI 曾判错，此处更正）

`format_url` 是**边界归一化器**：判据不是"happy path 里有没有被触发"，而是"**对任何合法输入都产出唯一规范形式**"。
`.env` 是用户手写的输入面（`.../v1/` 带尾斜杠是从文档、curl 例子里抄下来的常见写法），归一化放在加载边界、
存成唯一内部形式，位置正确。**别把"测试覆盖"的标尺套到"输入归一化"上。**
（边界行为已实测：`"http://x/v1//"` → `"http://x/v1"`；`"/"` → `""`，边角可不管。）

#### ⚠️ 遗留：两条取值链从未交汇（当前唯一实质问题）

`from_env` → `get_env` → `std::env`；`from_map` → 闭包。**两份独立实现** ⇒ **生产路径不经过可测内核**，
`from_map` 的测试全绿也**证明不了** `from_env` 对。

第 3 次迭代把这个代价变成了**具体可见的不一致**：`NotUnicode` **只有 `get_env` 能产生** ——
因为 `get_map` 的闭包边界 `Fn(&str) -> Option<String>` 已经把"为什么没有"**擦除**成了 `Option`。
⇒ **同一个非 UTF-8 环境变量走两条链得到不同变体**（`NotUnicode` vs `Missing`）；
⇒ **一旦把 `from_env` 委托给 `from_map`，`NotUnicode` 会静默失效**。

三条路，**推荐 C**：

| 方案 | 保住 `NotUnicode` | 单一内核（生产走被测内核） | 额外成本 |
|---|---|---|---|
| A 委托 + 删掉该变体 | ✗ | ✔ | 无 |
| **C 把"原因"带过边界** | ✔ | ✔ | 一个小两态枚举 |
| D 什么都不改 | ✔ | ✗ | 把分叉写成注释 / 一条测试钉住 |

**C 的关键（唯一一处改动）**：把注入边界的返回类型从 `Option<String>` 改成 `Result<String, 小枚举>`。
⇒ `get_map` 自己 match 出 `Missing` / `NotUnicode`，`name: &'static str` **完全不用动**；
`from_env` 传一个**纯适配器**进去 —— 这个适配器只做一件事：把 `std::env::var` 的 `VarError` 翻译成那个两态小枚举；
它**不认识 `ConfigError`**（造 `ConfigError`、拼 `name` 是内核的事），所以它的错误类型**只有两个不带字段的变体**
→ **`from_env` 就此走内核**；测试闭包不碰 `std::env`，**三个变体全都能造**。（C 已实测：脱离真实环境变量时三态全部可构造。）

> ⚠️ 上一版给的 A/B 二选一是**框窄了**：B 之所以要级联把 `name` 改成 `String`，
> 是因为让**闭包去构造 `ConfigError`**（闭包参数是自然的 `&str`，撞上 `ConfigError` 要的 `&'static str`）。
> 只要闭包只报告"**为什么没有**"、`ConfigError` 的构造留在内核，级联就消失。
> **教训：注入边界的类型决定了内核能知道多少 —— 选 `Option` 就是主动丢信息。**

##### 为什么边界不复用 `ConfigError` 本身（第 4 次追问，已实测）

直接用**能编译、行为也一样**（同一组用例两条路输出完全相同）。代价在类型说了什么：

1. **边界错误集 ≠ 内核错误集** —— 边界只可能遭遇 `std::env::var` 的两态（没这个变量 / 不是 UTF-8），
   而 `ConfigError` 还有 `Empty`。**"空"不是读取失败，是值的属性** —— 只有内核（拿到 `String` 之后）才可能知道。
   ⇒ 内核要么 `Err(e) => Err(e)`（**交出"造哪个错误"的决定权**，退化成一个透传管道），
   要么穷尽 match 出**一个永不可达的分支**。
   **实测**：把那个分支写成 `unreachable!()`，再喂一个"乱来的"闭包（返回 `Empty`）→ **真的 panic 了**。
   ⇒ **契约靠自觉，不靠类型**。换成两态枚举后，那个非法实现**根本写不出来**（编译器直接拒绝）。
2. **领域知识被迫下放** —— 适配器得知道 `NotPresent` 该翻译成 `Missing`，还得把 `name` 抄进错误。
   两态版的适配器输出里**连"是哪个配置项"都不含**，是纯中立的读取结果。
3. **`ConfigError` 被提升成公共注入协议** —— `from_map` 签名一写 `Fn(&str) -> Result<String, ConfigError>`，
   任何实现者（测试 / mock / 以后的别的配置源）都必须 `use` 它，还得各自判断 `Empty` 该不该返回。

**也否决"直接用 `std::env::VarError`"**（std 现成，实测 `Clone + PartialEq + Debug` 都有，测试里造得出来）：
它会把 **`std::env` 的类型泄漏进内核签名**（内核的 match 变成 `VarError::NotPresent`）⇒ **内核也"认识 `std::env`"了**，
而 `from_map` 存在的**全部理由**就是让内核不认识全局态 —— 等于自毁动机。

> 判据：**边界的错误类型应 1:1 对应它可能遭遇的失败**。两个集合不等（2 个无字段 vs 3 个带 `&'static str`）
> 就该是两个类型。用大的那个 = 让边界承诺一件它做不到的事。
> 对称地，那个两态小枚举描述的是"`std::env::var` 的失败模式"，跟配置无关 → 它是可复用的通用小类型，
> 不算"为配置特设"，也谈不上过度设计（过度设计是"每个函数配一个专属 Error"）。
> 类比 `my_api`：`sqlx::Error → ApiError` 的转换在 repo / handler 层，不在 sqlx 内部。

#### 两条硬事实（已实测）

- **`std::env::set_var` 在 `edition 2024` 要求 `unsafe`** —— 编译器在用类型系统提醒你它会**污染整个进程**、
  与并行跑的测试互撞。**这正是 `from_map` 存在的理由**：把全局态换成注入。
- **`use std::fmt::Debug;` 在本文件里是冗余的**（`impl std::fmt::Debug for Config` 写的是全限定路径），
  但 `rustc` **不会报**——因为 `#[derive(Debug)]` 出现在 `ConfigError` 上，同名解析让 lint 认为它被用了。可删，纯清理。

#### ⚠️ 落地 C 的坑：闭包参数不能留 `'static`（HRTB，已实测）

第 5 版落 C 时撞到的报错**不是**常见的类型不匹配，而是：

```
error: implementation of `Fn` is not general enough
   = note: closure with signature `fn(&'2 str) -> _` must implement `Fn<(&'1 str,)>`, for any lifetime `'1`...
   = note: ...but it actually implements `Fn<(&'2 str,)>`, for some specific lifetime `'2`
```

关键词是 **`for any lifetime` vs `for some specific lifetime`**。
`impl Fn(&str)` 脱糖为 **`for<'a> Fn(&'a str)`**（HRTB），承诺"**对任意生命周期**都能调用"；
而 `get_env(name: &'static str)` 只承诺"对 `'static` 能调用" ⇒ **承诺少的实现不了承诺多的接口**。

> 方向（实测）：把上游 bound 写成 `Fn(&'static str)` 时，泛化 fn / 泛化闭包 / `'static` 闭包**三个全过**；
> 反之不行。⇒ **作为 trait bound，`&str` 比 `&'static str` 更严格** —— 和"`&str` 更通用"的直觉相反。

**为什么 `get_env` 的 `'static` 是多余的**：它内部只把 `name` 交给 `std::env::var`，而
`var<K: AsRef<OsStr>>(key: K)` 接受任意生命周期的 `&str`，没有 `'static` 需求。那个 `'static` 是
**第 1 版遗留**（当时 `get_env` 直接返回 `ConfigError`，被 `Missing { name: &'static str }` 逼的）；
C 落地后返回类型换成无字段的 `ValueError`，**约束已失去存在理由却没被清掉**。
对比 `get_map(name: &'static str)` —— 那个是**真必需**（它要往 `ConfigError::Missing { name }` 里塞）。
⇒ **教训：改了返回类型要回头检查参数上的生命周期约束是否还站得住。**

**修复**：`fn get_env(name: &str) -> Result<String, ValueError>`。之后 `from_env` 可以直接
`Self::from_map(&get_env)` —— fn item 本身满足 `Fn(&str) -> Result<String, ValueError>`，**连闭包都不用写**。
✅ **已按此修复（用户当晚改好，`cargo check --all-targets` 通过）。**

**附带一个坑**（实测）：若保留闭包写法，**参数类型必须显式写 `&str`**，编译器不会替你推 HRTB ——
`let c = |n| get_env(n);` ✗（推成某个固定生命周期）；`let c = |n: &str| get_env(n);` ✔。
rustc 会直接把答案写在 help 里：`consider adding an explicit type annotation to the closure's argument`。

**实测验证**：改成 `&str` 后四种写法全过 —— fn item / 显式闭包 / 测试夹具闭包 / 生产路径 `from_env`。

#### ⚠️ 第 5 版新暴露：`pub fn from_map` 的参数里藏着私有类型（`private_bounds`）

```
warning: type `ValueError` is more private than the item `Config::from_map`
  --> src\config.rs:74:5
note: but type `ValueError` is only usable at visibility `pub(self)`
```

`pub fn from_map(get: &impl Fn(&str) -> Result<String, ValueError>)` —— **声明了 `pub`，签名里却是私有类型**
⇒ 外部调用者**写不出**一个返回 `ValueError` 的闭包（要 `Err(ValueError::Missing)` 就得够得着它）
⇒ **这个 `pub` 是空头支票**，实际只对 crate 内可用。

这恰好是"边界用哪个错误类型"那个决策的**镜像**：
**把它塞进公开签名 ⇒ 它成了公共协议；不塞又不改可见性 ⇒ 公开签名成了空头支票。**
⇒ **`from_map` 应为 `pub(crate)`**（它只服务 `from_env` 与 `tests`，没有任何外部调用者）；`from_env` 保持 `pub` 是对的。
（若哪天真要对外支持"自定义配置源"，再把 `ValueError` 提升为 `pub`，并按正式 API 设计，加 `#[non_exhaustive]`。）

#### `ValueError` 要不要 `thiserror::Error`？—— 可以不要（已实测）

`thiserror` 给三样东西，**`ValueError` 一样都没用上**：

| 它给什么 | 用到了吗 |
|---|---|
| `impl Display`（`#[error("...")]` 生成） | ✘ **从未被打印** |
| `impl std::error::Error`（能进 `Box<dyn Error>` / 当 `source`） | ✘ **从未传播出去** |
| `#[from]` 生成 `From` 转换 | ✘ **用的是手写 `match`** |

grep 整个 crate：`ValueError` 只出现在 config.rs 的 5 处（定义 / `get_env` 返回 / `get_map` 参数 / 两处 `match` 臂）；
`dyn Error`、`.source()`、`#[from]`、`#[source]`、`bail!`、`.context()` **一个都没有**。

**`?` 也不需要它**：`get_env` 里那个 `?` 是**恒等**转换（`map_err` 之后 E 已经是 `ValueError`，函数返回也是 `ValueError`）。
真需要 `From` 的写法长这样 —— 实测报错：

```
error[E0277]: `?` couldn't convert the error to `ConfigError`
note: `ConfigError` needs to implement `From<ValueError>`
= note: the question mark operation (`?`) implicitly performs a conversion on the error value using the `From` trait
```

而 **`From<ValueError> for ConfigError` 造不出来** —— `ConfigError::Missing` 需要 `name`，`From` 手里没有。
⇒ **C 的代价就是"必须手写 `match`"，收益是"内核掌握造错的全部信息"。**
"要不要 thiserror"跟这是同一件事的两面：**既然永远用不上 `Error` 的机制，就别实现它。**

**建议**：`#[derive(Debug, Clone, Copy, PartialEq, Eq)]`。
（`Debug` 必需 —— `unwrap` / `expect` 要求 `E: Debug`，`assert_eq!` 失败信息也走 `Debug`；
`PartialEq` 供测试断言；零字段枚举 `Copy` 免费。）
顺手删掉 `#[error("值不存在")]` / `#[error("值不是utf-8")]` —— **永不打印**，且文案本身含糊（哪个值？）。
（实测：去掉后全部代码路径 + 断言照常工作。）

**诚实的一句**：留着**也不算错**。thiserror 已在依赖里（`ConfigError` 在用），**零依赖增量**；
两个变体展开约 10 行，编译成本可忽略。真正的成本是**认知成本** ——
读者看到 `thiserror::Error` 会默认"这是个会向外传播的错误"，而它其实是一个**活不过一行的中间值**。

**判据**：**这个类型会不会被 `?` 抛出函数边界，或被别人当 `source` 包起来？**
会 → `Display` + `Error` 值得（thiserror 就该上）；不会（在同一函数里被 `match` 消费掉）→ 只要 `Debug`（+ 断言用 `PartialEq`）。

| | `ConfigError` | `ValueError` |
|---|---|---|
| 跨函数边界传播 | ✔（→ `main`） | ✘（活不过 `get_map` 一行） |
| 被 `Display` 打印 | ✔（给用户看的错误信息） | ✘ |
| 进 `source` 链（anyhow） | ✔ | ✘ |
| 字段插值 `{name}` | ✔ | ✘（无字段） |
| `#[from]` | 可用 | **用不上**（缺 `name`） |
| 结论 | **留 thiserror** | **可退到 `#[derive(Debug, PartialEq)]`** |

**顺手两条**：
- `src/scratch/` 是空目录 —— 按 §3.5，`scratch/` 应在**项目根**（`my_agent/scratch/`），不在 `src/` 下。
- `ConfigError` 目前**没有 `PartialEq`**。测试里想 `assert_eq!` 就得加；更好的选择是用 **`matches!`**
  （不要求 `PartialEq`，也不必给生产类型加测试专用的 derive）。

#### 下一步：`config.rs` 的测试（先不写代码）

`#[cfg(test)] mod tests` 正是 `from_map` 这条缝的用途。先自答：

1. 测 `from_map` 时闭包从哪拿数据？为什么这里**不能**用 `env::set_var` 造场景？
2. `Missing` / `Empty` 各一条测试就够，那 **`NotUnicode` 怎么测**？（答不出来，就是在告诉你该选 A 还是 C）
3. `format_url` 的行为边界：`"http://x/v1"`、`"http://x/v1/"`、`"http://x/v1//"`、`"/"` 分别得到什么？
4. `from_env` 要怎么写、测什么，才算"这条测试真的覆盖了生产路径"？

### 3.4 协议备忘：`tool_calls` 报文解剖

这是 V0 真正要建模的对象，**先看清形状再定类型**：

```jsonc
// ① 你发出去的：声明工具（JSON Schema）
"tools": [{ "type": "function",
  "function": { "name": "read_file", "description": "…",
    "parameters": { "type": "object",
      "properties": { "path": { "type": "string" } },
      "required": ["path"] } } }]

// ② 模型回给你的：要求调工具
"message": { "role": "assistant", "content": null,
  "tool_calls": [{ "id": "call_abc", "type": "function",
    "function": { "name": "read_file",
      "arguments": "{\"path\":\"src/main.rs\"}" } }] }   // ← 注意

// ③ 你回灌的：工具结果
{ "role": "tool", "tool_call_id": "call_abc", "content": "…" }
```

**`arguments` 是 JSON 字符串，不是对象。** 这是最常见的坑，也是你在 Rust 里必须**二次反序列化**的原因 ——
外层解出 `tool_calls`，内层再 `from_str` 成你的枚举。它正好就是 §3.2 第 2 条那个类型决定的落点。

### 3.5 安全边界（V0 必须，不是可选项）

Agent 能 `write_file` + `run_command`，等于把「任意文件写入 + 任意命令执行」交给一个**概率模型**。

#### scratch 是什么（先说清，否则"沙箱"只是个词）

`my_agent/scratch/` = **agent 被允许操作的文件系统的根**（sandbox root）—— 它**不是**"测试运行目录"这么轻。三个身份：

1. **安全边界（执行点）** —— 工具里**所有**路径（`read_file` / `write_file` / `list_files`）都以它为根解析，
   **拒绝一切逃逸**（`../`、绝对路径、符号链接）；`run_command` 的 **`cwd` 也钉死在这里**。
   下面那几条（白名单 / 确认 / 截断）是**机制**，scratch 是这个机制**生效的地方**。
2. **靶场** —— 要修的目标代码**放进去**（`git clone <my_api> scratch/my_api`），agent 在这份**副本**上动手。
3. **一次性的** —— 里面全是可弃物（克隆来的嵌套 `.git`、agent 改烂的文件），**已加进 `.gitignore`**。

> 别和另两个"目录"混：`my_agent/` 是你写 **agent 本体**的地方；`tests/` 里若需要临时工作区，
> 用 `tempfile` **现造现删** —— 那是测试夹具，**和 scratch 不是一回事**。

⚠️ **路径沙箱在 Rust 里的一个真坑**：别用**字符串** `starts_with` 判前缀 ——
`scratch = /a/b` 而路径 `/a/bc/x` 会被**误判为"在内部"**。
正确做法：先 `std::fs::canonicalize`（**顺带解掉 `..` 与符号链接**），再用 **`Path::starts_with`**
（按**路径分量**比较，不是子串）判断。这条值得自己写一遍、再故意撞一次。

- ⚠️ **只在 scratch 里跑**，**绝不指向真实项目**
  - ⚠️ **修正上一轮的说法**：dogfooding 的靶子是 **`my_api` 的副本**（`git clone` 到 scratch 下），**不是本体**。
    拿一个还在调试的实验性 agent 去写 `my_api`，风险与收益不成比例。
- **写操作与 shell 走人工确认**：先打印 diff / 待执行命令，回车才执行（这也是真实 agent 的标准设计）
- **工具白名单**，不做万能 shell；命令限定在 `cargo check` / `cargo test` 这类
- **工具输出截断**，别把整个文件塞回上下文

### 3.6 验收标准

**功能**：在 scratch 里放一段编译不过的代码（用 `my_api` 的副本），对 agent 说"让它通过编译"，它自主完成
**读文件 → 改 → `cargo check` → 读报错 → 再改**，**中途不需要你介入**。

**"能真实用起来"是验收标准，不是范围驱动** —— 它只问一件事：**做完之后，你愿不愿意每天真的打开它？**
门槛比想象的低：**比的不是功能数，是可靠性**。一个只会做一件事但每次做对的 agent，胜过 20 个工具却总改错文件的框架。

> 需求的正确来源是 **dogfooding，不是设计**。三个小时之内你会亲自撞到真正的需求：上下文塞不下、它改了错的文件、
> 它在同一个错误上循环。**这些是设计不出来的。**（就像你在 T2 里第三次才撞到"提权时序"一样。）

### 3.7 工程化分档

判据只有一条：**这项工程化，每小时能让你学到多少 Rust，以及是否直接提升可靠性 / 可测性 / 安全性。**

| 档 | 内容 |
|---|---|
| **V0 就做** | 错误分层（领域错误 `thiserror` + 出口 `anyhow`，**结论见 §3.7.1**）· 结构化日志 · 超时与重试 · **把 LLM 调用藏在一个 trait 后面**（分发选择见 §3.2.1）· 路径沙箱 · 命令白名单 · dry-run 确认 · 工具输出截断 · token / 成本统计 |
| **V1–V2 再做** | 会话持久化 · 上下文压缩 · 流式输出 · 多后端抽象 · MCP（**等真实需求出现，不提前设计**） |
| **对本目标不做** | 多 agent 编排 · 插件系统 · GUI · 打包分发 · 可观测性后端（每小时学到的 Rust 最少） |

**其中一项回报特别大，值得单独点出来 —— 把 LLM 调用藏在一个 trait 后面**：

```rust
trait LlmClient {
    async fn chat(&self, msgs: &[Message], tools: &[ToolSpec]) -> Result<Response>;
}
```

一个抽象换来三样东西：**单测不用联网**（喂一个 scripted 假 client，就能确定性地测「模型要求调工具 → 你正确执行 → 结果正确回填」，
否则 agent 的核心逻辑永远无法自动化测试）· 换模型只改一处 · 成本/token 埋点有统一入口。

> 这是整个 V0 里**唯一建议"提前"引入**的抽象。其余一律等第二处需要出现时再加。

### 3.7.1 错误策略分层（2026-10-16 定案，`config.rs` 已练过一遍）

#### 分界线：「**这个错误有没有人能处理**」

- `expect` / `unwrap` = **终止（panic）**，不是"报告"；`?` + `.context()` = 把错误**当值**交给上层。
- **可以 panic**：`Config` 加载失败、CLI 参数解析失败、**测试代码**（panic 就是测试的失败信号）。
- **绝不可 panic**：**agent 循环内部** —— LLM 调用 / 工具执行 / JSON 解析。三条理由：
  ① panic 掉之后**模型永远看不到失败信息，无法自我纠正**（这直接毁掉 §3.8 第 2 问的答案）；
  ② async 里的 panic 更糟（变成 `JoinError`，可观测性差）；
  ③ panic 不可测（那个分支测不出来）。

#### `thiserror` 与 `anyhow` 的分工：**库用 thiserror，二进制用 anyhow**

| | `thiserror` | `anyhow` |
|---|---|---|
| 用途 | **内核 / 领域错误**：可 `match`、可分类、可穷尽 | **二进制出口层**：类型擦除，只能 `downcast` |
| 形状 | `enum`（`ConfigError` 就是） | `anyhow::Error` + `.context()` |

两者**可共存**，机制是 `anyhow::Error` 对 `E: std::error::Error` 的 blanket `From` ——
所以签名叫 `anyhow::Result<T>` 的内部照样能用 typed error，`?` 自动转换。
本项目已落地：`ConfigError`（thiserror）在里，`from_env() -> anyhow::Result<Config>` 在外。

#### 几个容易踩的点

- **`.ok()` 会静默吞错**（`Result` → `Option`，错误丢掉；反向是 `.ok_or(e)`）。
  用它之前必须能回答「**这个错误真的可以不管吗**」。`dotenvy::dotenv().ok()` 属正当（没有 `.env` 也该能跑）。
  §3.3.2 里 `from_map` 的闭包用 `Option` 就是这类"主动丢信息"付学费的真实案例。
- **`anyhow::Ok` 是真实存在的自由函数**（带 `#[allow(non_snake_case)]`），是 `E` 推断失效（**E0282**）时的备用写法，
  等价于 `Ok::<_, anyhow::Error>(v)`，常见于闭包 / iterator 链。
  **别 `use anyhow::Ok;`** —— 会遮蔽 prelude 的 `Ok`，报错位置离根因很远。需要时写限定路径。
- **错误要结构化，不要字符串**：`Missing { name }` 让人一眼知道缺哪个变量；拼成
  `"缺少环境变量 LLM_API_KEY"` 就丢了可编程性（`ConfigError` 已经做对了）。
- ⚠️ **全局态是测试的敌人**：`std::env::set_var` 在 edition 2024 是 `unsafe`（它会污染整个进程、
  与并行跑的测试互撞）。凡是依赖全局态的函数都难可靠测试 ——
  **这才是"依赖注入"（§3.3.2 的 `from_map`）的真实动机**，不是什么优雅癖。

### 3.8 要自答的 4 个问题（答完再动手）

1. `tool_calls` **为空**和模型返回**纯文本**，哪个才算"结束"？两者一定会一起出现吗？
2. 工具执行返回 `Err` 时，回灌给模型的 `content` **应该长什么样**？（这决定模型能不能自己纠正）
3. `run_command` 的白名单，怎么表达才算"**不靠字符串判断就安全**"？
4. 历史每轮全量重发 —— 什么时候会撑爆上下文？**V0 要不要先不管这件事？**（先想清楚"为什么可以先不管"）

### 3.9 第一动作

**不要**先建 `Tool` trait。先只写一件事：

> 发一个最朴素的 chat 请求，把**响应的 JSON 原样打印出来**。

理由：你得先**亲眼看到** `tool_calls` 那个字段长什么样、`arguments` 到底是字符串还是对象、`finish_reason` 有哪些值，
才知道 trait 该怎么定义。**这一步和上个项目里"先写 `GET /health` 那一条"是同一个套路。**

---

## 四、外部资料怎么用：Datawhale《Hello-Agents》

Python 教程（21.9k★）。**当"协议参照 + 概念词典 + 失败模式清单"，不当教程跟做。**

| 部分 | 内容 | 用法 |
|---|---|---|
| ch1-3 概念 / LLM 基础 | 术语、范式谱系 | 当词典用（对齐术语） |
| **ch4 手写 ReAct** | 循环、工具声明、结果回填 | **V0 直接参照** |
| ch5-7 低代码与框架 | Coze / Dify / LangGraph / 自研框架 | **跳过** |
| ch8-12 高级机制 | Memory / 上下文工程 / MCP / 训练 | V1–V2 再回来（RL 跳过） |
| ch13-16 案例与毕设 | 旅行助手 / 深度研究 / 赛博小镇 | 只当灵感 |

**两个必须警惕的地方**：

1. **看清它用的是哪种 ReAct。** 老写法是在 prompt 里要求模型输出 `Thought: … / Action: …` 再**正则解析**；
   新写法用原生 **`tools` 参数**。⚠️ 前者不要学 —— 那是 function calling 普及前的过渡方案。
2. **框架章跳过。** 它教的是「怎么把循环抽象掉」，而本项目的价值恰恰在**不抽象**（这也是为什么不用 `async-openai` 而手写 `reqwest`）。
   更要紧的是：它的 Python OOP 设计（`class Agent` / `class Tool`）会把你带向**错误的 Rust 类型设计** ——
   Rust 侧该是 `enum` + `trait`，而不是照抄那套分层。

**教程给不了的（这才是本项目的意义）**：教程教"**要什么**"，Rust 教"**怎么保证**"。

---

## 五、进度记录

**V0**

- [x] `.env.example` 入库（三变量空壳）+ `.gitignore` 加 `.env` / `.env.local` / `scratch/`（§3.3.1）
- [x] `Config` / `ConfigError` 成型，`dotenvy` 读三变量（§3.3.1 / §3.3.2，**四版迭代**；`cargo check` 零代码告警）
- [ ] **收敛 `from_env` / `from_map` 两条链**（§3.3.2 → 方案 **C**，推荐）
- [ ] `config.rs` 的 `#[cfg(test)] mod tests`：三变体 + `format_url` 边界（§3.3.2 末 4 问）
- [ ] 建 `.env` 本体，填入真实 `base_url` / `model` / `key`
- [ ] 发一个最朴素的 chat 请求，打印原始 JSON 响应（§3.9）
- [ ] 确定 API 的 `base_url` + `model`（填进 `.env`，代码不动），跑通一次带 `tools` 声明的调用
- [ ] `Message` 类型的决定落定（§3.2 ①）
- [ ] 工具参数类型的决定落定（§3.2 ②）
- [ ] `LlmClient` trait 立起来（§3.7）
- [ ] 4 个工具实现：`list_files` / `read_file` / `write_file` / `run_command`
- [ ] 循环接上：`tool_calls` 非空 → 执行 → 回灌 → 继续
- [ ] 终止条件 + 步骤上限（防死循环）
- [ ] 安全边界落地：scratch 目录 + 写/shell 确认 + 命令白名单 + 输出截断
- [ ] 自答 §3.8 的 4 个问题
- [ ] **验收**：拿 `my_api` 副本跑通一次真实修复（§3.6）

**V1**

- [ ] 手写 SSE 流式解析（含 `tool_calls` 跨 chunk 累积）
- [ ] `CancellationToken` 取消 + `tokio::select!` 超时（Ctrl-C 可中断）
- [ ] 上下文管理：历史裁剪 / assistant·tool 消息配对协议

**V2**

- [ ] 会话与消息落 sqlite（复用 `my_api` 的 sqlx 层 + 迁移工作流）
- [ ] Axum 包一层：`POST /chat` + SSE / WebSocket
- [ ] 鉴权复用 `CurrentUser` → 会话按用户隔离

**V3**

- [ ] MCP
- [ ] 多后端抽象

---

## 六、通用提醒

- **对"新版库 API"的断言不可靠时**：直接 grep 本机 registry 源码（`D:\Rust\.cargo\registry\src\`），
  编译器和源码不会说谎。这条上个项目已经验证过多次。
- **时间预期**：doc §5.3 那句仍然成立 —— "核心代码也就这么一个循环"是极度乐观的说法，
  真正的活在流式、可靠性、上下文那一圈。**按两周以上算，不是"一个周末"。**
