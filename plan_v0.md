# my_agent · V0 档案（详细设计与实测记录）

> 归档自 `plan.md`（2026-10-20 拆分）。**主文件仍是 [`plan.md`](./plan.md)**：路线图、V1 之后的计划、外部资料、通用提醒。
> 本文件只装 **V0**：范围、三个类型决定、`config.rs` 五版演化、协议解剖、真实报文、安全边界、验收标准、进度清单。
>
> ⚠️ **章节号沿用原编号**（§3.1 … §3.9），所以别处笔记里的「详见 §3.5」「§3.4.4」等引用**继续有效**，只是这些节现在住在本文件。
>
> 约定不变：**代码自己写**，本文件只负责"别漏"和"别想错"，不含可复制的完整实现。

**什么时候改这里、什么时候改 `plan.md`**：V0 阶段的新发现（实测报文、类型决策、踩坑）→ 改本文件；
只有**会影响 V1+ 设计**的结论，才在 `plan.md` 里加一条摘要并链回本文件。

## 本文件目录

| 节 | 主题 |
|---|---|
| §3.1 | 范围（4 工具 · 非流式 · CLI） |
| §3.2 · 3.2.1–3.2.3 | 三个类型决定 · `impl`/`dyn`/`Box` 判据 · enum vs trait · **建模词汇缺口** |
| §3.3 · 3.3.1–3.3.2 | 实测依赖版本 · `.env` 方案 · **`config.rs` 五版演化** |
| §3.4 · 3.4.1–3.4.14 | **协议备忘**：`tool_calls` 解剖 → serde tag → 两层建模 → `tools` 注册表 → 两个 enum → 字段类型 → **args 怎么解析** → **Args struct 成本核算** → **`content` 三档严格性** → **请求/响应体缺什么** |
| §3.5 | 安全边界（scratch = 沙箱根，不是测试目录） |
| §3.6 | 验收标准（dogfooding，比的不是功能数） |
| §3.7 · 3.7.1 | 工程化分档 · 错误策略分层 |
| §3.8 | 要自答的 4 个问题 |
| §3.9 | 第一动作 + **实验 1 / 2 / 2.5 的真实报文**（`model.rs` 的唯一事实来源） |
| §五 | 进度记录（V0 清单） |

---

## 三、V0 详细

### 3.1 范围

**做**：
- 单文件 `main.rs`（约 300 行），CLI，**非流式**，**不用任何 agent 框架**
- **4 个工具**：`list_files` / `read_file` / `write_file` / `run_command`
- 核心循环：`Vec<Message>` → `POST {LLM_BASE_URL}/chat/completions` → `tool_calls` 非空则逐个执行、结果 push 回 messages → 继续；为空则输出文本、结束
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

### 3.2.2 `Message` / `Tool` 该是 **enum** 还是 **trait**？（2026-10-19，四条实测）

先说判据：**不是"简单 vs 高级"，而是集合是闭合还是开放。**

| | `Message` | `Tool` |
|---|---|---|
| 集合性质 | **闭合**——`role` 就 `system`/`user`/`assistant`/`tool` 四种，你加第五种线上对面不认 | **开放**——V1 之后工具会越加越多，理想是"注册进来就能用" |
| 要序列化成线上格式吗 | **要**（整个请求体的核心） | 不要（工具声明是静态 JSON Schema） |
| 需要穷尽匹配吗 | **要**（每个 role 的处理逻辑不同，漏了就是 bug） | 不需要 |
| 结论 | **enum** | trait（但 V0 有实打实的成本，见下） |

**四条实测证据**（临时工程已删）：

1. **enum + `#[serde(tag = "role")]` 直接产出线上格式**，不用手写 role 字段：
   ```
   {"role":"system","content":"You are helpful."}
   {"role":"assistant","content":"我来阅读这个文件。","tool_calls":[…]}
   {"role":"tool","tool_call_id":"call_19413a…","content":"## 你好"}
   ```
   `size_of::<Message>() = 56`，无堆分配、无虚表。
2. **`dyn Message` 不能直接序列化**：
   ```
   error[E0277]: the trait bound `dyn Message: serde::Serialize` is not satisfied
   note: required for `Box<dyn Message>` to implement `Serialize`
   ```
   想用 trait 就得给每个实现手写 `to_json`，或者先装进 `Value`——把类型安全原路退回。
3. **`async fn` 的 trait 不 dyn 兼容**（这条最致命，直接决定 `Box<dyn Tool>` 能不能落地）：
   ```
   error[E0038]: the trait `Tool` is not dyn compatible
   4 |     async fn run(&self, args: &str) -> String;
     |              ^^^ ...because method `run` is `async`
     = help: only type `ReadFile` implements `Tool`; consider using it directly instead.
   ```
   rustc 那句 help 很有信息量：**"只有一个实现"时它建议你根本别用 trait**。
4. **闭合集合的"穷尽"是白拿的护栏**：给 enum 加第 5 个变体 `Developer`，所有不带 `_` 兜底臂的 `match` 立刻报错：
   ```
   error[E0004]: non-exhaustive patterns: `&Message::Developer { .. }` not covered
   ```
   反过来，给 trait 加一个 `impl`，**全项目没有一处会报错**——这对 `Tool` 是优点，对 `Message` 是灾难。

**"让非法状态不可表达"（这条才是最本质的理由）**
`struct Message { role, content: Option<String>, tool_calls: Option<Vec<_>>, tool_call_id: Option<String> }`
字段组合有 **2³ = 8 种**，合法只有 4 种（`tool` 必须带 `tool_call_id` 且不该有 `tool_calls`；`user` 不该有 `tool_call_id`…）。
enum 里合法的只有列出来的那几个变体，**剩下 4 种写不出来**。这和上一轮 `Vec<ToolCall>` vs `Option<Vec<ToolCall>>`
是同一条原则的第二次出现。

**`Tool` 的三种落地写法（V0 选第一个）**

| 写法 | 成本 | 适合 |
|---|---|---|
| **`enum ToolCall`**（4 个变体，`from_wire(name, &arguments_str)` 在边界解析） | 0，且参数在编译期强制成形、未知工具在解析处就 `Err` | **V0 直接用它** ✔ |
| `trait Tool` + `Pin<Box<dyn Future<Output=…> + Send + 'a>>` | 每个 `impl` 都要重复写一遍签名，实测可编译 | V1 之后工具真变开放 |
| `trait Tool` + `#[async_trait]` | 多一个依赖，宏展开后与上一行等价 | 同上，图省字 |

⚠️ **回灌陷阱（实测）**：`#[serde(tag="role")]` 的 enum 反序列化时，**没声明的字段会被静默丢弃**——
把真实那条 assistant message 解析进只声明了 `content`/`tool_calls` 的变体，`reasoning_content` **直接消失**，
再序列化回去就没了，且**不报错**。所以"语义原样回灌"要求：要保留的字段必须写进变体
（不想输出时用 `#[serde(skip_serializing_if = "Option::is_none")]` 控制成"缺键"而不是 `null`）。
另外 `tool_calls` 仍建议 `Vec<ToolCall>` + `#[serde(default)]`（**键缺失 → 空 vec**；
⚠️ 但 `null` 吃不下，会 `invalid type: null` —— 见 §3.4.7 实测）。

### 3.2.3 建模词汇缺口：为什么总想到 struct + trait，想不起 enum（2026-10-19）

这不是"忘了语法"，是**旧语言的工具箱里没有这个工具**。

**根因：Rust 的 `enum` 不是 C++ 的 `enum class`。**

| | 形状 | 能带数据吗 |
|---|---|---|
| C++ `enum class Color { Red, Green }` | 一组命名常量 | ✘ |
| Python `enum.Enum` | 一组命名常量 | ✘（勉强用 value 塞） |
| **Rust `enum`** | **每个变体可带不同字段** | ✔ |

Rust 的 `enum` 对应的其实是 **`std::variant`** / **Python `Union[A, B, C]` + `match`**，
学名是**和类型（sum type）**：`A` **或** `B` **或** `C`，且每个分支带自己那份数据。
而 `struct` 是**积类型（product type）**：`A` **且** `B` **且** `C`。

**关键洞察：继承本来就是在模拟和类型。**
C++/Python 里表达"A 或 B"只能靠基类指针 + 子类，代价是**每个实例一次堆分配 + 一次指针跳转**。
Rust 把这个需求做成了语言原语，所以"用基类模拟 OR"的习惯在这里是纯粹的负担。

**血统（2026-10-19 查证，不是推测）**

- Rust 的**第一个编译器是 ~38,000 行 OCaml**（2006–2011，之后才自举）。`enum` / `match` / `Option` / `Result`
  直接来自 **ML 家族（OCaml / Standard ML）的 ADT**；词条原文把 "**algebraic data types, and pattern matching**"
  列为 Rust 受 FP 影响的部分。
- `trait` 是 **Haskell type class** 的直接对应物（原文：*"a mechanism similar to type classes, called 'traits',
  inspired directly by the Haskell language"*）。
- **早期 Rust 有 `obj` 关键字做显式 OOP，后来被删掉**（同时被删的还有一个 typestate 系统）
  ⇒ 这门语言**认真试过 class 那条路，最后选了 ADT + trait**。
- 时间线：ML(1973) → Miranda/Haskell(1990) → OCaml → Rust(2006)。反过来看工业界补得多晚：
  C++ `std::variant` 到 **C++17(2017)**、Python `match` 到 **3.10(2021)**、Java 至今没有原生 sum type。
  ⇒ 这个"思维缺口"不是个人问题，是 C++/Python 生态补得最晚的一块。
- 一句话记：**Rust = C++ 的内存模型（RAII / move）+ ML 的类型系统（ADT / pattern matching）+ Haskell 的抽象（type class）。**
  熟的那半是内存模型，缺的那半是类型系统。

**配套习惯也要一起搬**（ADT 不是"多一个语法"，是一套建模顺序）：
先问"**这些值有哪几种形状、每种带哪些数据、哪些组合非法**"，再写函数；而不是先问"有哪些类、谁继承谁"。
"让非法状态不可表达"这个说法的出处是 OCaml 社区的 *make illegal states unrepresentable*（Yaron Minsky, 2010）——
§3.2.2 里 `enum` 赢过 `struct + Option` 的全部理由，都是这一条的推论。

**词汇对照**

| 旧习惯（C++/Python） | Rust 里的对应物 |
|---|---|
| 基类 + 一组子类（种类有限、自己说了算） | **`enum`** |
| ABC / `Protocol`，实现由别人提供（要 mock、要换后端） | **`trait`** |
| `isinstance` / `dynamic_cast` 长长的 if-else 链 | **`match`**（穷尽性由编译器检查） |
| 虚函数表 | `dyn Trait`（动态、一次间接）或泛型（静态、单态化） |

**证据 0：你其实一直在写 enum。** `Option<T>` 和 `Result<T, E>` **就是 enum**；
`?` 是 match 糖，`unwrap()` 是 match 糖，`if let` 是单臂 match。所以"用 enum 建模"不是学新东西，
是把你天天在用的形状搬到自己的类型上。

**实测代价对比**（自定义 `GlobalAlloc` 数分配次数，代码：100 条消息）

| | 堆分配次数 | 单条大小 |
|---|---|---|
| `Vec<Box<dyn Msg>>`（OOP 风格） | **101** | 16 字节胖指针 + 堆上真身 |
| `Vec<EMsg>`（enum） | **1**（只是 Vec 的缓冲） | **8 字节内联** |

**零成本证据（niche 优化）**——`Option` 包装通常**一分钱不加**：

```
&u8 = 8            Option<&u8>        = 8
Box<u8> = 8        Option<Box<u8>>    = 8
NonZeroU32 = 4     Option<NonZeroU32> = 4
EMsg = 8           Option<EMsg>       = 8     ← 判别式还有空位可借
                   Option<u32>        = 8     ← 这个才 +4
```

**状态机是 enum 的主场**（`Step` 的四个状态各带自己的数据）：

```
size_of::<Step>()      = 32
size_of::<StepFlat>()  = 96     ← struct 把 pid/output/error 全摆平，3 个 Option 撑大了 3 倍
```
`StepFlat` 的 4 个字段有 2³ = 8 种组合，合法只有 4 种；`Step::Pending` 里**根本不存在 `output` 这个字段**。

**表达式问题：两种建模各自怕什么**

| 你要加的东西 | `enum` | `trait` |
|---|---|---|
| 加一个新**种类**（变体 / impl） | 所有 `match` 编译报错，逐个补（**编译器帮你找全**） | **免费**，一处都不用改 |
| 加一个新**操作**（方法 / 函数） | **免费**，写个新函数 + 新 `match` | 每个 `impl` 都得实现一遍 |

所以选型其实是问："**这个维度未来会往哪边长大？**" 消息种类由协议定死（往"操作"长）→ enum；
工具集合会不断扩充（往"种类"长）→ trait。

**三句话自检**

1. 这些种类**有限**、且**由我说了算**吗？（是 → enum）
2. 每种**带的字段不一样**吗？（是 → 更该 enum，因为 struct 会退化成一堆 `Option`）
3. 实现会由**别人**提供、或需要**运行时可换**吗？（是 → trait）

**本项目三个落点**：`Message`（协议定死 4 种）→ enum；`ToolCall`（V0 只 4 个自己写）→ enum；
循环里的 `Step` / `ToolOutcome`（Ok/Failed 各带不同数据）→ enum。`LlmClient` 是唯一该 trait 的。

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
LLM_BASE_URL=https://<host>/v1     # ← 线上/本地同一个形状；是否含 /v1 见约定 ①
LLM_MODEL=<model-name>
LLM_API_KEY=sk-...
```

⚠️ **两个约定要先定死，否则后面全是隐性耦合**：

1. **`LLM_BASE_URL` 是否含 `/v1`？** —— 建议**含**（示例里就带上），**代码里只往后拼 `/chat/completions`**。
   把"厂商差异"全部压进环境变量，代码只认一种形状。（否则 `base_url` 尾斜杠的有无、`/v1` 的有无，会变成三个 case 的分支。）
   ⇒ 按 **OpenAI 兼容接口**的通行形状来 —— 线上各家与本地 llama.cpp / Ollama / vLLM 基本都是这个形状；
   少数不带 `/v1` 的端点，改 `.env` 一行即可，**代码永远只拼 `/chat/completions`**。
2. **读取顺序**：`dotenvy::dotenv().ok()` —— 用 **`.ok()` 而不是 `?`**：有真实环境变量时没有 `.env` 也能跑。
   `dotenvy` 默认**不覆盖**已存在的环境变量 ⇒ 天然形成"**真实环境变量 > `.env`**"。

**建议的落地形状**：启动时把三变量读进一个 `Config` 结构体，**缺哪个就带名字报错** ——
名字要进**类型字段**（`Missing { name }`），**不是**进 `anyhow::Context` 的字符串：前者能 `match`、能断言，
后者只能打印（与 §3.7.1「错误要结构化，不要字符串」是同一件事）。
**不要拖到发请求时才 `unwrap`**。这是 V0 第一个真实的 Rust 小练习：**依赖在边界处一次性校验**。
（顺手一条：**永远不要把 key 打进日志**。）

> ⚠️ **第一件该做的事**：`.gitignore` 里**原本没有 `.env`** —— 一旦 `git add .` 就永久进历史。
> **已代为加上**（同时加了 `scratch/`）。key 泄漏是这类项目最常见的翻车方式。

### 3.3.2 `config.rs` 的五版演化与决策日志（2026-10-16）

实际写下来的过程比预想曲折，**五版的差异本身就是这一节最值钱的部分**：

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

#### ✅ 已解决：两条取值链曾未交汇（方案 C 已落地，以下是决策过程）

> 状态：**2026-10-16 已按方案 C 落地**（`from_env` 直接 `Self::from_map(&get_env)`，`get_env` 返回 `Result<String, ValueError>`）。
> 下面保留的是**当时为什么这么选**的完整推导 —— 结论看表格，过程看后面几小节。

`from_env` → `get_env` → `std::env`；`from_map` → 闭包。**两份独立实现** ⇒ **生产路径不经过可测内核**，
`from_map` 的测试全绿也**证明不了** `from_env` 对。

第 3 次迭代把这个代价变成了**具体可见的不一致**：`NotUnicode` **只有 `get_env` 能产生** ——
因为 `get_map` 的闭包边界 `Fn(&str) -> Option<String>` 已经把"为什么没有"**擦除**成了 `Option`。
⇒ **同一个非 UTF-8 环境变量走两条链得到不同变体**（`NotUnicode` vs `Missing`）；
⇒ **一旦把 `from_env` 委托给 `from_map`，`NotUnicode` 会静默失效**。

三条路，**选定 C（已落地）**：

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
✅ **已落地**：实际写成了**私有 `fn`**（比 `pub(crate)` 更严），`private_bounds` 警告随之消失。
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

#### 3.4.1 规范核对（2026-10-19 查证）

> **结论先行**：路线不用改。OpenAI 自家新项目推荐 Responses API，但**所有第三方"OpenAI 兼容"网关
> （含本项目这条 OpenCode Go 通道）实现的都是 Chat Completions**。V0 只按 Chat Completions 来。

**请求侧**

| 字段 | 取值 | 备注 |
|---|---|---|
| `tools[]` | `{"type":"function","function":{name,description,parameters,strict?}}` | `parameters` 是 **JSON Schema 对象**（不是字符串） |
| `tool_choice` | `"none"`（无 `tools` 时默认）/ `"auto"`（有 `tools` 时默认）/ `"required"` / `{"type":"function","function":{"name":"…"}}` / `{"type":"allowed_tools",…}`（新） | V0 用 `"auto"` |
| `parallel_tool_calls` | bool，**默认 `true`** | 一次可回**多个** tool_call ⇒ 回灌必须**按 id 逐个**回应 |
| `strict` | bool，**默认 false** | 开启需 `additionalProperties:false` 且**所有**属性进 `required`；第三方常不支持 |

**响应侧**（`choices[i]`）

- `finish_reason`：`"stop"` / `"length"` / `"tool_calls"` / `"content_filter"`（旧接口另有 `"function_call"`）
- `message.tool_calls[]` = `{ "id": "call_…", "type": "function", "function": { "name": …, "arguments": "…" } }`
  - **`arguments` 是 JSON 字符串**（官方文档与实测一致）⇒ Rust 侧必须二次 `from_str`
  - **`content` 可能是 `null`**（只有调用、没有正文）⇒ serde 必须 `Option<String>`，**不能裸 `String`**
  - 推理模型可能**额外**带 `reasoning_content`（本通道已实测，与 `content` 同级）
- ⚠️ **`tool_calls` 非空与 `finish_reason == "tool_calls"` 不保证同时成立**。判"要不要调工具"看
  **`message.tool_calls` 是否非空**，`finish_reason` 只当旁证 —— 兼容层实现质量参差。

**回灌侧**

```jsonc
// ① 必须先把 assistant 那条【原样】放回历史（含 tool_calls 与 id），不能只存正文
{ "role": "assistant", "content": null, "tool_calls": [ … ] }
// ② 再为每个 id 各回一条，且紧跟其后（并行调用时缺任何一条 → 400 / 行为未定义）
{ "role": "tool", "tool_call_id": "call_…", "content": "<工具输出，字符串>" }
```

- `content` 是**字符串**（新版也接受 `[{"type":"text","text":…}]` 数组）；要回对象就自己 `to_string()`
- `tool_call_id` 必须与 `id` **逐字一致**，且 id **不许自己编**（它是模型侧的关联键，不是你的编号）

**两处新版变化**（V0 用不上，知道即可）

1. `tool_choice` 新增 `{"type":"allowed_tools", "mode": …, "tools": […]}`（声明 N 个、只允许用其中 M 个）
2. `tools[].type` 新增 `"custom"` —— 自由文本入参，可选 Lark/regex 文法约束（GPT-5 系专属）

**Responses API（另一套消息模型，不是本项目的路线）**

`output[]` 里是 `{"type":"function_call","id":"fc_…","call_id":"call_…","name":…,"arguments":"…"}`，
回灌 `{"type":"function_call_output","call_id":…,"output":"…"}` —— **认 `call_id` 不是 `id`**；
推理模型还要求把 `reasoning` item 一并回放。⇒ 换路线等于**换整套消息模型**，不是改几个字段。

**已废弃**：`functions` / `function_call` 参数、`role:"function"` → 用 `tools` / `tool_calls` / `role:"tool"`。

#### 3.4.2 serde 的四种 tag 策略，以及「变体形状」的硬限制（2026-10-19 实测）

需求：`role` 不能作为变体内部的字段 → 用 `#[serde(tag = "role")]`，**不需要手写 `Serialize`**。
四种策略实测输出：

| 写法 | 输出 |
|---|---|
| 默认（externally tagged） | `{"User":{"content":"hi"}}` |
| **`#[serde(tag="role", rename_all="lowercase")]`** | **`{"role":"user","content":"hi"}`** ✅ |
| `#[serde(tag="role", content="data")]`（adjacent） | `{"role":"user","data":{"content":"hi"}}` |
| `#[serde(untagged)]` | `{"content":"hi"}` |

⇒ OpenAI 的消息格式只有 **internally tagged** 对得上：tag 被 serde 从变体里"提"出来，与字段**平铺在同一层**。

⚠️ **硬限制：newtype 变体不能 internally tagged。** `System(String)` 这种写法**编译通过、运行时报错**：
```
序列化  : cannot serialize tagged newtype variant Message::System containing a string
反序列化: invalid type: map, expected a string
```
原因：internal tag 要求**每个变体自身能表示成一个 map**（tag 要摊平进去），而 `String` 不是 map。
⇒ 必须写成 `System { content: String }`。

**什么时候才真需要手写 `Serialize`**：判据是"线上形状**不是**「变体名当 tag + 字段平铺」"。
本项目四条消息全都对得上 ⇒ 不需要。真手写时必须**同时手写 `Deserialize`**，否则往返不对称 ——
而"原样回灌"正是这个项目的硬需求。

**三个字段级坑（用真实报文往返验证过）**
1. `content` 必须 `Option<String>`：`"content":null` 真实出现过。
2. `tool_calls` 用 `Vec<ToolCall>` + `#[serde(default)]`：收尾那轮**整个键都不存在**。
3. `reasoning_content` 可保留；`skip_serializing_if = "Option::is_none"` 让 `None` 变成**缺键**而非 `null`。
   实测往返：第一轮 assistant（content + reasoning + tool_calls）逐字还原；`content:null` 保留为 `null`；
   收尾（无 `tool_calls` 键）回写也不带该键；`tool` 消息还原。
   注意 `skip_serializing_if = "Vec::is_empty"` 会让**显式传来的 `[]` 不被回写**（语义等价，但要知道）。

**未知 role 会直接解析失败**（实测）：
```
Err: unknown variant `developer`, expected `user`
```
OpenAI 后来加过 `developer` 角色（替代 `system`），网关也可能透传新值。加 `#[serde(other)] Unknown`
能兜住，但它**只能放在 unit 变体上**，且会**丢掉该条消息的全部内容**，回写变成 `{"role":"unknown"}`
—— 是"别 panic"的兜底，不是解法。**V0 建议不兜、让它报错**：读不懂的一条消息继续往下跑，比早失败更糟。

#### 3.4.3 为什么 `arguments` 是「字符串化的 JSON」，它到底伤 Rust 哪里（2026-10-19）

**先归因：这是协议的问题，不是 Rust 的问题。** 三个原因叠在一起：

1. **模型的输出是 token 流**，函数参数天然是**文本**，不是对象。
2. **流式**要求 `arguments` 能增量拼接——delta 片段是 `"{\"pa"`、`"th\":\"src"` 这种**半截 JSON**，
   任何解析器都会失败。⇒ 协议层只能把它当**不透明字符串**，客户端拼完再解析。这是结构性要求，不是疏忽。
3. 服务端因此**不承担校验责任**（除非 `strict`），坏 JSON 原样传给你 —— 校验被推给了客户端。

**伤 Rust 的地方只有一处，可以精确定位**：serde derive 有个隐含前提——**内容是"结构"，不能是"字符串化的结构"**。
想一步到位用 `#[serde(tag = "name", content = "arguments", rename_all = "snake_case")]` 建模，实测两头都错：

```
序列化   → {"name":"read_file","arguments":{"path":"scratch/readme.md"}}
          ↑ arguments 变成对象（协议要字符串），且丢了 `function` 这层包装
反序列化 → invalid type: string "{\"path\":…}", expected struct variant Args::ReadFile
```

⇒ **derive 无解，只能手写一层边界**。但代价是**一次性、且能隔离在一个点上**：
外层 `RawCall` / `RawFn` 原样收（`arguments: String`），再用一个 `from_wire(name, &arguments)`
match 出带类型的参数。之后全项目都是类型化的，没有任何地方再碰字符串。

**Rust 在这里其实占便宜**：模型生成的 JSON **真的会坏**，实测三种失败都有清晰错误：

```
{"path":      -> EOF while parsing a value          （被截断）
{}            -> missing field `path`               （漏字段）
{"path":1}    -> invalid type: integer, expected a string
```

Python 是 `json.loads` 抛异常、TS 是 `JSON.parse` 抛异常并给回 `any`——同样要处理，只是**不逼你处理**。
Rust 把它变成 `Result`，不 match 就编译不过，正好接上"agent 循环里绝不可 panic"（§3.7.1）。

**四条实操**

- **别解析后重写**：`arguments` 保留原始 `String`，只在执行前解析一次（回灌保真 + 不反复解析）。
- 无参工具的 `arguments` 是 `"{}"`，不是 `null`。
- 少数兼容层会把 `arguments` 直接给成**对象**（不是字符串）。V0 按字符串写，真撞上再用
  `#[serde(untagged)] enum Arguments { Str(String), Obj(Value) }` 兜，别提前设计。
- 两段解析的**第二段返回值必须能报告"为什么坏"**（未知工具 / 截断 / 类型不对），否则模型看不到失败就无法自纠。

#### 3.4.4 `ToolCall` 该是 struct 还是 enum：**两层建模**（2026-10-19 实测）

直觉是对的：`id` / `type` 是协议外壳，`function` 才是"真正被调用的工具"。但结论不是二选一，
而是**传输层用 struct、执行层用 enum**（DTO vs domain model）：

| 层 | 类型 | 必须能做什么 |
|---|---|---|
| **传输层（wire / DTO）** | `struct ToolCall { id, kind, function: Function }`，`Function { name: String, arguments: String }` | 任何 name / 任何 arguments **都收得下**；回写**逐字一致** |
| **执行层（domain）** | `enum Invocation { ReadFile { path }, RunCommand { cmd }, … }` | 只在这一层拒绝坏输入，且**说清为什么坏** |

**① 嵌套不能摊平（实测）**：`#[serde(flatten)]` 是"内联"，**不是"包一层"**：

```
① 手写摊平  {"id":"call_1","name":"read_file","arguments":"…"}
② flatten   {"id":"call_1","name":"read_file","arguments":"…"}   ← 与①逐字相同，function 层没了
③ 保留嵌套  {"id":"call_1","type":"function","function":{"name":"read_file","arguments":"…"}} ✅
```

**② 两层分工实测**：wire 层往返**逐字一致**（`to_string(&tc) == wire` 为 `true`），坏东西在执行层被拒：

```
正常     -> Ok(ReadFile { path: "a" })
未知工具 -> Err("unknown tool: rm_rf")
参数截断 -> Err("read_file 参数不合法: EOF while parsing a value")
漏字段   -> Err("read_file 参数不合法: missing field `path`")
```

**③ 为什么不能把整体 enum 化（决定性理由）**：模型幻觉出一个不存在的工具名时——

```
整条 tool_call 解析失败: unknown variant `rm_rf`, expected `read_file`
→ 连 `id` 都拿不到
```

拿不到 `id` 就**发不出** `{"role":"tool","tool_call_id":…}`，这一轮只能整体报废；
而保留字符串 `name` 时，你可以回一句 `unknown tool: rm_rf`，**模型下一轮自己会改**。
⇒ **在 agent 里"能报告错误"比"编译期更严格"更值钱。** 这条对 `Message` 不成立（role 错了没法救，
且 role 不是我方生成的），但对**工具名**成立（是模型生成的，可自纠）。

**④ 为什么 `id` 不放进 enum 变体**：它对所有变体完全一样 —— 变体应当**互不相同**，
共有字段属于外层。放进每个变体等于把同一个值抄四遍，且每个 match 臂都要重新抽出来。

**⑤ 判决**
- `ToolCall` / `Function` = **struct**（`arguments: String` 原样保留；`Function` 别试图 derive 成 enum，§3.4.3 已证不行）
- `enum Invocation`（或 `ToolArgs`）= 由 `to_invocation(&ToolCall) -> Result<Invocation, ToolError>` 产出，
  **只在执行那一刻存在，绝不进历史**
- `type` 是 Rust 关键字 ⇒ `#[serde(rename = "type", default = "…")] kind: String`，
  给它默认值以防万一（但我们回写时永远发 `"function"`）

#### 3.4.5 请求侧的 `function` ≠ 响应侧的 `function`：同名不同物，必须两个类型（2026-10-20 实测）

**问题**：请求 `tools[].function` 与响应 `tool_calls[].function` 撞名，但字段完全不同：

```jsonc
// 请求侧（我方发出去的「工具声明」）
"function": { "name": "read_file", "description": "…",
              "parameters": { /* JSON Schema 对象 */ }, "strict": true }

// 响应侧（模型生成的「工具调用」）
"function": { "name": "read_file", "arguments": "{\"path\":\"x\"}" }   // ← 字符串
```

| | 请求侧 `function` | 响应侧 `function` |
|---|---|---|
| 语义 | 工具的**声明 / 契约**（参数**应该**是什么） | 工具的**调用**（参数**实际**是什么） |
| 谁生成 | 我方，**编译期就定死** | 模型，运行时生成 |
| 出现频次 | 每次请求**原样全量**重发（4 个工具全带） | 只有被调用的那几个（0..n） |
| 载荷 | `parameters` = JSON Schema（**schema**） | `arguments` = JSON 字符串（**实例**） |
| 有 `strict` 吗 | ✔（约束解码） | ✘ |

⇒ **`Function` 这个名字必须拆成两个**：响应侧 `FunctionCall`（`name` + `arguments`），
请求侧 `FunctionDecl`（`name` + `description` + `parameters` + `strict`）。
**同名不同物是 bug 温床**，而 serde 不会告警 —— 字段名不重叠，两个 struct 各解析各的，
出错只发生在「发出去少了一半字段」的那一刻。

**① 实测：复用响应侧 `Function { name, arguments }` 去发请求**

```rust
struct ToolDecl { kind: String, function: Function }   // 复用 { name, arguments }
```
```
输出: {"function":{"arguments":"{\"path\":\"x\"}","name":"read_file"},"type":"function"}
     ↑ 没有 description / parameters / strict —— 服务端必然 400（缺 parameters）
```
反向也失败（拿请求侧对象喂响应侧 struct）：
```
解析失败: missing field `arguments`
```
⇒ 两者**字段集不相交**（只共享 `name`），共用一个类型就是两头都不对。

**② 真正的决策点：请求侧到底要不要建模？**

关键判据不是「在请求里还是在响应里」，而是 **这份数据是常量还是变量**：

| 数据 | 常量 / 变量 | 建模收益 |
|---|---|---|
| `tools` 声明 | **常量**（写死的 4 个工具） | 见下 —— 收益不在"防打错"，在**和派发同源** |
| `messages` 历史 | **变量**（每轮增长、4 种形状） | 高：字段多、每轮变、错了难定位 |
| `tool_results` | 变量 | 高：要按 `tool_call_id` 关联 |

> ⚠️ **这一格最初写成"收益只有防手打字段名"，理由是错的**（2026-10-20 用户反驳后实测更正）。
> `tools` 在这张表里的**性质确实是常量**，但"常量"只说明它不随时间变化，**不说明它可以用字符串写死**。
> 真正的问题是**同一个事实被写了两遍**：声明里的 `"read_file"` 和派发 `match` 里的 `"read_file"`。
> 详见 **§3.4.8**（实测：漏一个臂编译零告警，模型真去调才炸）。
> 结论已改为：**`FunctionDecl` struct + `ToolKind::ALL` + `spec()`，`tools` 由序列化生成**。

~~它就是一段常量 JSON~~（旧建议，已撤回）：

```rust
// ✗ 不推荐：与 dispatch 的 match 构成双写
const TOOLS_JSON: &str = r#"[{"type":"function","function":{…}}]"#;
```

**③ 实测：const 版与 struct 版的产物逐字相同**

```
写法1 const   : {"model":"m","tools":[{"function":{"description":"…","name":"read_file","parameters":{…},"strict":true},"type":"function"}]}
写法2 struct  : {"model":"m","tools":[{"function":{"description":"…","name":"read_file","parameters":{…},"strict":true},"type":"function"}]}
   两者字节相同: true
```
所以 **struct 化换不来任何线上差异**，只换来「编译期查字段名」。要不要这个，取决于你信不信自己会拼错 ——
而这 6 个字段（`type` / `function` / `name` / `description` / `parameters` / `strict`）撞一次就 400，
反馈延迟约 1 秒，比编译更快。**这是唯一一处「错了也能立刻知道」的地方，所以常量字符串的代价极低。**

**④ 若坚持建模，`parameters` 必须留 `Value`**

```rust
struct FunctionDecl {
    name: &'static str,          // ← §3.3.2 那条：&'static 是「这是常量」的类型级证明
    description: &'static str,
    parameters: serde_json::Value,   // ← 别给 JSON Schema 建模
    strict: bool,
}
```

**为什么不能给 JSON Schema 建模**：它是**开放的递归集合**。实测写一个「看起来够用」的 enum：

```rust
enum Schema {
    Object { properties: HashMap<String, Schema>, required: Vec<String>, additional_properties: bool },
    Array { items: Box<Schema> },   // ← 递归，必须 Box
    String, Integer, Number, Boolean,
}
```
编译过，也输出对了：
```
① 建模后输出: {"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}
```
但 JSON Schema 关键字有几十个且还在增长（`$ref` / `$defs` / `oneOf` / `anyOf` / `allOf` / `const` /
`patternProperties` / `minLength` / `maximum` / `format` / `default` / `examples` / `$comment` …）。
三种**都合法且语义相同**的现实中写法的：
```
{"type":"object","properties":{…},"required":["path"]}
{"type":"object","properties":{…},"required":["path"],"additionalProperties":false}
{"type":"object","properties":{…},"required":["path"],"additionalProperties":false,"$comment":"x"}
```
**建模后第三种里 `$comment` 无处安放** —— 要么静默丢弃（改写了原意），要么报错（拒绝合法输入）。
而客户端**从不解释 `parameters`**，只是透传给服务端。⇒ **给一个只搬运、不理解的载荷建模，纯属自伤。**

**⑤ 反例：把两边字段全塞进一个 struct**

```rust
struct FunctionAll { name, description, parameters, strict, arguments }  // 全部 Option
```
实测输出：
```
[{"function":{"arguments":null,"description":"x","name":"read_file","parameters":{"type":"object"},"strict":true},"type":"function"}]
              ↑ arguments:null 混进请求侧声明
```
这就是 §3.4.4 那条「让非法状态不可表达」的反向案例：**6 个 `Option` 能表达 64 种组合，合法只有 2 种。**

**⑥ 顺带一个 strict 的硬要求（实测审计规则）**

`strict: true` 时 OpenAI 要求：`additionalProperties: false` **且每个 property 都必须在 `required` 里**。

```
合规            : []
漏 additionalProperties : ["strict 要求 additionalProperties: false"]
有可选参数      : ["strict 要求每个 property 都进 required，缺: limit"]
```

⇒ **strict 下不能有「可选参数」**。这对 V0 的 4 个工具是个实际约束：
`read_file(path)` 没问题，但 `run_command(cmd, timeout?)` 这种就得把 `timeout` 改成必填（给默认值语义），
或者退回 `strict: false`（放弃约束解码，参数合法性全靠自己在执行层校验 —— 反正 `to_invocation` 本来就要校验）。

**⑦ 双写漂移：真正的长期风险**

`enum Invocation::ReadFile { path: String }` 和 tools 声明里的 `{"path":{"type":"string"}}`
是**同一个事实的两次书写**。改一处忘另一处 → 模型按旧 schema 传参，`to_invocation` 拒绝，
模型收到错误再改 —— 能自愈，但白烧一两轮 token。

- 现状（4 个工具、都自己写）：**接受双写**，靠一条测试钉住（见下）。
- 想消除漂移：`schemars 1.2.3`（MIT，5.6 亿下载）能从 Rust 类型 derive 出 JSON Schema，
  配合 `serde(rename_all)` 得到和 `Invocation` 同源的 schema。
  ⚠️ **1.0 是大改**，网上教程多是 0.8 写法（`#[derive(JsonSchema)]` 的 trait 名没变，但
  `schema_for!` 返回类型改成了 `Schema`）。**V0 不引** —— 多一个依赖、`Value` 还得再转一次，
  换来的是 4 个手写 schema 的一致性。等工具超过 8 个再说。

**⑧ 判决**
- 响应侧：`ToolCall` / `FunctionCall { name, arguments }` = **struct**（已写，保留）
- 请求侧：**`FunctionDecl { name, description, parameters: Value, strict }` + `ToolKind::ALL` + `spec()`**，
  `tools` 用序列化生成（**§3.4.8 已改推荐**；`const TOOLS_JSON` 方案撤回）
- **两处绝不共用一个类型**（响应侧 `FunctionCall` 只有 `name`+`arguments`，请求侧 `FunctionDecl` 有
  `description`/`parameters`/`strict`）；名字上也别都叫 `Function`

**⑨ 附带一个 `model.rs` 的隐患：`Invocation` 不该 derive serde（2026-10-20 实测）**

当前 `model.rs` 写的是 `#[derive(Debug, Serialize, Deserialize)] enum Invocation { ReadFile { path } }`。
看起来「顺便就能解析 `arguments`」——**实际两头都不对**：

```
① serde_json::from_str::<Invocation>(r#"{"path":"scratch/readme.md"}"#)
   → Err: unknown variant `path`, expected `ReadFile` at line 1 column 7
   （默认 externally tagged 期待 {"ReadFile":{"path":"…"}}，而 arguments 里没有变体名）

② Invocation 序列化出来 → {"ReadFile":{"path":"scratch/readme.md"}}
   跟线报文 {"name":"read_file","arguments":"{\"path\":…}"} 毫无关系
```

两个原因叠在一起：
- **变体名不在 `arguments` 里**，它在 `function.name` 里（那是另一层的字段）⇒ 必须先按 `name` 分支，再解析参数
- **`arguments` 是字符串化的 JSON**（§3.4.3）⇒ 无论如何都要走两次解析

⇒ 正确形状是「**先选变体，再解析参数**」，每个工具配一个纯参数 struct：

```rust
#[derive(Deserialize)] struct ReadFileArgs { path: String }

match fn_call.name.as_str() {
    "read_file" => {
        let a: ReadFileArgs = serde_json::from_str(&fn_call.arguments)
            .map_err(|e| ToolError::BadArgs { tool: "read_file", source: e.to_string() })?;
        Ok(Invocation::ReadFile { path: a.path })
    }
    other => Err(ToolError::UnknownTool { name: other.to_string() }),
}
```
实测第 ② 步通过：`Ok(ReadFile { path: "scratch/readme.md" })`。

**所以 `Invocation` 的 derive 应该是 `#[derive(Debug)]` 就够**（加 `Clone/PartialEq` 看测试需要）。
derive 了 `Serialize` 等于给它发了一张「可以进历史」的通行证 —— 而 §3.4.4 的结论恰恰是
**它绝不进历史**。**类型的能力应该和它的职责一致**：能序列化，就一定会有人把它塞进 `messages`。

#### 3.4.6 `type: "function"` 不是装饰：它是**判别标签**，而且真有第二个取值（2026-10-20 查证 + 实测）

**先回答问题**：`type` 看着像"还有别的类型"，是因为它**本来就是** discriminated union 的 tag ——
和 `Message.role` 完全同构。协议 2023-11 引入 `tools` / `tool_calls` 时就留了这个口，
**2025 年 GPT-5 系真的把第二个取值 `"custom"` 用上了**。

**① `tools[].type` 的两个取值，而且形状不同**（这是最关键的一点）

```jsonc
// type = "function"：载荷包在 function 里（嵌套）
{ "type": "function",
  "function": { "name": "read_file", "description": "…",
                "parameters": { /* JSON Schema */ }, "strict": true } }

// type = "custom"：name/description 直接平铺在 tools[] 层级（扁平），没有 function 那层
{ "type": "custom", "name": "code_exec", "description": "Executes arbitrary Python code.",
  "format": { "type": "grammar", "syntax": "lark", "definition": "start: /.+/s" } }
```

⇒ **`type` 不只决定"叫什么"，它决定整个结构的形状**（嵌套 vs 扁平）。
所以它不能只当一个被忽略的字符串常量 —— 它是**形状选择器**。

`custom` 存在的意义：让模型把**任意字符串**当参数（省掉 JSON 包装），可选用 Lark / regex 文法约束输出。

**② `tool_calls[].type` 也是两个取值，且载荷键跟着变**

```jsonc
{ "id": "call_1", "type": "function",
  "function": { "name": "read_file", "arguments": "{\"path\":\"a\"}" } }   // ← arguments 是 JSON 字符串

{ "id": "call_2", "type": "custom",
  "custom": { "name": "code_exec", "input": "print(1)" } }                 // ← input 是自由文本
```

注意 **`custom` 键替代了 `function` 键**，`input` 替代了 `arguments`，而且 `input` **不是** JSON 字符串
（它就是模型吐的原始文本，可能是一段 Python）⇒ **第二层"解析"这一步根本不需要**。

（Responses API 里形状又不同：`{"type":"custom_tool_call","call_id":…,"input":…,"name":…}`，
`call_id` 而非 `id`。V0 不碰。）

**③ 实测：四种 Rust 建模方案 × 三种输入**

| 输入 | T1 `kind:String` + `function` **必填** | T2 整条 `#[serde(tag="type")]` enum | T3 `kind:String` + **两个 `Option`** | T4 **单变体** enum |
|---|---|---|---|---|
| `function` | Ok | Ok | Ok | Ok |
| `custom` | **Err: missing field `function`** ← 误导 | Ok(Custom) | Ok(`custom: Some`) | **Err: unknown variant `custom`** |
| 未知 `computer_use` | **Err: missing field `function`** ← 误导 | Err: unknown variant `computer_use` | **Ok，`id` 保住了** | Err: unknown variant |

原文（未知 type 那行最关键）：

```
T1 结构体必填 : Err(missing field `function` at line 1 column 56)
T2 tag enum   : Err(unknown variant `computer_use`, expected `function` or `custom` at line 1 column 36)
T3 两 Option  : Ok(T3 { id: "call_3", kind: "computer_use", function: None, custom: None })
T4 单变体enum : Err(unknown variant `computer_use`, expected `function` at line 1 column 36)
```

三条读出：

- **T1 的报错是误导性的**：真因是「type 不认识」，它却说「缺 function」。这是"用必填 struct 冒充 union 单支"的通病 ——
  serde 先找不到 `function` 键就报缺字段，压根没机会告诉你 `type` 不对。
- **T2 严格但丢 `id`**：跟 §3.4.4 那条一样 —— 拿不到 `id` 就发不出 `role:"tool"` 回灌，这轮只能报废。
- **T3 是唯一能在未知 type 时保住 `id` 的**，代价是两个 `Option`（4 种组合，合法 2 种）。
- **T4 单变体 enum 是最差的选择**：服务端加一个取值，整条立刻炸，而 `type` 恰恰是**变化最频繁**的字段。

**④ 判决：`kind: String` 现在是对的，`function` 那个必填键建议改成 `Option`**

V0 只声明 `function` 工具 ⇒ 服务端**不会**回 `custom`。所以 T1（现状）能跑。
但它有两个隐患，改造成本只有**一个 `Option`**：

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,                 // ← 保持 String，不做单变体 enum（见 T4）
    #[serde(default)]
    pub function: Option<FunctionCall>,
    #[serde(default)]
    pub custom: Option<CustomCall>,   // { name: String, input: String }
}
```
再加一个收敛方法，把"两 Option"的非法态挡在执行层之前：
```rust
pub fn payload(&self) -> Result<&FunctionCall, ToolError> {
    self.function.as_ref().ok_or_else(|| ToolError::UnsupportedToolType { kind: self.kind.clone() })
}
```
这样未知 type 时得到的是 `UnsupportedToolType { kind: "computer_use" }` ——
**一句能贴给模型看的话**，而不是 "missing field `function`"。

**⑤ 这个协议里 `type` 到处都是（同一模式的重复出现）**

| 位置 | `type` 的取值 | 作用 |
|---|---|---|
| `messages[].role` | `system` / `user` / `assistant` / `tool` / `developer` | 同上，只是键名叫 role |
| `tools[]` | `function` / `custom` | 形状选择器（嵌套 vs 扁平） |
| `tool_calls[]` | `function` / `custom` | 载荷键跟着变 |
| `tool_choice` | `function` / `allowed_tools` / `custom` | 选择策略 |
| `content`（数组形式） | `text` / `image_url` / `input_audio` | 多模态 |
| `response_format` | `text` / `json_object` / `json_schema` | 强约束输出 |
| `format`（custom 工具内） | `grammar`（`syntax`: `lark` / `regex`） | 文法约束 |
| Responses API `output[]` | `message` / `function_call` / `reasoning` / `custom_tool_call` | item 类型 |

⇒ **整个协议就是一棵 tagged union 的树**。所以 §3.2.2 那条"闭合集合用 enum"不是巧合 ——
OpenAI 的 schema 本身就是用和类型组织的，Rust 的 `enum` 恰好是它的直译。
**你写 `#[serde(tag = "role")]` 那一步，其实是在做协议的忠实映射。**

**⑥ 通道提醒**

`type: "custom"` **只有 GPT-5 及以后的模型支持**，且部分网关会把它改写坏掉。
你走的是 OpenCode Go + `deepseek-v4.1-flash`，**V0 一律按 `function` 写**，别去试 custom。

#### 3.4.7 `#[serde(default)]` 到底管什么：**只管"键不存在"，不管 `null`**（2026-10-20 实测）

**先纠正一条之前写错的话**：`Vec<T>` + `#[serde(default)]` **不能**把 `null` 也归成空 vec。
它只处理"键根本没有"，遇到 `"key": null` 照样报错。实测（`{"x":null}`）：

```
D  Vec<String> + default : Err(invalid type: null, expected a sequence)
```

**① 三个机制各管一件事（这是全部混乱的来源）**

| 你想要的效果 | 该用的工具 | 谁负责 |
|---|---|---|
| 键**不存在**时给个默认值 | `#[serde(default)]` | serde derive |
| 值是 **`null`** 时映射成 `None` | **字段类型写成 `Option<T>`** | serde 对 `Option` 的内建特判 |
| 值是 `null` 时要变成 `[]` / `""` / `0` | `deserialize_with = "…"` 手写 | 你自己 |
| 值存在但**类型错** | 什么都不管用，就该报错 | —— |

**② `Option<T>` 缺键本来就不报错 —— `default` 在它身上是多余的**

这是最容易搞错的一条。serde derive 生成的代码对缺失字段调用 `missing_field::<T>()`，
而它的内部 deserializer 对 `deserialize_option` 直接 `visit_none()`。实测：

| 输入 | `Option<String>` 无 default | `Option<String>` + default |
|---|---|---|
| `{}`（缺键） | **`Ok(None)`** | `Ok(None)` |
| `{"x":null}` | `Ok(None)` | `Ok(None)` |
| `{"x":"hi"}` | `Ok(Some("hi"))` | `Ok(Some("hi"))` |
| `{"x":123}` | `Err(invalid type: integer)` | 同左 |

两者**逐行相同** ⇒ `#[serde(default)]` 加在 `Option<T>` 上纯属冗余
（留着只是自文档："这个字段允许缺席"，不算错）。

三种场景都验证过，行为一致：普通 struct、`#[serde(tag = "role")]` 的 internally tagged enum 变体、
以及 `#[serde(flatten)]` 内部（flatten 缺键**不加 default 也是 None**）。

**⇒ 用你真实的 `model.rs` 结构跑真报文，全部通过**：

```
① 有 content + reasoning + tool_calls : Ok(Assistant{…tool_calls:[1 条]})
② 收尾轮（无 tool_calls 键）          : Ok(Assistant{…tool_calls: [] })
③ 连 reasoning_content 都没有          : Ok  ← Option 缺键自动 None
④ 只有 tool_calls、content 键缺失      : Ok(Assistant{content: None, …})
⑤ tool_calls: null                     : Err(invalid type: null, expected a sequence)
```

**所以你的 `reasoning_content: Option<String>` 不加 `default` 是安全的**（第 ③ 行），
而 `tool_calls` 那个 `default` 是**必需的**（第 ② 行：不加会 `missing field`）。

**③ 非要吃 `null` 时，两种写法**

```rust
// 写法 A：手写 deserialize_with（推荐的通用解）
fn null_to_default<'de, D, T>(d: D) -> Result<T, D::Error>
where D: serde::Deserializer<'de>, T: Default + serde::Deserialize<'de> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}
#[serde(default, deserialize_with = "null_to_default")]
tool_calls: Vec<ToolCall>,

// 写法 B：套一层 Option（但会把 2 态变回 3 态，和"非法状态不可表达"冲突）
#[serde(default)]
tool_calls: Option<Vec<ToolCall>>,
```
实测 A：`{}` → `[]`、`null` → `[]`、`["a"]` → `["a"]`，三种输入全过。

**V0 要不要做？不要。** 契约很明确：存在就是数组，`[]` 也表示空。服务端真发 `null` 是它违约，
应该让解析器喊出来（`invalid type: null` 这句话本身就足够定位问题）。

**④ 还有两种写法：struct 级 / 自定义函数**

```rust
#[derive(Debug, Default, Deserialize)]
#[serde(default)]                       // ← 放在 struct 上：所有字段都享受，要求实现 Default
struct Opts { timeout: u32, retries: u32 }

#[serde(default = "dflt_timeout")]      // ← 指定函数，适合默认值不是 Default 的情况
timeout: u32,
fn dflt_timeout() -> u32 { 30 }
```
实测：struct 级 `#[serde(default)]` + `{}` → `Opts { timeout: 0, retries: 0 }`；
`default = "dflt_timeout"` + `{}` → `30`，给了值就用给的值。

**⑤ `default` 只管输入，输出要靠 `skip_serializing_if`（两个方向，别混）**

```rust
#[serde(default)]                                   x: Vec<String>   // 输出 {"x":[]}
#[serde(default, skip_serializing_if = "Vec::is_empty")] x: Vec<String>  // 输出 {"y":"v"}
```
```
只有 default            : {"x":[],"y":"v"}
default + skip_if_empty : {"y":"v"}
```
⇒ **`default` = 输入宽容，`skip_serializing_if` = 输出省略**。它们经常成对出现，
但完全不互相影响 —— 你 `model.rs` 里 `tool_calls` 两个都写了，是对的。

**⑥ 什么时候「不该」加 `default`（这条比"怎么加"更重要）**

`#[serde(default)]` 的本质是：**把"对方忘了发"变成"它确实想表示空"**。
这两件事在协议里语义完全不同：

- `tool_calls` 缺键 = 模型没调工具（真实语义）→ 该加
- `id` 缺键 = 报文残缺 → **绝不加**（加了会得到一个空字符串 id，然后拿去发 `tool_call_id: ""`，得到 400）
- `function.arguments` 缺键 = 报文残缺 → **绝不加**（空字符串 `""` 会让"二次解析"报一个奇怪的 EOF 错误，掩盖真因）

**判据**：**这个字段的"空"是一个有意义的业务状态吗？** 是 → 加 `default`；不是 → 让它报错。

**⑦ 一个做不到的事**：`Option<Option<T>>` 区分不了"缺键"和"显式 `null`"

```
{}          -> None
{"x":null}  -> None        ← 想区分这两个？做不到
{"x":"hi"}  -> Some(Some("hi"))
```
真要区分（例如 PATCH 语义里"不传 = 不改"、"传 null = 清空"），得给外层也套
`deserialize_with` + `Option::deserialize` 的 `Some` 包装技巧。**本项目的协议不需要，别提前做。**

#### 3.4.8 `tools` 必须只有一个来源：常量字符串 → 类型生成 → 注册表（2026-10-20 实测）

**起因**：上一节最初推荐 `const TOOLS_JSON: &str`，理由是"错了 1 秒内 400"。用户反驳：
「tool 本身应该有一个统一结构，最终是靠序列化把所有 tool 合成一个 JSON 文本，直接写死是不是奇怪？
而且学 agent 时提到 tool 是可以管理、自己注册新 tool 的。」—— **这个反驳成立，原推荐撤回。**

原因：**"打错后多久得到反馈"是反馈速度，"同一事实写了几遍"是结构正确性。** 我用前者替代了后者。

##### 实测 ① 常量 + `match` 派的漂移是**静默**的

4 个工具写进常量，`dispatch` 里只写了 3 个臂（漏 `run_command`）：

```
advertised=read_file    dispatch=Ok("read")
advertised=list_files   dispatch=Ok("list")
advertised=write_file   dispatch=Ok("write")
advertised=run_command  dispatch=Err("unknown tool: run_command")

注意：dispatch 里少了 run_command，但编译完全通过。
裂口 = 1 个 → 只有模型真去调它时才会暴露
```

**为什么编译器帮不上**：`match name` 的 `name` 是 `&str`，字符串匹配**永远不可能穷尽**，
所以 `_ =>` 兜底臂是语法上必需的。有了兜底臂，漏掉任何一个具名臂都不会报错。
⇒ **"声明"和"派发"是两份独立的手写数据，编译器看不见它们的关系。**

##### 实测 ② 把声明交给类型，JSON 由序列化产生

统一结构（就是 `model.rs` 里那个还空着的 `FunctionDecl`）+ enum 当集合：

```rust
#[derive(Serialize)]
struct FunctionDecl { name: &'static str, description: &'static str,
                      parameters: Value, strict: bool }

impl ToolKind {
    const ALL: &'static [ToolKind] = &[ReadFile, ListFiles, WriteFile, RunCommand];
    fn name(self) -> &'static str { match self { … } }        // 名字唯一来源
    fn spec(self) -> FunctionDecl { match self { … } }        // 声明由类型产出
    fn from_name(n: &str) -> Option<ToolKind> { … }           // 派发入口，查同一个 ALL
}
fn tools_json() -> Value { to_value(ALL.iter().map(|t| WireTool { kind: "function", function: t.spec() })) }
```

结果：

```
语义相等（与手写常量相比）: true
read_file    -> Some(ReadFile)
rm_rf        -> None
```

⇒ **线报文一个字没变**（所以这次改造是纯重构，不是协议改动），但**名字、描述、schema、派发入口
现在都挂在同一个 enum 上**。加工具时的动作从"改字符串 + 改 match + 祈祷两处一致"变成"加一个变体"。

⚠️ 一个可观察差异：生成的键序是 `serde_json::Map`（默认 `BTreeMap`）的**字典序**
（`description` 排在 `name` 前面），与手写顺序不同。JSON 对象键序无语义，服务端解析后按
chat template 重渲染，**不影响 prompt cache**（§3.9 已更正过"cache 看语义不看字节"）。

##### 实测 ③ enum 注册表也有最后一厘米，而且编译器抓不到

加一个 `DeleteFile` 变体、忘了写进 `ALL`：

```
error[E0004]: non-exhaustive patterns: `ToolKind::DeleteFile` not covered
  --> src\bin\c_gap.rs:24:15    （name() 的 match）
help: ensure that all possible cases are being handled…
```

- ✔ `name()` / `spec()` 的 `match` 有**穷尽检查** → 加变体必报错，强制你实现
- ✘ `const ALL` 是**手写数组** → 变体没进去，编译器一句不说（报的错来自 `name()`，不是 `ALL`）

这个缝的三种补法：

| 补法 | 依赖 | 说明 |
|---|---|---|
| `strum 0.28.0` 的 `EnumIter`（MIT，7 亿下载） | +1 | `ToolKind::iter()` 自动包含新变体，缝彻底消失 |
| 手写 `macro_rules!`（一份清单生成 enum + ALL + name） | 0 | 零依赖，但要多读一层宏 |
| **一条测试钉住** | 0 | `assert_eq!(ALL.len(), 4)` + 名字唯一 —— **V0 选这个** |

**V0 建议**：不引 strum、不写宏，用一条测试钉。理由是 V1 换成 `trait Tool` 注册表后，
**这个缝会自然消失**（`Vec` 本身就是 `ALL`），现在为它引入宏或依赖是给即将被替换的代码加复杂度。

##### 实测 ④ `Vec<Box<dyn Tool>>` 才是"可管理的注册表"

```rust
trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn spec(&self) -> Value;                                   // ← 同步，dyn 完全没问题
    fn run<'a>(&'a self, args: &'a str)                        // ← async 才要装箱
        -> Pin<Box<dyn Future<Output = String> + Send + 'a>>;
}
```

实跑：

```
注册后对外声明: [read_file, run_command, echo]        ← 三个 spec() 直接生成
只读模式只剩: ["read_file", "echo"]                    ← tools.retain(...)
run_command 已被过滤掉 —— 模型若仍调它，可以回一句「该工具在当前模式下不可用」
run 返回: Pin<Box<dyn Future<Output = String> + Send>>  ← 不需要 runtime 就能构造
```

三条要点：

1. **注册 = `tools.push(Box::new(X))`**，内核一行不改（这正合"工具可管理"的直觉）
2. **运行时按能力过滤**是注册表最实用的能力（只读模式、按用户权限、按 step 预算裁剪）
3. **请求侧的声明完全不受 `async` 影响**（`spec()` 是同步的）。E0038 那个坑（§3.2.2）
   只在**执行**侧。⇒ **"声明走注册表、派发走 enum"是完全可行的混合方案**，不是二选一

##### "注册"这个词要拆成三级，否则容易许愿

| 层级 | 机制 | 新工具何时出现 | 成本 |
|---|---|---|---|
| 编译期·集中 | `enum` + `ALL` | 重编译 | 0 |
| 编译期·分散 | `inventory 0.3.25`（dtolnay，1.4 亿下载）/ `linkme` | 重编译，但**不用改中心列表**（每个工具文件自己登记） | +1 dep + 链接期机制 |
| 运行时 | **MCP server 的 `tools/list`** / WASM 插件 / dylib | **不重编译** | 一个协议客户端 |

关键区分：**`Vec<Box<dyn Tool>>` 是"可增删的容器"，不是"可加载新代码"。** 每个 `impl Tool`
仍然躺在编译期。它能做到"管理"（增删、过滤、排序），做不到"注册新实现"。

##### 你记得的"自己注册新 tool"在工业界是协议层，不是语言层

MCP（Model Context Protocol）规范原文：

- `tools/list` → 返回 `{ name, description, inputSchema }` 清单（就是我们的 `FunctionDecl`）
- `tools/call` → `{ name, arguments }`（`arguments` 在这里是**对象**，不是字符串 —— 因为不流式）
- `capabilities.tools.listChanged: true` + `notifications/tools/list_changed` → **清单可以变，服务端主动通知**
- 原文还有一句：`The set MAY vary by the authorization presented on the request — for example,
  returning only the tools the caller's granted scopes permit`
  ⇒ 这跟实测 ④ 里的 `retain(只读)` 是同一个语义，只是搬到了协议上
- 另外一条可直接用的硬要求：`Servers SHOULD return tools in a **deterministic order** …
  improves **LLM prompt cache hit rates** when tools are included in model context`
  ⇒ **`tools` 数组必须顺序稳定**（别用 `HashMap` 迭代生成），否则打掉前缀 cache

所以："工具能管理、能注册"是**协议能力**（MCP / 插件系统），不是 Rust 内部一个 `Vec` 的事。
V3 接 MCP 时，`ToolDecl` 就来自 `tools/list` 的响应 —— 那时注册表里的元素才真正是**运行时来的**。

##### 落到本项目

| 阶段 | tools 声明来源 | 派发 | 缝 |
|---|---|---|---|
| **V0（现在）** | `ToolKind::ALL` → `spec()` → 序列化 | `from_name` + `match`（enum） | `ALL` 手写 → 一条测试钉 |
| V1 | `Vec<Box<dyn Tool>>` → `spec()` → 序列化 | `iter().find` + `run` 装箱 Future | **无**（容器即清单） |
| V3 | MCP `tools/list` | `tools/call` | 无（协议保证） |

V0 这一改的净成本约 40 行，换来的是**声明与派发同源**。这跟 `config.rs` 那次"两条平行实现链"
是同一种病：**同一个事实有两个写入点，就一定会有漂移，只是早晚。**

#### 3.4.9 `ToolKind` 与 `Invocation` 必须是两个 enum（2026-10-20 实测）

问：注册表 `ToolKind`（无载荷）和执行层 `Invocation { ReadFile { path } }` 是不是重复？要让一个派生另一个吗？

**判决：必须两份，因为两者的「形状」不同，不是"懒得分层"。**

- `ToolKind` = **无载荷标签**（unit-only）⇒ 能枚举，`const ALL: &[ToolKind]` 成立；
- `Invocation` = **带载荷的调用** ⇒ **没法枚举**：带载荷的和类型**不存在"无参实例"**。

而注册表要求的恰恰是"每个工具恰好一个值，且这个值不带任何参数"。

**实测：硬合并其实能编译，但语义是假的。**

```rust
const ALL: &[Invocation] = &[Invocation::ReadFile { path: String::new() }];  // ✅ 编译通过
```

`String::new()` 是 `const fn`，所以这行合法。它暴露三个问题：

1. 每个"注册项"都是**假调用**（`path: ""`），`spec()` 只能向假实例问"你是什么工具"；
2. 同一类型既当"工具目录"又当"调用记录"，与 §3.4.4「执行层 `Invocation` 绝不进历史」直接冲突；
3. 换个参数类型（`enum Mode`、`Box<dyn …>`）连假实例都造不出来。

**`ToolKind` 的真正职责是「桥」：**

```
ToolCall --from_name--> ToolKind --parse(kind,args)--> Invocation --execute--> String
             (收窄)                    (校验)              (副作用)
```

`from_name(&str) -> Option<ToolKind>` 把模型给的任意字符串**收窄**成编译期可穷尽的集合，`parse` 才可能写成穷尽的 `match`。
`Invocation` 单独存在时，没有任何路径能从字符串进来。

**同步靠编译器，不靠"派生"：**

| 改动 | 谁抓你 |
|---|---|
| `ToolKind` 加变体 | `name()` 与 `parse(kind, args)` 两处 `match` **同时 E0004**，强迫你补 `Invocation` 分支 |
| 规格字段改 | `spec()` 返回类型不匹配 |
| **`const ALL` 漏加** | **没人抓** → §3.4.8 那条测试（长度 + 名字唯一 + `from_name` 往返） |

**四条衍生规则：**

1. `Invocation` **不要** `name()` / `from_name()` / `Serialize` / `to_functioncall()`。
   回灌的键是 `tool_call_id`，不是工具名；`arguments` 是字符串，一旦解析成 `Value` 就拼不回原样（§3.9 事实 3）。
   ⇒ 依赖方向**只有** `ToolKind → Invocation` 一条，不存在双向重复。
2. `parse` 吃 `kind: ToolKind` + `args: &str`，**返回 `Result<Invocation, ToolError>` 而不是 `Option`**
   —— `Option` 会把 `WrongParams { name }` 的信息丢掉，模型收不到"你错在哪"，回灌自纠直接失效。
3. V0 **不要**上 `macro_rules!` / strum 去生成两份：`ToolKind` 是**临时结构**，V1 换 `Vec<Box<dyn Tool>>`
   之后注册表不再需要 enum（§3.4.8 表）。给一个 V1 会消失的类型写宏 = 白写。
4. ⚠️ **`ALL` 的语义是"已实现的工具集"，不是"计划中的工具集"。** 往里放一个只有占位 schema 的工具，
   等于向模型宣告一份**不存在的契约**：它会去调，你还没实现，只能回灌 `unknown tool`。
   ⇒ **加变体最便宜的时刻，永远是它真被实现的那一刻。**

##### 转换函数住在哪一层（2026-10-20 追问后的澄清）

⚠️ **别把"两个类型"听成"两件事互不相干"。** 判据反过来用：**如果能合并，就根本不需要这条转换** ——
转换函数的存在本身就是"两个类型"的证据。真正算"搅合"的只有三种，见下表。

但转换函数的**落点**有讲究：别写成 `impl ToolKind { fn parse(self, args) }`，
那会让**协议层依赖执行层**（注册表知道了"怎么执行"）。正确方向：

```
协议层：ToolCall / FunctionDecl / ToolKind     只知道自己长什么样
   ↑ 被依赖（单向）
执行层：Invocation / 各自的 execute            知道协议，才可能解析 arguments
```

⇒ 落点 = `Invocation::parse(kind, args)`，或更省事的 `to_invocation(&ToolCall)`（§五 里那条）。
**判据：V1 把 `ToolKind` 换成 `trait Tool` 时，`Invocation` 不该被牵动** —— 依赖方向反了就会被牵动。

| 真·搅合（禁止） | 为什么 |
|---|---|
| `const ALL: &[Invocation]`（加假实例） | 注册项变成"假调用"，协议层被塞进执行层的数据 |
| `Invocation` derive `Serialize` / 带 `to_functioncall()` | 执行层反写回协议层；回灌键是 `tool_call_id`，重建 `arguments` 只会变形 |
| 给 `Invocation` 加 `name()` 然后拿它当注册表 | 一个类型同时当"目录"和"调用记录" |

**三个边界 = 三个错误变体（1:1，同 §3.3.2 `ValueError` 那条）**

| 边界 | 失败 | 变体 |
|---|---|---|
| `tc.payload()`：`type` 不是 `function` | 形状不认识 | `UnsupportedCallType { kind }` |
| `ToolKind::from_name(&f.name)` | 名字不认识（模型幻觉） | `UnknownTool { name }` |
| `Invocation::parse(kind, &f.arguments)` | 名字对、参数坏 | `WrongParams { name, detail }` |

现在 `ToolError` 把前两种混在 `UnsupportTool { name }` 里（`name` 有时塞 `type`、有时塞工具名）⇒
回灌时模型分不清该改"换调用方式"还是"换个工具名"。`detail` 是 serde 的原文
（`missing field \`path\``），没有它模型只知道"参数有误"却不知道错在哪 ⇒ §3.8 第 2 问白问。
⚠️ serde 多行报错很长，回灌前**截断**（§3.5 输出截断同样适用于错误文本）。

**顺带两个便宜优化（不急）**：`tools()` 每次调用都重建 `parameters` 的 `Value`（`json!` 的 map 会分配）；
在**循环外**构造一次 `Vec<WireTool>` 即可拿到"零分配/请求"和"输出字节稳定"——后者才是 `cached_tokens` 想要的。
（~~`OnceLock<Value>` 缓存~~ 已作废：不要 `Value` 这一层，见 **§3.7.6** 实测。）

#### 3.4.10 `FunctionDecl` 的字段为什么用 `&'static str`（2026-10-20 实测）

| | `&'static str` | `String` |
|---|---|---|
| 字段本身 | 16 B | 24 B |
| `FunctionDecl` 整体 | **72 B** | **88 B** |
| 造 4 个工具的分配次数 | **14** | **22** |
| 序列化产物 | **逐字节相同** | 同左 |
| 能从运行期数据反序列化 | **不能** | 能 |

三条硬差别：

1. **`&'static str` 是具体类型，不给 struct 添生命周期参数。** 写成 `&'a str` 才会让 `FunctionDecl<'a>` 污染每个签名。
   这是它和生命周期泛型的本质区别 —— `'static` 可以放进字段，`'a` 不行。
2. 从**运行期**数据反序列化 **E0597**：`let body = String::from(..); serde_json::from_str(&body)?` →
   `argument requires that body is borrowed for 'static`。
   ⚠️ **陷阱**：JSON 写成**字面量**时能编译（字面量本身是 `'static`），一换成 HTTP 响应体才炸 —— "我试过能跑"是假象。
3. 第三条路 `Cow<'static, str>` **可以** Deserialize：serde_core `de/impls.rs:1980` 的实现是
   `T::Owned::deserialize(d).map(Cow::Owned)`，**没有** `'de: 'a` bound ⇒ 运行期数据永远降级成 `Owned`。
   尺寸 24 B（niche 优化，与 `String` 相同），字面量路径 **0 分配**。

判据：**这个字符串是谁产生的？**
我方编译期常量 → `&'static str`；模型/网络/文件 → `String`；两者都可能且暂不想定 → `Cow<'static, str>`。

⇒ `FunctionDecl.name/description` 用 `&'static str`（顺带让 `Deserialize` 写不出来，正好对得上"请求侧只序列化"）；
`FunctionCall.name`（模型给的）**必须** `String`。
唯一真代价：以后想从 `tools.json` / MCP `tools/list` 加载就装不下 → 那时改 `String` 或 `Cow`（方向单向，好改）。

**工具参数 schema 的"定不定"分级**（同一件事的另一面：`spec()` 里的 `parameters` 有多少设计自由度）：

| 工具 | 参数 | 自由度 | 什么时候定 |
|---|---|---|---|
| `read_file` | `{ path }` | 零 | 已定 |
| `list_files` | `{ path }` | 零 | 已定 |
| `write_file` | `{ path, content }` | **零**（少了 `content` 就不是"写"） | **立刻**，且它把"写路径 + 确认流程"带进循环 |
| `run_command` | `{ cmd, … }` | **高**：白名单怎么表达才"不靠字符串判断就安全"（§3.8 第 3 问） | **延后**，等 §3.5 落地再定 |

⚠️ `strict: true` 的副作用：要求**每个** property 都在 `required` 且 `additionalProperties: false`
⇒ **strict 下不许有可选参数**（§3.4.5 实测 ⑤）。`list_files(path)` 想以后默认 `"."` 就得退回 `strict: false`。
且"网关接受 `strict` ≠ 它真的强制"（§3.9 实验 2.5 第 3 条）⇒ **执行层照样必须校验**，不能 `from_str` 后直接 `unwrap`。

#### 3.4.11 `Invocation` 不能 (de)serialize，那 `args` 怎么解析？（2026-10-20 实测）

问：`Invocation` 不 derive `Serialize`/`Deserialize`，是不是得自己从 `arguments` 字符串里手抠字段？

**不用手抠。serde 的强项正是解析，只是它解析的对象不该是 enum，而是「每个变体一个 Args struct」。**

根因：**判别标签不在 `arguments` 里**，它在 `function.name` 里（协议如此）。
serde 的三种 tagged enum 表示全都要求**标签出现在数据中**（external 还要多包一层、internal 要一个 tag 字段、adjacent 要两个字段）；
唯一"不需要标签"的是 `untagged`，代价是**它只能按字段形状猜**。

实测四种写法（`arguments = {"path":"x.txt"}`）：

| 写法 | 结果 | 判决 |
|---|---|---|
| enum `#[derive(Deserialize)]`（external） | `Err: unknown variant \`path\`, expected \`read_file\`` | ✗ 标签不在数据里 |
| `#[serde(untagged)]` | `Ok(ReadFile{path:"x.txt"})` | ✗✗ 猜的 |
| `#[serde(tag = "tool")]`（internal） | 可行 | 要 `arguments` 里冗余一份工具名，与 `function.name` 重复；`strict` 还得把它列进 `required` |
| **每变体一个 Args struct** | `Ok(ReadFileArgs{path:"x.txt"})` | ✓ |

`untagged` 的三条致命实测（`enum {ReadFile{path}, ListFiles{path}, WriteFile{path,content}}`）：

```
{"path":"x","content":123}  → Ok(ReadFile{path:"x"})   ← 参数坏了不报错，还静默选了错的变体
{"path":"."}                → Ok(ReadFile{path:"."})   ← list_files 的调用被读成 read_file
{}                          → Err("data did not match any variant of untagged enum Inv")  ← 等于没说
```

`read_file` 与 `list_files` 参数**同形**（都是 `{path}`），untagged 永远命中第一个 ⇒ 名字这层信息被彻底丢掉。

**推荐形状：变体直接装 Args struct，字段名只写一次**

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadFileArgs { pub path: String }

#[derive(Debug)]
pub enum Invocation {
    ReadFile(ReadFileArgs),   // 字段名只有 Args 这一个来源，变体里不再重写
    WriteFile(WriteFileArgs),
}
```

配一个泛型小 helper，把"解析 + 错误映射"收成一处（每个 match 臂一行）：
`fn from_args<T: DeserializeOwned>(kind: ToolKind, args: &str) -> Result<T, ToolError>`
—— 用 `DeserializeOwned` 而不是 `Deserialize<'a>`，是为了不给 `parse` 添生命周期参数。

**serde 的报错质量（这就是 `WrongParams { detail }` 的来源）：**

```
缺字段    missing field `content` at line 1 column 12
多余字段  unknown field `force`, expected `path` at line 1 column 19
类型不对  invalid type: integer `123`, expected a string
坏 JSON   EOF while parsing a value at line 1 column 8
```

**`deny_unknown_fields` 在这里该开，与 §3.9 事实 5（外层响应绝不能 deny）不矛盾 —— 方向不同：**

| | 谁定的字段集 | 多余字段意味着 | 策略 |
|---|---|---|---|
| **输入侧**（模型生成的 args） | **我方** schema | 模型幻觉 | `deny_unknown_fields` + 报出字段名 |
| **输出侧**（provider 响应） | 对方 | 对方加了字段（`logprobs`） | **绝不能 deny** |

**什么时候才值得手抠 JSON**：`v["path"].as_str().ok_or(..)` 只在需要**跨字段校验**（`path` 与 `cwd` 互斥）
或自定义错误语义时才划算。V0 的工具参数全是"字段 → 类型"的直映射 ⇒ **全交给 serde**。

### 3.4.12 一工具一 `Args` struct：成本核算（V0 是否合理）

**问**：「加个新工具就要加个新 struct，这做法现在合理吗？」

**答：合理，而且它不是为了"每个工具配一个"才写的 —— 是每个工具的契约本来就不同。**
但实测暴露了 **2 处静默漏洞 + 1 处真双写**，那三处该修。

#### 实测：加第 4 个工具（`grep {pattern, path}`）到底动几处

基线 = 3 工具（`read_file` / `list_files` / `write_file`），只给 `ToolKind` 加 `Grep` 变体：

```
error[E0004]: non-exhaustive patterns: `ToolKind::Grep` not covered   → name()
error[E0004]: non-exhaustive patterns: `ToolKind::Grep` not covered   → spec()
error[E0004]: non-exhaustive patterns: `ToolKind::Grep` not covered   → parse()
（共 3 个，from_name 与 ALL 一声不响）
```

补全后统计：**改 7 处，净增约 10 行。**

| # | 位置 | 编译器抓？ |
|---|---|---|
| 1 | `enum ToolKind` 加变体 | — |
| 2 | `const ALL` 加一项 | ❌ **静默** |
| 3 | `name()` 加臂 | ✅ E0004 |
| 4 | `spec()` 加臂 | ✅ E0004 |
| 5 | `parse()` 加臂 | ✅ E0004 |
| 6 | `enum Invocation` 加变体 | — （`parse` 的臂会逼它存在） |
| 7 | `struct XxxArgs` | — |
| — | `from_name()` | ❌ **静默**（`_ => None` 兜底） |

#### 漏洞 ①：`from_name` 的手写 `match` 是第 5 个真相源，可整个删掉

```rust
// 现在（每个工具名写两遍：name() 一次、from_name() 一次）
pub fn from_name(n: &str) -> Option<ToolKind> {
    match n { "read_file" => Some(..), ..., _ => None }
}

// 改成：名字只在 name() 里出现一次，ALL 是唯一权威
pub fn from_name(n: &str) -> Option<ToolKind> {
    Self::ALL.iter().copied().find(|t| t.name() == n)
}
```

**这一步同时消掉两个问题**：

- 工具名从两处降到**一处**（`name()`），`from_name` 加工具时彻底不用改；
- 手写版会出现**自相矛盾状态**：`match` 写了 `"grep"` 但 `ALL` 忘了加 ⇒ `from_name` 认识 grep、`tools` 却不宣告它（死代码，模型永远调不到，还没人报错）。改 `find` 后两者不可能不一致 —— 最坏情况是**一致地不存在**（工具真没生效，一眼能看出）。

代价：`from_name` 从 O(1) 变 O(n) 线性扫。n=3~10 个 `&'static str` 比较，**在「一次工具调用」面前完全不可观测**，别为它纠结。

#### 漏洞 ②：`ALL` 依然没人管 → 只能靠测试

`ALL` 漏加变体 = 工具静默不生效。编译器管不到（数组长度不是类型的一部分）⇒ 用 §3.4.8 那条测试钉住：
`ALL.len() == 预期` + 名字唯一 + 每个 `t` 满足 `from_name(t.name()) == Some(t)`（往返）。

#### 真双写：`spec()` 的 JSON Schema 字符串 ↔ `Args` 字段

实测把 `PathArgs { path }` 改成 `{ file }`：**只报 1 个 `E0560`（构造点）**，
`spec()` 里那 5 处 `"path"` 字符串**编译器一个都不查**。

后果：忘了改 schema ⇒ 模型照 `path` 调、`deny_unknown_fields` 拒掉 ⇒
`unknown field \`path\`, expected \`file\``。**运行时才暴露，且只在模型真调这个工具时才暴露。**

这是**同一份契约的两种表示**（对外描述 / 对内校验），跨语言边界上无法靠 `derive` 消掉。三条路：

| 做法 | 评价 |
|---|---|
| 手工对齐 + 一条「样本参数」测试（每个 `ToolKind` 喂合法样本，`parse` 必须 `Ok`） | **V0 选这个**，成本 1 个测试 |
| `schemars` 从 `Args` struct **生成** schema | V1 正解，字段名只剩一份 |
| `typify` 从 JSON Schema 生成 struct | 反过来，适合 schema 是外来的（MCP） |

#### 那为什么 V0 不直接上 `trait Tool`（V1 形态）

实测同样加一个工具：**trait 版 10 行，但只写在一个连续段落 + 注册表一行**（enum 版 10 行散在 7 处）。

```rust
pub trait Tool: Send + Sync {
    type Args: DeserializeOwned;     // 关联类型，每个工具自带
    const NAME: &'static str;
    fn description() -> &'static str;
    fn schema() -> serde_json::Value;
    fn run(args: Self::Args) -> Result<String, String>;
}
```

**trait 版更好，但现在上不划算，两个硬理由：**

1. **`run` 最终必须 `async`，而 async trait 方法不 dyn 兼容（E0038）** ⇒ 要 `Pin<Box<dyn Future>>` 手写，或引入 `async-trait`。V0 还在跑通阶段，不值得先吃这个复杂度。
2. **`ToolKind` 在 V1 会整个消失**（注册表变成 `Vec<Box<dyn Tool>>`）。为它写宏/生成器等于给一次性结构投资。

**关键认知：现在的「一工具一 `Args` struct」不是弯路，是 V1 的预演。** V1 里 `Args` 变成 `trait Tool` 的关联类型，struct 不是变多而是**成为唯一形态**。所以现在写的每个 `XxxArgs` 到 V1 都留着，既不白写、也不会被推翻 —— 只是从「散在 7 处的变体」收进「一个 `impl Tool` 块」。

#### 判据（回答"要不要为这个工具建 struct"）

> **这个工具的参数字段集，是不是只被这一处使用、且有明确类型？**
> 是 → 建 struct（V0 的三个工具全是）。
> 否（多个工具共用同一字段集，如 `read_file` / `list_files` 都是 `{path}`）→ 可以复用同一个 struct，
> **但先问语义是否相同**：`read_file.path` 是文件、`list_files.path` 是目录，未来会分化（`recursive`、`pattern`）
> ⇒ **V0 也分开写**，一个 struct 3 行，别为省 3 行制造耦合。

### 3.4.13 `content` 不是处处可选 —— 三个 role 实测（2026-10-20 真实请求）

现在 `Message` 四个变体的 `content` 全是 `Option<String>`。**实测同一个 `null`，三个 role 反应完全不同：**

| role | `"content": null` | HTTP | 真实后果 |
|---|---|---|---|
| `system` | **静默丢弃** | 200 | 模型完全没看到 system prompt，无任何报错 |
| `user` | **静默接受** | 200 | 模型**幻觉出一个问题**：我发的是 `null`，它答"法国首都是** Paris**" |
| `tool` | **拒绝** | 400 | `tool message content must be a string on the gRPC path` |
| `assistant` | 合法 | 200 | 只有 `tool_calls` 时本来就是 `null` |

**`user` 那一行是最危险的形态**：不报错、不 400、模型自己发明了一个问题并回答了它。这种 bug 只能靠**类型**消灭，回灌错误信息没用（根本没产生错误）。

⇒ **`System` / `User` / `Tool` 的 `content` 用 `String`（不是 `Option`），`Assistant` 保留 `Option<String>`。**

判据（同 §3.4.4「让非法状态不可表达」）：

- 三个 role 的 content 由**我方**产生（system prompt 是常量、user 输入是命令行参数、tool 输出是执行结果）
  ⇒ 我方永远有值，`Option` 只是允许自己写错；
- `Assistant` 的 content 由**模型**产生，三个合法形态都存在（有文本 / 纯 `tool_calls` 为 `null` / 收尾文本）
  ⇒ 必须 `Option`。

**注意这个改动让 `Tool` 变体的空输出怎么表达**：用 `String::new()`（空字符串），不用 `None`。
空输出 = "命令跑了但没打印任何东西"，是**有意义的状态**；`None` = "没有输出这个概念"。§3.4.7 那条判据
（"空"是不是有意义的业务状态）在这里的答案是"是"。

#### 顺带：`parse_kind` 的两步解析丢掉了行号列号（实测）

现在是 `from_str::<Value>(args)` 再 `from_value::<T>(v)`。同一批坏输入对比：

```
{"path":123}               两步: invalid type: integer `123`, expected a string
                           一步: invalid type: integer `123`, expected a string at line 1 column 11
{"path":"a","force":true}  两步: unknown field `force`, expected `path`
                           一步: unknown field `force`, expected `path` at line 1 column 19
```

⇒ 直接 `from_str::<ReadFileArg>(args)` 一步到位。好处两个：保留位置信息（`column 19` 指出坏在哪个字段）、
少一次 `Value` 的中间分配。**`ToolError::BadParse` 随之可以删掉** —— 非法的 `arguments` 一律是
`WrongParams { name, details }`，"JSON 坏了"和"字段不对"对模型来说是同一类事（都是"参数有误"），
分成两个变体反而要维护两个 `map_err`。

### 3.4.14 循环开始之前，数据层还差什么

`model.rs` 现有：`Message` · `ToolCall` · `FunctionCall` / `CustomCall` · `Invocation` + 三个 `Arg` ·
`WireTool` / `FunctionDecl` / `ToolKind` / `ToolError`。**够描述"一条消息"和"一份工具声明"，不够描述"一次请求"和"一次响应"。**

循环要用的两样东西，现在还只能靠 `main.rs` 手抠 `v["choices"][0]`：

```rust
// 请求侧
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<WireTool>,              // 或 Value；见 §3.4.8 OnceLock
    pub tool_choice: &'static str,          // "auto"
    pub parallel_tool_calls: bool,
}
// 响应侧（只声明要用的字段，其余丢弃 —— 绝不开 deny_unknown_fields）
pub struct ChatResponse { pub choices: Vec<Choice>, pub usage: Option<Usage> }
pub struct Choice { pub message: Message, pub finish_reason: Option<String> }
pub struct Usage { pub prompt_tokens: u32, pub completion_tokens: u32, pub total_tokens: u32 }
```

三个要点：

1. **`Choice.message` 直接用 `Message`**，不要新建 `AssistantMessage`。
   响应里的 message 恒为 `role:"assistant"`，tag enum 会正确分派到 `Assistant` 变体；
   而回灌时本来就需要一个 `Message` —— **同一个类型既能解析又能原样回灌**，这正是 §3.4.4 选 tag enum 的收益。
2. **`finish_reason` 不参与终止判断**（§3.4.4 已定：判据是 `tool_calls.is_empty()`）。
   声明它只是为了打日志 / 排查，用 `Option<String>` 容忍新值。
3. **`usage` 用 `Option`**：V0 只想在日志里确认 `cached_tokens` 有没有涨（验证"历史只追加不改写"），
   拿不到就继续跑。**不要**为它写 `#[serde(default)]` 之外的任何东西。

> ⚠️ **上面这个 `ChatRequest` 只是"协议形状"的速记，不是最终形态。** 它该不该 `pub`、`messages` 该是
> `Vec<Message>` 还是 `&[Message]`、以及 `chat()` 到底收参数列表还是收它，见 **§3.7.6**（结论：
> **私有 + 借用 + 只在 `chat` 内部活一次**，对外签名是参数列表）。

**不需要的（别提前建）**：`logprobs`（忽有忽无，见 §3.9 事实 5）、`system_fingerprint`、`service_tier`、
流式的 `delta`（V1）、`tool_calls[].index`（流式专用）。

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

#### `thiserror` 与 `anyhow` 的分工：按「API 边界」而非「crate 类型」

> ⚠️ **2026-10-17 更正**：旧口诀是「**库用 thiserror，二进制用 anyhow**」。它按 **crate 类型**划分，粒度错了 ——
> 真正的判据是 **`pub` 可见性**：`lib` 里的 `pub fn`，错误类型就是它 API 承诺的一部分（`# Errors` 那一节写的就是它）；
> `lib` 里的私有函数用 anyhow 完全没问题；`bin` 的 `main` 用 anyhow 也对。
> `config` 恰好卡在缝上：住在 `lib` 里、且是 `pub` ⇒ **必须具体类型**。

| | `thiserror` | `anyhow` |
|---|---|---|
| 用途 | **内核 / 领域错误**：可 `match`、可分类、可穷尽 | **出口层**：类型擦除，只能 `downcast` |
| 形状 | `enum`（`ConfigError` 就是） | `anyhow::Error` + `.context()` |
| 什么时候用 | 调用者要**按类型处理**，或测试要断言变体 | **没人能按类型处理**，只能打印或上抛 |

两者**可共存**，机制是 `anyhow::Error` 对 `E: std::error::Error` 的 blanket `From` ——
所以 `main` 里 `Config::from_env()?` 照样能把 `ConfigError` 收进 `anyhow::Result`，**main 一行都不用改**。

⚠️ **本项目原先的示范是错的**：「`ConfigError`（thiserror）在里，`from_env() -> anyhow::Result<Config>` 在外」——
`from_env` / `from_map` 都应是 `Result<_, ConfigError>`。旧文档还同时写着「`anyhow::Context`」和「错误要结构化，不要字符串」，
两条本身在打架：`Context` 加的是**字符串**，`Missing { name }` 加的是**类型字段**。

**`anyhow` 一个被低估的用法：它是「延迟设计类型」的工具。**
V0 第一步根本不知道会失败几次、都有哪些形态（DNS 错 / 401 / 429 / JSON 形状不对 / `choices` 为空…）——
这时硬定义 `ChatError` 是**在信息不足时做类型设计**，八成要返工。先用 anyhow + `.context()` 跑通，
等真实失败形态见全了再抽 thiserror。**升级信号：你第一次需要 `match` 它。**

> 实测证据（2026-10-17）：三层调用各加一句 `.context()`，`io::Error` 的原产地不动、类型零改动，
> `{:#}` 打成一条链 `初始化 agent 失败: 读取工具列表 tools.json 失败: 系统找不到指定的文件。 (os error 2)`，
> `{:?}` 打出完整 `Caused by:` 列表。thiserror 想获得同样效果，得为每句说明新增一个 enum 变体。

#### 几个容易踩的点

- **`.ok()` 会静默吞错**（`Result` → `Option`，错误丢掉；反向是 `.ok_or(e)`）。
  用它之前必须能回答「**这个错误真的可以不管吗**」。`dotenvy::dotenv().ok()` 属正当（没有 `.env` 也该能跑）。
  §3.3.2 里 `from_map` 的闭包**曾经**返回 `Option`，就是这类"主动丢信息"付学费的真实案例
  （现已改成 `Result<String, ValueError>` —— 见方案 C）。
- **`anyhow::Ok` 是真实存在的自由函数**（带 `#[allow(non_snake_case)]`），是 `E` 推断失效（**E0282**）时的备用写法，
  等价于 `Ok::<_, anyhow::Error>(v)`，常见于闭包 / iterator 链。
  **别 `use anyhow::Ok;`** —— 会遮蔽 prelude 的 `Ok`，报错位置离根因很远。需要时写限定路径。
- **错误要结构化，不要字符串**：`Missing { name }` 让人一眼知道缺哪个变量；拼成
  `"缺少环境变量 LLM_API_KEY"` 就丢了可编程性（`ConfigError` 已经做对了）。
- ⚠️ **全局态是测试的敌人**：`std::env::set_var` 在 edition 2024 是 `unsafe`（它会污染整个进程、
  与并行跑的测试互撞）。凡是依赖全局态的函数都难可靠测试 ——
  **这才是"依赖注入"（§3.3.2 的 `from_map`）的真实动机**，不是什么优雅癖。

### 3.7.2 `trait LlmClient` 什么时候抽：**跑通之后再抽，但先把缝留好**

**问**：「数据层够了吗？接下来先写循环还是先写 `LlmClient`？」

**答：数据层还差两块（§3.4.13 `content` 收紧 / §3.4.14 `ChatRequest` + `ChatResponse`）；
顺序是 `llm.rs` → 循环 → 最后抽 trait。** 不是"循环 vs trait"二选一 —— 循环依赖 `chat()`，trait 依赖循环的形状。

#### 为什么不先写 trait：签名不该猜（有实证）

§3.7 上面那段签名是 **2026-10-16** 拍的：

```rust
async fn chat(&self, msgs: &[Message], tools: &[ToolSpec]) -> Result<Response>;
```

跑完实验 2 / 2.5（10-19）才发现它**少了三样**：`model` 从哪来（现在在 `Config` 里，但 trait 不知道 `Config`）、
`tool_choice` / `parallel_tool_calls`（现在是请求体的一部分）、`usage`（要它才能验证 `cached_tokens`）。
`ToolSpec` 这个名字也不对 —— 实际用的是 `WireTool { kind: "function", function: FunctionDecl }`（§3.4.8）。

> **同一个签名在"没跑通"和"跑通一次"之间的差异，就是猜和抄的差异。**
> 这是 §3.4.8「给 V1 会删的结构写宏是白写」的同一条道理，只是这次的"一次性结构"是**签名**。

#### 为什么跑通之后抽 trait 几乎没有成本

V0 只有一个实现（OpenAI 兼容），所以 trait 的价值**不是多态，是可注入** —— 唯一兑现场景是写测试。
而"注入"这件事在调用点只有**一处**：

```rust
// 跑通版：具体类型
async fn run_loop(client: &OpenAiClient, cfg: &Config, question: &str) -> anyhow::Result<()>

// 抽 trait 后：只改类型标注 + 加一个 bound
async fn run_loop<C: LlmClient>(client: &C, cfg: &Config, question: &str) -> anyhow::Result<()>
```

**函数体一个字都不用动**。所以抽 trait 的正确定价是「5 分钟」，而提前写的定价是「猜一个错签名 + 回来改」。

#### 「留缝 ≠ 写抽象」—— 循环里必须遵守的两条

跑通版虽然不写 trait，但有两件事**必须现在就做对**，否则后面抽不动：

1. **`client` 必须从参数进来**，循环内部**不许** `reqwest::Client::builder()...` 或读 `.env`。
   一个函数自己造依赖 = 那个依赖永远注入不进去。
2. **循环的返回值里不要出现 `reqwest` 的类型**（不要返回 `reqwest::Response`，不要 `Result<_, reqwest::Error>`）。
   出口用 `anyhow::Result`（§3.7.1：出口侧用具体类型的错误，`reqwest::Error` 会泄漏进内核）。

做完这两条，"抽 trait"就退化成"改一个泛型参数"。

#### ⚠️ 一个跑通之后才会撞上的限制

`async fn` 在 trait 里**不 dyn 兼容（E0038）** ⇒ 抽出来的 `LlmClient` 只能当**泛型约束**用，
**不能** `Box<dyn LlmClient>`。V0 完全没问题（只有一个实现 + 一个调用点）；
但 V2 想做「运行时多后端」（`Vec<Box<dyn LlmClient>>`）时，要么 `#[async_trait]`，要么手写
`Pin<Box<dyn Future + Send>>`。**知道这个坑在哪就行，别为它提前设计。**

#### 循环本身的骨架（顺序 + 六种分支，不给实现）

顺序上 `llm.rs` 先，因为循环第一行就要 `client.chat(...).await?`。循环里必须显式处理的分支：

| # | 情形 | 处理 |
|---|---|---|
| 1 | `choices` 为空 | **报错退出**（协议违例，不是可恢复错误） |
| 2 | `tool_calls` 为空 | 打印 `content`，**正常结束**（判据是这里，不是 `finish_reason`） |
| 3 | `tool_calls` 非空 | 先把 assistant 消息**原样** push 进 history（含 `tool_calls`，§3.9 事实 1） |
| 4 | 单条解析失败（未知工具 / 参数坏 / `type` 不是 `function`） | 把 `ToolError` 的 `Display` 文本当成 `Tool.content` 回灌，**继续** |
| 5 | 执行成功 | 输出**截断后**回灌（§3.5） |
| 6 | 步数用尽 | 停，并把最后状态打出来 |

两个容易漏的点：

- **`for step in 0..max_steps` 比 `loop` + 手动计数安全**：天然不可能死循环，也不用自己维护计数器。
- **并行的 `tool_calls` 每条都要回一条 `role:"tool"`**（§3.4.3）。漏一条，下一轮请求直接 400 —— 
  这类错误网关会明确报 `tool_call_id`，算幸运的。

### 3.7.3 `Config` 的字段归谁：client / `ChatRequest` / loop

**问**：「config 里那些设置需不需要写进 client class？」

**判据一句话：换掉它，需要重建一个 `reqwest::Client` 吗？**

| `Config` 字段 | 归属 | 判据 |
|---|---|---|
| `llm_base_url` | **client**（拼成 `chat_url` 存起来） | 换 host 必须重建连接池 |
| `llm_api_key` | **client 存字段**，每请求 `.bearer_auth(&self.api_key)` | 同上。**⚠️ 别塞进 `default_headers`**，见下方实测 |
| `llm_model` | **`ChatRequest.model`** | 同一条连接可以换模型；协议里它本来就是**请求体字段** |
| （未来）`max_steps` / `sandbox_root` | **loop / tools** | 与网络无关，client 不该知道 |

`model` 那格最容易搞错。它看起来像"客户端的属性"，但协议把它定义成 `{model, messages, tools, ...}` 里的一个字段
⇒ **权威在请求里，client 顶多存一个默认值**。一旦 client 也存一份，就有了两个真相
（`cfg.llm_model` / `client.model` / `req.model`），§3.4.5 那种漂移又回来了。

#### 建议形状：client 只装"连接身份"，`ChatRequest` 由 `chat` 内部组装

```rust
pub struct OpenAiClient {
    http: reqwest::Client,      // 只放 UA / session（非敏感）；api_key 不在这里
    chat_url: String,           // base_url + "/chat/completions"，拼一次
    model: String,              // 默认模型，填进 ChatRequest
    api_key: String,            // 每请求 .bearer_auth(&self.api_key)
}
impl OpenAiClient {
    pub fn new(cfg: &Config) -> anyhow::Result<Self>;      // 吃 &Config，但【不存】Config
    pub async fn chat(&self, msgs: &[Message], tools: &Value) -> anyhow::Result<ChatResponse>;
}
```

三点说明：

1. **`new(&Config)` 可以，`OpenAiClient { cfg: Config }` 不建议。** 前者是"从配置**读**"，后者是"**持有**配置"。
   区别在 `Config` 以后会加 `max_steps`、`sandbox_root` 这类与网络无关的字段 —— 存整个 `Config`，
   client 的字段列表就不再回答"我到底依赖哪些东西"。
2. **`chat_url` 在 `new` 里拼一次。** 每次 `format!` 拼路径是 §3.4.5 那个"同一事实两个写入点"的缩小版：
   路径拼错一次，全项目每个请求都 404，而且错在字符串里、编译器不管。
3. **`ChatRequest` 仍然要建，但只活在 `chat` 内部。** 它的价值是**序列化的唯一真相**（字段名写错 1 秒 400）；
   如果不用它而用 `json!` 手拼，`ChatRequest` 就退化成"没被使用的好设计"。
   `messages` / `tools` 从参数进，`model` 从 `self` 进，`tool_choice` / `parallel_tool_calls` 是写死常量。

**为什么 `tools` 走参数而不是存 client**：`ToolKind::tools()` 是关联函数（§3.7.6 已把 `tool_json()` 判为多余中间层）；
V1 要按权限/模式 `retain` 过滤工具集（§3.4.8 三级注册）⇒ 工具集是**每次请求都可能不同**的东西。存 client 里以后要改。

#### ⚠️ 更正：`api_key` **不该**进 `default_headers` —— 三种写法只有一种「默认安全」

上一版这里写的是「key 进 `default_headers`」，**那个建议是错的**。实测（reqwest 0.13.5）四种写法：

| 写法 | `Client` / `Request` 的 `{:?}` | 结论 |
|---|---|---|
| `default_headers` 里 `format!("Bearer {k}").parse()` | `"authorization": "Bearer sk-SUPER-SECRET-KEY"` | ❌ **泄漏** |
| `default_headers` 里手动 `set_sensitive(true)` | `"authorization": Sensitive` | ✅ 安全，但要自己记 |
| `.header(AUTHORIZATION, format!("Bearer {k}"))` | `"authorization": "Bearer sk-SUPER-SECRET-KEY"` | ❌ **泄漏** |
| **`.bearer_auth(k)`** | `"authorization": Sensitive` | ✅ **自动**安全 |

**机制在源码里**（`reqwest-0.13.5/src/async_impl/request.rs:208`）：

```rust
pub fn bearer_auth<T>(self, token: T) -> RequestBuilder where T: fmt::Display {
    let header_value = format!("Bearer {token}");
    self.header_sensitive(crate::header::AUTHORIZATION, header_value, true)
    //                                                              ^^^^ sensitive
}
// header_sensitive 内部： if sensitive { value.set_sensitive(true); }
```

而普通 `.header()` 不走 `header_sensitive` ⇒ 不标记 ⇒ `Debug` 明文。`default_headers` 的官方文档
（`client.rs:1159`）也明写了这件事：

```rust
/// // Consider marking security-sensitive headers with `set_sensitive`.
/// let mut auth_value = header::HeaderValue::from_static("secret");
/// auth_value.set_sensitive(true);
```

**`set_sensitive` 只影响 `Debug`，不影响发送**（实测）：

```
Debug    = Sensitive
to_str() = Ok("Bearer sk-SUPER-SECRET-KEY")
as_bytes = "Bearer sk-SUPER-SECRET-KEY"
== 原值  = true
```

⇒ **结论：`llm_api_key` 存成 client 的字段，每请求 `.bearer_auth(&self.api_key)`。**
`default_headers` 只放 `user-agent` / `x-opencode-session`（本来就不敏感）。

这样两条防线互相独立、各自可验证：

- `http` 里**没有** key ⇒ `Client` 的 `Debug` 爱怎么打怎么打，`derive(Debug)` 都不会漏；
- `api_key` 是普通 `String` 字段 ⇒ 走你已经练过一遍的手法（手写 `Debug` 打 `"***"`，同 `Config`）。

**如果坚持用 `default_headers`**，那规矩就变成"struct 里有 `reqwest::Client` 就不能 `derive(Debug)`"，
因为 `Client` 的 `Debug` 是 reqwest 实现的、你覆盖不了：

```rust
#[derive(Debug)]                 // ← 这一行 + key 在 default_headers = 泄漏
pub struct OpenAiClient { http: reqwest::Client, .. }
```

`dbg!(client)` / `println!("{:?}", client)` / `unwrap()` 失败打印 / 测试断言失败 —— **key 全明文出去**。
对照 §3.3.2：你花力气给 `Config` 手写 `Debug` 打 `"***"`，这一处足以让那道防线整个作废，而且没人会想到去看。

#### 好消息：`Request` / `RequestBuilder` 的 `Debug` 本身不危险

`default_headers` **在 `send()` 时才合并**，所以：

```
// 走 default_headers 的 client 建出来的 Request
Request { method: POST, url: ..., headers: {} }
```

`Request::headers()` 里没有 `authorization`，body 也不打印。`dbg!(req)` 唯一会漏的情形是
**你自己用 `.header()` 塞了 key**（表格第三行）。

⇒ 两道防线是独立的：**header 层用 `bearer_auth`（自动 sensitive），struct 层手写 `Debug` 遮 `api_key` 字段。**

```rust
impl std::fmt::Debug for OpenAiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiClient")
            .field("chat_url", &self.chat_url)
            .field("model", &self.model)
            .field("api_key", &"***")          // 与 Config 同手法
            .field("http", &"<reqwest::Client>")   // 里面已无 key，但没必要打
            .finish()
    }
}
```

> 注意：这里遮的是**自己的字段** `api_key`，不是 `reqwest::Client` 内部的 header —— 后者靠 `bearer_auth` 的
> `Sensitive` 标记解决。**两个位置各有一道，缺一不可。**

#### 顺带：`reqwest::Client` 只有 **8 字节**（一个 `Arc` 指针）

`size_of::<Client>() == 8`，`Option<Client>` 也是 8（niche 优化）。`Clone` 是 `Arc` 自增，极廉价。

⇒ **`OpenAiClient` 可以放心 `#[derive(Clone)]`**（`String` 字段各自 clone，两个 String 而已），
不必为"复制连接池"担心。但**别在循环里每步 `new` 一个** —— 那就真的是新连接池 + 重新 TLS 握手，
而且 §3.9 那个 `cached_tokens: 256` 也帮不了你（prompt cache 是网关侧，与 TCP 无关，但连接复用的开销是实打实的）。

### 3.7.4 「用接口」和「用 JSON 拼」是两个正交的问题

**问**：「`.bearer_auth(llm_api_key)` 也是自己用 json 拼、而非使用接口，是吗？」

**不是。`bearer_auth` 就是接口**（源码见 §3.7.3，它内部 `format!("Bearer {token}")` + 自动 `set_sensitive`）。
把两件事分开看就清楚了：

| | 有没有"专门接口" | 手拼的后果 |
|---|---|---|
| **header** | ✅ 有：`bearer_auth` / `basic_auth` / `user_agent` / `header()` | `Sensitive` 标记丢失 → `Debug` 泄漏；`Bearer ` 前缀可能拼错 |
| **body** | ⚠️ 只有泛型 `.json(&T)`，**没有"按 schema 生成"的接口** | 字段名/嵌套写错 → **1 秒 400** |

所以「用 JSON 拼」说的**只是 body 这一侧**，而且它真正的对立面不是"用不用接口"，而是**载荷有没有类型**：

```rust
client.post(url).json(&json!({ "model": m, "messages": msgs }))   // 接口用了，载荷无类型
client.post(url).json(&ChatRequest { model: m, messages: msgs, .. }) // 接口同样用了，载荷有类型
```

两者调的是**同一个** `.json()`，差别只在 `T` 是 `Value` 还是 struct。而 `json!` 那版的失败模式与 §3.4.5
一模一样：写错一个键，编译零告警，服务器 400。

> 判据：**header 一律走专用方法（尤其认证类，因为 `bearer_auth` 免费送你 `Sensitive`）；
> body 一律走有类型的 struct（`json!` 只在"临时探针"里用）。**

顺带一条：`reqwest` 对 body 也做不了更多 —— 它不知道你的 schema。这条鸿沟只能靠 `ChatRequest` 这个 struct 填。

---

### 3.7.5 `Debug` 到底写不写：三种方案（2026-10-20 实测）

`api_key` 既然存成 client 的普通字段（§3.7.3 更正后），`derive(Debug)` 一定泄漏 ⇒ 只剩三个选择：
手写、用 newtype 包住、或者干脆不实现。**先纠正一个普遍误解，它决定了"不写"的代价有多大。**

#### ⚠️ 误解：`unwrap()` 要求 `Ok` 类型 `Debug`

**不要求。** 实测（一个完全不实现 `Debug` 的 `NoDebug`）：

| 用法 | 结果 |
|---|---|
| `Result<NoDebug, anyhow::Error>.unwrap()` | ✅ 通过 |
| `.expect("mk client")` | ✅ 通过 |
| `?` 传播到 `anyhow::Result` | ✅ 通过 |
| `dbg!(&c)` | ❌ **E0277** |
| `println!("{c:?}")` | ❌ **E0277** |
| `assert_eq!(a, b)` | ❌ **E0369**（`PartialEq`）+ **E0277**（`Debug`） |
| 父 struct `#[derive(Debug)]` 含该字段 | ❌ **E0277**，立刻级联到父 |

根因：`Result::unwrap` 的签名是 `where E: Debug` —— **约束在 `Err` 上**，与 `Ok` 类型无关。
所以"日常错误处理"（`unwrap` / `expect` / `?`）**一条都不受影响**，这也是"不写 Debug"成本远比想象低的原因。

⇒ **不写 Debug 的真实代价 = 失去 4 处可观测性，而且每一处都是编译期错误，不是运行期。**
不存在"忘了写 Debug，结果运行时悄悄泄漏"这种情形 —— 反向才成立（写了 `derive` 才会泄漏）。

#### 三种方案对比

| | 写法 | 以后新增字段时 | `{:?}` 输出 | 失效方向 |
|---|---|---|---|---|
| **A 整块手写** | `impl fmt::Debug`，逐字段 `.field("api_key", &"***")` | 忘了写 ⇒ **就不打印**（不泄漏，只是看不见） | `ClientA { api_key: "***", model: "m" }` | **fail-closed** ✅ |
| **B newtype + derive** | `struct Secret(String)` 手写 `Debug`，父 struct 照常 `#[derive(Debug)]` | 自动进 Debug；**新字段若也是敏感值而忘了包 ⇒ 泄漏** | `ClientB { api_key: "***", model: "m" }` | **fail-open** ⚠️ |
| **C 完全不实现** | —— | 任何 `{:?}` / `dbg!` **直接编译不过** | 不存在 | 最强 fail-closed |

A 和 B 的打印结果**完全一样**（实测），差别只在"以后改代码时哪一边更容易出错"：

- A 漏写一个字段 → 打印里少一项，**不会泄漏**，但也可能长期没人发现"这个字段没被观测到"；
- B 漏包一个敏感字段 → **直接泄漏**，而 B 的卖点恰恰是"父 struct 可以放心 derive"。

> 判据：**敏感字段多、且以后还会增加 → A（默认安全）；敏感字段只有一个、以后主要加普通字段 → B（默认好用）。**

方案 C（不实现）的唯一硬代价：开了 `#![warn(missing_debug_implementations)]` 会告警
（实测 `type does not implement Debug; consider adding #[derive(Debug)] or a manual implementation`；
该 lint 默认 `allow`，但不少项目/clippy 配置会开）。另外 client 从此**放不进任何 `derive(Debug)` 的结构** ——
V0 是裸 CLI，没有 `AppState`，所以无所谓；V2 若做服务端就会卡。

#### V0 建议：A（和 `Config` 同一个手法）

只有 `http` / `chat_url` / `model` / `api_key` 四个字段，`config.rs` 已经练过一遍手写 `Debug` 打 `"***"`，
**保持同一风格比省 6 行重要**。等 V1 字段涨到六七个（`timeout` / `retry` / `session` …）再换 B。

#### 一个实测小坑：手写 Debug 之后 `api_key` 会报 `dead_code`

因为 `Debug` 里写的是 `&"***"` 而不是 `&self.api_key`，编译器认为该字段**从未被读取**：

```
warning: field `api_key` is never read
```

真实项目里它会被 `.bearer_auth(&self.api_key)`（§3.7.4）读到，所以不会有这个警告。
**但如果哪天它真的只被 `new` 写、再没被读**，这个警告反而有用 —— 说明 token 压根没发出去。

#### 正统解：`secrecy`（B 方案的成品版，V0 不引）

`secrecy::SecretString` + `ExposeSecret`：`Debug` / `Display` 恒定输出 `<secret>`，`Drop` 时 `Zeroize` 清零内存，
**取真值必须显式 `.expose_secret()`**。最后这条才是它的核心价值 —— 比"遮住 Debug"强得多：
它让"我把 key 当普通字符串用了"这件事在代码里**看得见**。

V0 一个 key 不值得为它加依赖（本机 registry 也未缓存），但知道有这个成品即可。

#### 别搞错：`Config` 的 `Debug` 和 `OpenAiClient` 的 `Debug` 是两处独立的防线

- `Config` 遮的是"从 `.env` 读上来的那份 key"（§3.3.2 已做）；
- `OpenAiClient` 遮的是"client 持有的那份 key"。

`new(&Config)` 里 `api_key: cfg.llm_api_key.clone()` ⇒ 进程里**同时存在两份**。这没问题
（`Config` 是读取产物、client 是使用方，各有生命周期），**前提是两处 `Debug` 都遮住**。
真正搞错的是把它们塞进 `default_headers`：那会在 reqwest 内部产生**第三份**，而那一份你遮不掉（§3.7.3 实测）。

---

### 3.7.6 `chat()` 收参数列表，还是收一个 `ChatRequest`？（2026-10-20 实测）

三种写法，实测 + 判据：

| | 签名 | 谁填 `model` | 每轮历史 | 加一个 `temperature` 时 |
|---|---|---|---|---|
| **A 参数列表** | `chat(&self, msgs: &[Message], tools: &[WireTool])` | client 内部 | **零拷贝借用** | 改签名（**1 处调用点**，加 `None`） |
| **B 收 owned `ChatRequest`** | `chat(&self, req: &ChatRequest)`，`messages: Vec<Message>` | 调用方或 client（**两个真相**） | **必须 clone** | 零签名变动，但**调用点可能漏填** |
| **C 收借用 `ChatRequest<'a>`** | `chat(&self, req: &ChatRequest<'_>)` | 同上 | 零拷贝 | 同 B |

#### 实测 ①：`Vec<Message>` 与 `&[Message]` 的差别（自定义分配计数器）

20 条消息、每条 `content` 400 字节（产物 8.7 KB）：

```
A 借用 &[Message] 序列化一次 :     7 次分配
B clone 历史 + 序列化       :    29 次分配（clone 22 + 序列化 7）
两者序列化产物相同          : true
```

B 多出来的 22 次 = 1 次 `Vec` + 20 条 `Message` 各自的 `String` + 1 次 `model.clone()`。

> **但别拿这个当主要理由。** 22 次分配 ≈ 亚微秒级，相对一次网络请求（1~3 秒）**完全不可观测**。
> 真正的理由是下一条。

#### 真正的理由：owned `ChatRequest` 会让「历史」出现两个真相

```rust
loop {
    req.messages = messages.clone();     // ← 派生的副本
    let resp = client.chat(&req).await?;
    messages.push(...);                  // ← 真历史
}
```

`req` 持有一份**快照**。任何"忘了同步"的路径（比如执行完工具忘了 push、或 push 到 `req` 而忘了 push 到 `messages`）
都会让**发出去的和以为发出去的差一条**，而模型只会表现得更健忘 —— 这类 bug 你查日志都查不出来（日志里是 `req`，报错时看的是 `messages`）。

这和 §3.4.8（`tools` 两个写入点必有漂移）、`config.rs`（两条取值链）是**同一条**：
**同一个事实只要有两个可写的副本，早晚不一致。**

**借用版从类型上消灭了这种可能** —— 没有副本可写。

#### 实测 ②：`tools` 参数怎么表达"没工具"

对着真实网关发（§3.9 那条通道）：

| 写法 | 结果 |
|---|---|
| 省略 `tools` 键 | **200** |
| `"tools": []` | **200** |
| `"tools": null` | **200** |

三种都接受 ⇒ **不需要 `Option<&[WireTool]>`**，空切片自然序列化成 `[]`。
不必为"没有工具"再造一个状态 —— 空集合已经是它。

#### 推荐形状（V0）

```rust
// 对外：两个参数，无生命周期参数、无 pub 的请求类型
pub async fn chat(&self, messages: &[Message], tools: &[WireTool]) -> anyhow::Result<ChatResponse>;

// 对内：私有的、借用的、只活一次的线格式（§3.4.14 那个 ChatRequest 的真实形态）
#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,                 // ← &self.model，借用，不 clone
    messages: &'a [Message],
    tools: &'a [WireTool],          // ← 不是 Value，理由见下
    tool_choice: &'static str,      // "auto"：V0 的策略，写死在这里
    parallel_tool_calls: bool,      // true：同上
}
```

三件事一起成立：

1. **`'a` 只活在私有 struct 内部，一个字节都没进 pub API。** `chat(&self, msgs: &[Message], ..)` 的
   `&[Message]` 在签名里是省略的生命周期，各参数独立，不需要命名 —— 这就是 §3.4.10 那条"`&'a str` 会污染每个签名"
   的正面版本：**借用要写在"用它的那一层"，不要写在"传它的那一层"。**
2. **`ChatRequest` 不 `pub`。** 它一旦公开，就等于承诺"支持你从任意来源构造请求"（重放、批量、测试桩……），
   而 V0 只需要一处 `POST`。`ChatResponse` / `Choice` / `Usage` 相反 —— 它们是**返回值**，必须 `pub`。
3. **策略字段（`tool_choice` / `parallel_tool_calls`）留在 client 内部。** 它们是"我方怎么用模型"的决定，
   不是"这次请求的数据"。放参数里 = 每个调用点都要重复表态，且没人会去改它。

#### 判据：看"有几类调用方"，不看"结构漂亮"

> **参数列表 = 我承诺支持的边界；`Request` struct = 使用者可以自由指定的一切。**
> 每 `pub` 一个字段，就多一份必须永远正确的契约。

- **只有 1 个调用点（V0 的 loop）** ⇒ 参数列表。自由度没人用，纯负担。
- **多个调用方 / 要暴露成 SDK** ⇒ `Request` struct。行业里 `async-openai` 就是后者：
  `client.chat().create(request).await`，因为它要服务无数使用者，必须允许任意字段。
  **它是 SDK，我们是单一 agent。** 照抄它的形状就是把"通用性"这个负担提前买回来。

#### 什么时候该升级成 `ChatRequest`

任一条件出现才升级，代价很小（调用点只有一处）：

- 出现**第二个调用点**（比如 summarizer / 子 agent 各要一套参数）；
- 某个字段需要**每次请求不同**（例如长历史时切小模型、或按权限切 `tool_choice`）；
- 需要**重放**历史请求（调试 / 回放 / 录制测试）。

升级路径是单向且便宜的：`chat(&self, msgs, tools)` → 内部保留 `WireRequest`，
只是外面多包一层 `chat(&self, req: &ChatRequest)`；`WireRequest` 那层一个字不用改。

#### ⚠️ 若坚持 `pub ChatRequest`：`model` 立刻变成两个真相

`ChatRequest` 一旦 `pub { model: String, messages: Vec<Message>, .. }`，就必然和 client 的
`default_model` 撞车 —— 目前 §3.7.3 已经定了 **model 的权威在请求里**，那 client 顶多存默认值。
于是调用点只有三选一，没有第四个：

| 做法 | 后果 |
|---|---|
| 调用方填 `req.model` | `client.default_model` 变成死字段；两处都能改 ⇒ 迟早不一致 |
| `chat` 内部覆盖 `req.model = self.default_model.clone()` | `req` 里的值**静默失效** —— 传了等于没传，最坏的一种（和 §3.4.13 `user:null` 同类：不报错、行为不对） |
| 删掉 `ChatRequest.model` | 那它就不是"一次请求"，只是"一轮消息 + 工具" ⇒ 名字该叫 `ChatTurn`，且不能独立序列化 |

`tool_choice` / `parallel_tool_calls` 同理：它们写 `String` / `bool` 放进 `pub` 字段，
等于把"我方策略"降级成"使用者可任意指定"，而策略的合法取值是有限的（`"auto"` / `"none"` / `"required"` / 具名）。
**`&'static str` 加枚举**（§3.4.10 那条类型表达意图）在 `pub` 结构里立刻退化成 `String`。

⇒ **结论不变：V0 让 `ChatRequest` 私有、借用、只活一次。** 它公开的收益（外部构造请求）在只有一个 agent 时为零，
代价（model 两处真相 + 策略可被任意指定）是立刻产生的。

#### `tools` 该传 `&[WireTool]`，不是 `&[Value]`、更不是 `&Value`

用户代码里现在同时有两个函数：

```rust
pub fn tools() -> Vec<WireTool>                                         // ← 直接用这个
pub fn tool_json() -> serde_json::Value { to_value(Self::tools()).unwrap() }   // ← 多余的中间层
```

`Vec<WireTool>` **本身就是线格式**（`#[serde(rename = "type")]` 已经把线上字段名钉好了）。
再 `to_value` 一遍，等于**把已类型化的 struct 拆回无类型的树**，然后由 serde 再遍历那棵树写一遍。

实测（3 个工具、5 轮请求、同一份数据）：

| | 产物长度 | 5 轮总分配 | 键序 |
|---|---|---|---|
| **A `&[WireTool]`** | 637 | **62**（构造一次 40 + 5 轮序列化） | `type` 在 `function` 前（struct **声明序**） |
| B `&Value`（`tool_json()`） | 637 | **587**（≈ **105 / 轮**） | **字母序**：`function` 在前、`type` 在后 |
| C `&[Value]`（`.as_array()` 拆） | 637 | 同 B | 同 B |

#### ⚠️ 关键：A 和 B **语义相同但字节不同**

```
A_value == B_value : true      ← 逐项比较相等
A_bytes == B_bytes : false     ← 但字节不同
```

原因是 **`serde_json::Map` 默认后端是 `BTreeMap`**（除非开 `preserve_order` feature 换成 `IndexMap`）。
`to_value` 会把 struct 的字段序**压平并按字母排序**：

```jsonc
// A
{"type": "function", "function": {"name": "read_file", "description": "d", ...}}
// B
{"function": {"description": "d", "name": "read_file", ...}, "type": "function"}
```

网关不关心键序（JSON 对象无序），所以 B **不会报错** —— 它只是白花 105 次分配/轮，
外加一个副作用：**以后从 B 改回 A，`tools` 那段字节会整体变一次**，prompt cache 要重新预热一轮。
（`parameters` 内部两条路径**都是**字母序 —— 因为它本来就是 `json!` 造的 `Value`。
这也印证了 §3.4.5 那条：`Value` 只该留在"开放递归"的地方（`parameters`），
不该留在"能类型化"的地方（`tools` 数组本身）。）

⇒ **删掉 `tool_json()`**（grep 确认目前零调用点），签名收 `&[WireTool]`：

```rust
let tools = ToolKind::tools();      // 循环【外】构造一次：每次 tools() 都会重建 parameters 的 Value（40 次分配）
loop {
    let resp = client.chat(&messages, &tools).await?;
}
```

顺带把 §3.4.8 提过的 `OnceLock<Value>` 优化一并作废：不需要缓存 `Value`，
只需要在循环外构造一次 `Vec<WireTool>`（V1 要按权限 `retain` 过滤，本来也不能全局缓存）。

#### 什么时候 `&Value` 才是对的

**只有一种**：这段数据的"母语"本身就是 JSON，而不是 Rust 类型 —— 比如 V1 接 MCP 的 `tools/list`
（`tools/call` 的 `arguments` 是对象，`tools/list` 回来的 `inputSchema` 是外部 JSON）。
那时候要么加 `#[derive(Deserialize)]` 把它读成 `WireTool`，要么承认这一段就是外部 JSON、直接收 `&Value`。

> 判据：**这段数据是谁定义的？我方 Rust 类型 → 用类型；外部协议给的原样 JSON → 用 `Value`。**

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

#### ✅ 已落地（2026-10-18）：朴素 POST 打通，拿到真实报文

第一版 `main.rs` 踩了三个 bug，全是**报文层**的错，值得留档：

| Bug | 表现 | 原因 |
|---|---|---|
| `.get()` 写成 GET | **404**（不是 405） | `/chat/completions` 只挂 POST；CF 对未匹配方法直接 404，body 为空（`content-length: 0`） |
| `.json(&json_body)` 而 `json_body` 是 `String` | 400 | `.to_string()` 之后再序列化 ⇒ body 成了**带外层引号的 JSON 字符串**，不是对象 |
| 手写 `Content-Type: application.json` | 415 / 400 | 拼错（正确是 `application/json`）；且 `RequestBuilder::header` 是 **append** 而非覆盖，会与 `.json()` 自动添的那条**并存**，服务端取第一个（错的） |

外加**两条 provider 私有要求**（只作接入备注，**不进通用规范**）：

- **`User-Agent`**：reqwest 默认**不发** UA，缺了被 Cloudflare 拦成 `403`（body: `error code: 1010`）。放 `Client::builder().user_agent(...)`。
- **`x-opencode-session`**：缺了 `400 MissingSessionID`（用途是上游路由 + prompt cache，要求一次会话内稳定）。放 `default_headers`，**不写进业务代码**。
  HTTP 允许未知头、服务端一律忽略 ⇒ 换 provider 时**留着也无害**，不构成通用化障碍；要删只是一行。

首次 200 的真实响应（可作为 `model.rs` 的**唯一事实来源**，不要凭记忆写）：

```json
{
  "id": "gw-611de3d6ebaf-4f01e3",
  "object": "chat.completion",
  "created": 1792326206,
  "model": "deepseek-v4.1-flash",
  "choices": [{
    "index": 0,
    "finish_reason": "stop",
    "logprobs": null,
    "message": {
      "role": "assistant",
      "content": "你好！很高兴见到你 …",
      "reasoning_content": "The user just said \"你好。\" …"
    }
  }],
  "usage": {
    "prompt_tokens": 38,
    "completion_tokens": 63,
    "total_tokens": 101,
    "prompt_tokens_details": { "cached_tokens": 0 },
    "completion_tokens_details": { "reasoning_tokens": 27 }
  }
}
```

三条可直接用于类型设计的事实：

1. **`reasoning_content` 与 `content` 同级**（都在 `message` 下），不是嵌套对象 —— 之前"thinking 内容放在哪"的猜测到此为止；
2. **thinking 默认开启**：只回两个字也产生 27 个 reasoning token，`usage.completion_tokens_details.reasoning_tokens` 是唯一的量化出口；
3. **`choices` 是数组**（本次单元素）—— 解析不能假设"取第 0 个就完事"，尽管实践中 OpenAI 兼容层基本恒为单元素。

⇒ **下一步（实验 2）**：加一个 dummy `tools` 声明再发一次。**要看的清单**（§3.4.1 给了"应该是什么"）：

1. `finish_reason` 是否变 `"tool_calls"`
2. `tool_calls[].function.arguments` 是**字符串还是对象**
3. `content` 是否为 `null`；`reasoning_content` 还在不在（与 `content` 同级）
4. 该通道**接不接受** `tool_choice` / `strict`（不支持时是 400 还是静默忽略）
5. **回灌一轮**：assistant 原样 + `role:"tool"` 各一条 → 是否 200，且模型改用正文回答
   （第 5 条才是"循环能转"的最小验证；1–4 只是形状）

**看完再写 `model.rs`**（届时才决定要不要 `deny_unknown_fields`）。

> 提示：题面必须**只能用工具完成**（例如声明 `read_file(path)` 后问"读 src/main.rs 并告诉我行数"），
> 否则模型仍可能只回文本 → `tool_calls` 为空，实验白跑。
> 另一条通道备忘：DeepSeek 官方 API 现在把模型名写作 `deepseek-flash`（服务 V4.1-Flash-1017），
> 本项目走的是 OpenCode 网关的 `deepseek-v4.1-flash`，两者**不可混用**。

#### ✅ 实验 2 已落地（2026-10-19）：拿到真实 `tool_calls` 报文

`model.rs` 的**第二份事实来源**（第一份是上面的纯文本响应）：

```json
{
  "id": "gw-a70a2abc01d3-4b1f50", "object": "chat.completion", "created": 1792391459,
  "model": "deepseek-v4.1-flash",
  "choices": [{
    "index": 0,
    "finish_reason": "tool_calls",
    "message": {
      "role": "assistant",
      "content": null,
      "reasoning_content": "The user wants me to read the scratch/readme.md file. Let me do that.",
      "tool_calls": [{
        "id": "call_7c4d7ca56b784e7da63c67ee",
        "type": "function",
        "function": { "name": "read_file", "arguments": "{\"path\":\"scratch/readme.md\"}" }
      }]
    }
  }],
  "usage": { "prompt_tokens": 308, "completion_tokens": 63, "total_tokens": 371,
    "prompt_tokens_details": { "cached_tokens": 256 },
    "completion_tokens_details": { "reasoning_tokens": 19 } }
}
```

**清单核对**

| # | 要看 | 实测 | 结论 |
|---|---|---|---|
| 1 | `finish_reason` | `"tool_calls"`，且与 `tool_calls` **同时出现** | 与 §3.4.1 一致 |
| 2 | `arguments` 类型 | **字符串** `"{\"path\":\"…\"}"` | 必须二次 `from_str` |
| 3 | `content` / `reasoning_content` | `content: null`（键在、值为 null），`reasoning_content` 仍在 | 两个都得 `Option<String>` |
| 4 | `tool_choice` / `strict` | 本次报文手写，**未带** | 仍未验证 |
| 5 | 回灌一轮 | **未做** | ⬅ 这才是循环能否转的判据 |

**五条只有见过真实报文才知道的设计事实**

1. **`id` 是 `call_` + 32 位小写 hex**（网关生成）⇒ 当**不透明 `String`**，不校验格式、不做前缀假设。
2. **非流式响应里 `tool_calls[]` 没有 `index`** —— 那是**流式 delta 专用**字段。别把它写进非流式的 `ToolCall`。
3. **`arguments` 必须存原始 `String`**。一旦解析成 `Value` 就拼不回原样（键序、空白都变）；而 assistant 消息要
   **原样回灌**，且 `cached_tokens: 256` 证明 prompt cache 真的在命中。
   ⚠️ **更正（同日）**：cache 命中与否取决于**消息内容 / 条数 / 顺序**，**不是 JSON 字节** —— 服务端解析后按
   chat template 重新渲染成 token 序列，对象里的键序到此已不存在。所以"原样"= **语义原样**（字段不丢、值不改、
   顺序不变），不是"逐字节一致"。存 `String` 的真实理由是**不必重建**：你手里只有原文时，重序列化只是多此一举。
4. **`cached_tokens` 是 `x-opencode-session` 稳定的直接回报**（308 里 256 命中）⇒ **历史顺序必须稳定**，
   别每轮重建时打乱；这也说明"消息历史"该是有序 `Vec`，不是 `HashMap`/`HashSet`。
5. **同一个网关，两次响应的字段集不一样**：第一次有 `"logprobs": null`，这次整条缺失
   ⇒ **外层响应结构绝对不能 `deny_unknown_fields`**（这条原来还只是"怕以后加字段"，现在有实证了）。

**`model.rs` 的 6 个决定点**（形状已定，代码自己写）：

| 字段 | 建议类型 | 理由 |
|---|---|---|
| `content` | `Option<String>`（**`default` 可省**，见 §3.4.7） | 显式 `null` 与缺键都吃下 —— 但这两件事由**两个不同机制**负责，`Option` 自己就够 |
| `reasoning_content` | `Option<String>`，**可以回灌** | 实验 2.5 已带上且 200（网关不拒）；"原样复制"最省事。注意它**不是**协议字段，换 provider 后可能消失/无意义 |
| `tool_calls` | **`Vec<ToolCall>` + `#[serde(default)]`**（**不用 `Option<Vec<_>>`**） | `Option<Vec>` 有 4 态（缺/`null`/空/非空），语义只有 2 态 ⇒ 让**非法状态不可表达**（§3.8 第 1 问的答案就在这）。⚠️ `default` 只吃**缺键**，吃不下 `null`（§3.4.7 实测） |
| `arguments` | `String`，执行前才 `from_str` | 原样保留（见事实 3） |
| `finish_reason` | `Option<String>`，或 enum + `#[serde(other)]` 兜底 | 别穷尽，provider 会加新值 |
| `deny_unknown_fields` | **禁用** | 见事实 5 |

⇒ **下一步不是写 `model.rs`，是补做清单第 5 条**（实验 2.5）：把 assistant 那条**原样**塞回 + 一条
`{"role":"tool","tool_call_id":"call_7c4d…","content":"…"}` 再发一次，看是否 200 且模型改用正文收尾。
**这一步过了，循环的最小闭环才算被验证。**

#### ✅ 实验 2.5 已落地（2026-10-19）：闭环成立

第二轮返回 **200**、`finish_reason: "stop"`、`content` 变成正文（还按内容做了总结）。**"发送 → 收调用 → 回灌 → 收尾"整条链路打通。**

四条新事实（**修正/强化了上面的判断**）：

1. **带 `tool_calls` 时 `content` 也可以是正文，不是恒 `null`。** 本轮 `content = "我来阅读这个文件。"` —— 模型先说了句话再调工具。
   ⇒ ① `Option<String>` 的判断不变，但**别假设"有 tool_calls 就 content 为 null"**；
   ② 这段 preamble 正好可以直接打给用户看（CLI 的进度提示免费拿到了）。
2. **收官那轮的 `message` **完全没有 `tool_calls` 键**（不是 `null`、不是 `[]`）** ⇒ 这是 `Vec<ToolCall>` + `#[serde(default)]`
   的**直接实证**：`Option<Vec<_>>` 遇到"键不存在"还要多一条 `unwrap_or_default()`。
3. **`strict: true` / `tool_choice: "auto"` / `parallel_tool_calls: true` 全被接受（200）** —— 清单第 4 条答了一半：
   网关**不拒绝**它们。但"接受"≠"强制约束"，`strict` 是否真按 schema 约束仍未验证（V0 不依赖它）。
4. **cache 跨轮稳定**：`prompt_tokens` 308 → 387（+79，正好是 assistant+tool 两条），`cached_tokens` 仍是 **256**。
   ⇒ **"只追加、不改写"** 这个约束得到验证：往尾部追加不影响前缀缓存，只有动老消息才会打掉。

**当前 `main.rs` 的定位**：它是**协议探针**，不是 agent。两处"假"的地方要清楚：
- 工具结果是**硬编码**的 `"## 你好\n\n你好\n"`，并没有真读文件（模型"读到"的内容是假的）；
- 只有**两轮**，写死的顺序，没有循环、没有终止条件、没有 `max_steps`。

⇒ **下一个动作 = 写 `model.rs`**（6 个决定点已定），然后才是 `llm.rs` + 真正的循环。

---

## 五、进度记录（V0）

- [x] `.env.example` 入库（三变量空壳）+ `.gitignore` 加 `.env` / `.env.local` / `scratch/`（§3.3.1）
- [x] `Config` / `ConfigError` 成型，`dotenvy` 读三变量（§3.3.1 / §3.3.2，**五版迭代**；`cargo check` 零代码告警）
- [x] **收敛 `from_env` / `from_map` 两条链**（§3.3.2 → 方案 **C**，已落地）
- [ ] `config.rs` 的 `#[cfg(test)] mod tests`：三变体 + `format_url` 边界（§3.3.2 末 4 问）
- [x] 确定 API 的 `base_url` + `model`（OpenCode Go / `deepseek-v4.1-flash`，只填 `.env`，代码不动）
- [x] 建 `.env` 本体，填入真实三值
- [x] 发一个最朴素的 chat 请求，打印原始 JSON 响应（§3.9，**200**）
- [x] 摸清该通道的两条私有 header：`User-Agent` / `x-opencode-session`（§3.9）
- [x] **实验 2**：带 dummy `tools` 跑通一次，看清 `tool_calls` 形状（§3.9，**2026-10-19 拿到 `finish_reason:"tool_calls"`**）
- [x] **实验 2.5（回灌）**：assistant 原样 + `role:"tool"` 回灌一轮 → 200 且模型改用正文（§3.9，**2026-10-19 闭环成立**）
- [x] `Message` 类型的决定落定（§3.2 ① → **§3.2.2 判据 + `model.rs` 已起草 tag enum**）
- [x] 工具参数类型的决定落定（§3.2 ② → **§3.2.2 / §3.4.4：`Invocation` enum，已起草**）
- [x] `ToolCall::payload() -> Result<&FunctionCall, ToolError>` 收敛未知 `type`（§3.4.6；原计划的 `to_invocation` 已拆成 `payload()` + `parse_kind`，组合点在循环里，2026-10-20）
- [x] `ToolCall.function` 改 `Option` + `payload()` 收敛（未知 `type` 要能报出类型名，§3.4.6）—— 实测 `custom` 缺键时**无需 `#[serde(default)]`** 也得到 `None`（serde 对 `Option` 字段缺键隐式 `None`）
- [x] **请求侧 `tools` 声明**：`FunctionDecl` + `ToolKind::ALL` + `spec()` + `tools() -> Vec<WireTool>`（**§3.4.8 / §3.4.10**，`&'static str` 已采用；`tool_json()` 待删，见 §3.7.6）
- [x] 删掉 `Invocation::to_functioncall` / `from_functioncall`，改 `parse_kind(kind: &ToolKind, args: &str) -> Result<Invocation, ToolError>`（**§3.4.9** 规则 1/2；`Option` 会丢错误信息）
- [ ] `write_file` 补 `content` 参数（`Invocation` 同步加字段）；`run_command` 的 `cmd` 等白名单设计定案再定（**§3.4.10** 末表）
- [ ] `ToolKind::ALL` 防漂移测试：长度 + 名字唯一 + `from_name` 往返（§3.4.8 实测 ③ / §3.4.12 漏洞 ②；V1 换 `trait Tool` 后此测试可删）
- [ ] `from_name` 改成 `Self::ALL.iter().copied().find(|t| t.name() == n)`，删掉手写 `match`（**§3.4.12 漏洞 ①**：消掉工具名第二份 + 消掉「`from_name` 认识但 `ALL` 不宣告」的矛盾态）
- [x] 每个 `Invocation` 变体装箱 `XxxArgs`（`#[serde(deny_unknown_fields)]`），`parse` 用 `from_args::<T>`（**§3.4.11 / §3.4.12**）
- [x] **`content` 收紧**：`System` / `User` / `Tool` 改 `String`，`Assistant` 留 `Option<String>`（**§3.4.13** 实测：`user` 的 `null` 会让模型幻觉出问题且不报错；`tool` 的 `null` 直接 400）
- [x] `parse_kind` 改一步 `from_str::<T>(args)`，删 `ToolError::BadParse`（**§3.4.13** 末：两步解析丢 `line/column`）
- [x] `ChatResponse` / `Choice`（**`pub`**）+ **私有** `WireRequest<'a>`（**§3.4.14 / §3.7.6**：`Choice.message` 直接用 `Message`，`finish_reason` 不参与判断；请求类型**不 `pub`**、`messages: &'a [Message]` 借用不 clone）
- [ ] `ChatResponse` 补 `usage: Option<Usage>`（只为看 `prompt_tokens_details.cached_tokens`；2026-10-20 实测 `cached_tokens: 384 / prompt_tokens: 420`，V1 上下文管理要用）
- [x] `llm.rs`：`OpencodeGoClient { session_id, chat_url, default_model, api_key, client }` + `new(&Config)` + `async fn chat(&self, msgs: &[Message], tools: &[WireTool]) -> anyhow::Result<ChatResponse>`，内部构造私有 `WireRequest<'a>`（**先具体实现，不写 trait**）—— **2026-10-20 真实 200 跑通**
- [x] **删 `ToolKind::tool_json()`**（零调用点），`chat` 的 `tools` 收 `&[WireTool]`；`let tools = ToolKind::tools()` 放**循环外**构造一次（**§3.7.6** 实测：`&Value` 路径每轮多 105 次分配，且 `to_value` 会把键按字母序重排 ⇒ 与 struct 声明序字节不同）
- [x] `OpencodeGoClient` **手写 `Debug`**：`api_key` 打 `"***"`、`client` 打 `"<reqwest::Client>"`（**§3.7.3 / §3.7.5** 实测：`reqwest::Client` 的 `Debug` 会明文打出 `authorization: Bearer sk-...`）
- [x] api_key **走 `.bearer_auth()` 每请求设置**，**不要**塞进 `default_headers`（**§3.7.3** 实测：`bearer_auth` 自动 `set_sensitive`，其余三种写法有两种明文泄漏）
- [ ] **`chat()` 里删掉 `serde_json::to_value(chat_request)?`**，直接 `.json(&chat_request)`：现在跑的是「结构体 → `Value` → 再序列化」两趟，`to_value` 还会经 `BTreeMap` 把键按字母序重排
- [ ] **`chat()` 先判状态码再 `text()`**：现在 4xx 会退化成 serde 的 `expected value at line 1 column 1`，body 里的真实报错丢了 ⇒ `if !status.is_success()` 时把 `status` + `body` 一起塞进 `anyhow` 错误
- [ ] 删 `model.rs` 的 `pub struct ChatRequest`（零调用点，与 `llm.rs::WireRequest` 构成「两个真相」）+ 删 `main.rs` 的 `test_chat` / `get_chat`（协议探针使命已完成，现在是 dead code）
- [ ] 循环接上：`tool_calls` 非空 → 执行 → 回灌 → 继续（**§3.7.2** 六分支表；`client` 从参数进来、返回值不出现 `reqwest` 类型）
- [ ] 终止条件 + 步骤上限（防死循环；用 `for step in 0..max_steps`）
- [ ] `LlmClient` trait 立起来 —— **排在循环跑通之后**，签名从实际代码抄（§3.7.2）
- [ ] 4 个工具实现：`list_files` / `read_file` / `write_file` / `run_command`
- [ ] 安全边界落地：scratch 目录 + 写/shell 确认 + 命令白名单 + 输出截断
- [ ] 自答 §3.8 的 4 个问题
- [ ] **验收**：拿 `my_api` 副本跑通一次真实修复（§3.6）
