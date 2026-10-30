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
> ⚠️ **2026-10-30 改判：V0 改手写 `Logger`**（见 §3.6.6）。`tracing` 解决的正是「库发事件、subscriber 决定去向」，
> 方向没错，但现在只有 1 个模块要打日志、span / 字段过滤一个都用不上 ⇒ 收益 0，成本 2 个依赖 + `#[instrument]` 遍历。
> **触发条件**：出现第二个要打日志的模块，或需要按字段过滤（如只看 `tool=read_file`）。
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

#### 工具实现层：三条横切关注点应该只有一处（2026-10-21，评审 `read_file`/`list_files` 后）

`read_file` / `list_files` / `write_file` / `run_command` 里**每个都要做**三件事：
**① 路径沙箱检查 ② 输出截断 ③ 分派**。如果写在各自的函数里，加第 4 个工具时忘一次就漏一次
（同 §3.4.8「`tools` 单一来源」那条道理：**能靠结构保证的，别靠记性**）。

推荐：把它们收进**一个** `impl Invocation { pub fn invoke(&self) -> anyhow::Result<String> }`，
**写在 `tool.rs`（执行层），不能写回 `model.rs`**：

- **依赖方向**：执行层可以认识协议类型，协议层不该认识执行 —— 同 §3.4.9「`Invocation::parse` 而不是 `ToolKind::parse`」。
- **所有权**：`match &self` 拿到 `&ReadFileArg`，`&str` 直接可用，**一次 move 都不需要**
  （正是 §3.7.2 那个 `Turn<'a>` 借用问题的正解：执行只需要借，不需要拥有）。
- **不加 `async`**：V0 的四个工具全是阻塞 IO，CLI 里只有一个任务，占一个 worker 无所谓；
  V1 要并发调工具时再换 `tokio::fs` / `spawn_blocking`。

⚠️ **工具输出必须确定性（实测 2026-10-21）**：`fs::read_dir` **不保证任何顺序**，NTFS 按**不区分大小写**的
索引序返回：

```
read_dir 原始序 : ["apple.txt", "Beta.txt", "Zebra.txt", "_x.txt", "中文.txt"]
sort() 之后     : ["Beta.txt", "Zebra.txt", "_x.txt", "apple.txt", "中文.txt"]
两者相同吗      : false
```

Linux ext4 是 hash 序，跨机器更不一致。后果不是"看着乱"，而是**同一目录两次运行给模型不同的 context**
⇒ 回答不稳定 + prompt cache 命中率掉（MCP 硬要求 `tools/list` 确定性顺序，理由是同一个）。加一行 `sort()`
即可；**要的不是"符合人类排序"，是"每次一样"**。同理 `Path::to_str()` 失败时若 `if let` 静默跳过，
那个文件在列表里凭空消失（而 `read_file` 对同一路径会报错）。

**非 UTF-8 路径怎么办（2026-10-21 实测）**：三种转换不是"好/坏"，是失败语义不同：

| 调用 | 输入 | 输出 | 失败时 | 丢信息 |
|---|---|---|---|---|
| `OsStr::to_str()` | `&OsStr` | `Option<&str>` | `None` | 不丢（值还在） |
| `OsStr::to_string_lossy()` | `&OsStr` | `Cow<str>`（合法时 `Borrowed`，零分配） | 替换成 `U+FFFD` | **丢了，不可逆** |
| `OsString::into_string()` | **`OsString`（消耗）** | `Result<String, OsString>` | `Err` 里**原值还给你** | 不丢 |

实测（Windows，孤立代理 `0xD800`）：`OsStr` 内部是 **WTF-8**，非法码元存成 3 字节 `ED A0 80`；
`to_string_lossy` 把它换成 `EF BF BD`（`U+FFFD`）⇒ 字节不同、**原路径再也打不开**；
`into_string()` 的 `Err` 分支仍持有原值（`as_encoded_bytes() == 原始字节`，逐字节相同）。

⇒ `list_files` 里 `entry.path()` 本来就是 **owned 的 `PathBuf`**，走 `.into_os_string().into_string()`
不额外花钱且**无损**；`Err` 再降级成 lossy 并**显式标注**（如 `… （非 UTF-8，可能打不开）`）。
**核心要求不是选哪个，是"不许静默"** —— 对模型来说，"有个文件但名字读不出来" 和 "没有这个文件" 是两条完全不同的信息。

#### 决策：先打通循环，工具层只保留「一处分派」（2026-10-22）

用户问：*"工具方面先这样，先考虑打通的问题？现在这两个 Tool 也并没有修改的能力所以应该不会造成危害"*。

**结论：同意先打通，但"没危害"这个理由要换掉** —— 它成立得不完整，有两条真实代价在打通第一天就会撞上：

| 直觉中的风险 | 真实情况 |
|---|---|
| 「只读工具不会改坏文件」 | ✅ 成立。V0 的失败模式是**浪费一次请求**，不是数据丢失 |
| 「所以可以不管安全边界」 | ❌ **`read_file(".env")` 是通的**（CWD 就是 `my_agent`）⇒ key 进了 `messages`、进了每一次请求、进了你贴出来的日志。**这不是"改坏"，是泄漏**，而且不会报错 |
| 「所以也不急着截断」 | ❌ `plan_v0.md` 现在 2600+ 行 ≈ 100 KB ≈ 3 万 token。模型一次 `read_file("plan_v0.md")` 就吃掉大半上下文 ⇒ 后续 400 或答案质量塌陷 |

但**代价极小**：这两件加起来 ≈ 20 行，而且 `truncates` 已经写好了。所以顺序是
**「写 `invoke`（沙箱 + 分派 + 截断三行）→ 立刻写循环」**，不是「先跳过它们」。

**真正可以推后的是这些**（都不影响"打通"这件事本身）：
`write_file` / `run_command` 的**人工确认**（工具都还没实现）· `run_command` 的 head+tail 策略 ·
错误分类细化 · `trait Tool` 抽象 · 每工具独立上限 · 精确到行的截断报告。

🔴 **现有 `tool.rs::truncates` 有一个必 panic 的分支（2026-10-22 实测）**：
```rust
let mut end = max_bytes;
if let Some(pos) = text.as_bytes()[..end].iter().rposition(|&b| b == b'\n') {
    end = pos + 1;          // 找到换行：end 是 ASCII 位置，永远合法
}
text[..end].to_string()     // ← 没找到换行时 end 仍是 max_bytes，可能落在字符内部
```

```
[有换行]        原 10892 字节 -> 8186 字节, 以换行结尾=true
[无换行中文]    PANIC: end byte index 8192 is not a char boundary;
                       it is inside '中' (bytes 8190..8193 of string)
[8190 是 3 的倍数] OK, 8190 字节      ← 陷阱：换个上限值就"看起来没问题"
```

触发条件 = **整份文件没有 `\n`（或第一行就超 8KB）+ 中文/emoji**。真实场景：压缩后的 JSON、minified JS、
单个巨大 base64。最后一行实测很关键 —— **它证明这个 bug 会随上限数值时隐时现**，所以"我手动试过一次没崩"不算验证。
修法就是补回退：

```rust
while !text.is_char_boundary(end) { end -= 1; }   // 最多退 3 次，0 必然是边界
```

另外两条顺手要改的（不阻塞打通，但一起做更省事）：

- **截断结果要自解释**：`…（已截断，共 1240 行 / 96 KB）`。同"空目录"那条道理 —— 模型会把"只有 8KB"当成"文件就这么长"。
- **签名改成吃 owned `String`**：`fn truncate(mut s: String, max_bytes: usize) -> String`，
  算出 `end` 后 `s.truncate(end)` **原地零分配**；现在 `&str -> String` 等于把前 8KB 再整串拷一遍。
  （`raw_invoke()` 返回的本来就是 owned，天然匹配。）

#### `invoke` / `raw_invoke` 都写成 `impl Invocation` 的方法？（2026-10-22，实测）

问：横切层和分派层是不是都该挂在 `impl Invocation` 上。**答：位置可以，但可见性必须不同 —— 这是唯一有编译器保证的一条。**

```rust
impl Invocation {                      // 写在 tool.rs
    pub fn invoke(&self, sandbox: &Path, max_bytes: usize) -> anyhow::Result<String> { .. }
    fn raw_invoke(&self) -> anyhow::Result<String> { .. }     // ← 不带 pub
}
```

实测（`.probe_impl/`，已删）：

| `raw_invoke` 的可见性 | `agent.rs` 里调 `i.raw_invoke()` |
|---|---|
| `fn`（私有） | `error[E0624]: method 'raw_invoke' is private` ✅ |
| `pub(crate)` | **编译通过** ❌ 沙箱形同虚设 |

⇒ **`raw_invoke` 绝不能 `pub(crate)`**。它不是"内部辅助函数"，它是**绕过所有横切约束的后门**；`pub(crate)` 在别处看起来无害，在这里等于把沙箱拆了。

**跨模块 inherent impl 合法**（`Invocation` 定义在 `model.rs`、实现写在 `tool.rs`，同 crate 任意模块都行，可分散成多个 `impl` 块，已实测）。代价：在 `model.rs` 读 enum 定义时看不到它有哪些方法，跳转定义要跳两次。这个代价正是"依赖方向"要付的 —— 反过来把 `invoke` 写回 `model.rs` 才是真错（协议层认识执行层）。

**但"能不能挂方法"和"该不该"是两件事，判据是这个层有没有外部状态**：

| 层 | 依赖 | 形状 |
|---|---|---|
| 分派层 `raw_invoke` | 只有 `self`（`match &self` 就知道调谁） | **天生的方法** |
| 横切层 `invoke` | `self` + `sandbox` + `max_bytes`（**从外面传进来的执行策略**） | 方法可以，但已经有点勉强 |

横切层的参数是"执行策略"，不是 `Invocation` 的固有属性 —— 同一个 `Invocation` 在不同 sandbox 下合法/非法。**一旦参数涨到第三个（`allow_write` / `dry_run` / 人工确认），就该收成一个 `Executor`/`Policy` struct**，那时 `invoke` 搬到它身上：

```rust
struct Executor<'a> { sandbox: &'a Path, max_bytes: usize }
impl Executor<'_> { fn run(&self, inv: &Invocation) -> anyhow::Result<String> { .. } }
```

V0 两个参数、一个调用点，为它多一个 struct 不值（同 §3.2.3：闭合集合、四个工具，别提前上 trait/装饰器）。

**沙箱检查需要 per-variant 的"这次会碰哪些路径"**，那是各变体自己的知识，所以 `match self` 要写在横切层里（`check_paths` 第一行），漏臂编译器报 E0004。**别给 `Invocation` 加 `fn path(&self) -> &str` 这种统一 getter** —— 它假设"每个工具恰好只有一个待读写路径"，而 `run_command` 有 cwd 却没有文件；`WriteFile` 还要检查**父目录**（`canonicalize` 要求路径存在，见下）。

#### 路径用相对还是绝对？沙箱检查用哪个（2026-10-22，实测）

**两层不同形式，转换只发生在 `invoke` 一处**：

| 层 | 形式 | 为什么 |
|---|---|---|
| **接口层**（模型看到、模型传的） | **相对沙箱根** | 闭环 + 可重放 |
| **检查层**（沙箱内部） | **绝对 + `canonicalize` 过** | 唯一能可靠比较的形式 |

接口层用相对的三个理由：

1. **闭环自洽**：`list_files(".")` 的输出就是模型能原样回传给 `read_file` 的路径。若输出绝对路径，模型要抄一长串 `D:\WorkSpace\rust\rust\web_learn\my_agent\scratch\...`，抄错、浪费 token，而且下一次 `list_files` 又给一遍。
2. **历史可重放**：context 里一旦出现宿主机绝对路径，换目录/换机器后同一段历史就不可重放，prompt cache 也不同。
3. **少泄漏环境**（用户名目录等）。

⚠️ **但不能靠"拒绝绝对路径"来保证安全**。安全只能靠 `canonicalize + starts_with`。V0 建议**接受**绝对路径、但必须落在沙箱内 —— 模型有时会从系统提示/错误信息里学到绝对路径，直接拒会让它陷入试错循环。

**检查层四条实测铁律**（`.probe_path/`，已删，Windows）：

```
canonicalize("sb")            = "\\?\D:\...\sb"          ← ① 带 verbatim 前缀
raw_abs                       = "D:\...\sb"
raw_abs == sandbox            = false
sandbox.starts_with(raw_abs)  = false                    ← ② 一边 canon 一边不 canon 必失败
```

① **`\\?\` 前缀是 Windows 上 `canonicalize` 的固定产物**，对 `read_to_string` 透明（实测能直接读）。别去 strip 它，两边都带就一致。
② **必须两边都 `canonicalize`**。混着比一定 `false` —— 这是最阴的坑，因为代码看起来完全正确。
③ **`Path::starts_with` 逐字节比较、不折叠大小写**：`D:\WORKSPACE\...`.starts_with(`D:\WorkSpace\...`) = `false`。但 canon 会解析并归一成**真实大小写**（实测 `canon("SB/Sub/../A.TXT")` → `...\sb\a.txt`），所以两边都 canon 就一致。
④ **组件比较没有字符串前缀陷阱**：`sb2`.starts_with(`sb`) = `false`（`/a/b` vs `/a/bc` 那条老坑在这里天然不成立，因为 `Path` 按分量比）。

🔴 **`Path::join` 遇到绝对路径会静默替换整个路径**（Rust 既定语义）：

```
Path::new("sb").join("D:\\Windows")     = "D:\\Windows"      ← 绝对路径 → 直接替换
Path::new("sb").join("\\Windows")       = "\\Windows"        ← 根相对 → 也替换
Path::new("sb").join("../../outside.txt")= "sb\\../../outside.txt"   ← .. 保留，canon 时才解
Path::new("sb").join(".")               = "sb\\."            ← 所以 list_files(".") 会带 "./" 前缀
```

⇒ **`sandbox.join(model_path)` 本身不提供任何保护**。顺序必须是 **join → canon → starts_with**，一步都不能省。

**分工具策略**（`canonicalize` 要求路径存在）：

| 工具 | 目标存在？ | 检查方式 |
|---|---|---|
| `read_file` / `list_files` | 必须存在 | target `canon` → `starts_with(root)` |
| `write_file` | **可能不存在** | **父目录 `canon` → `starts_with`**，文件名单独校验（不含分隔符、不是 `.`/`..`） |
| `run_command` | 不管路径 | 命令白名单，另说 |

**`list_files` 的输出天然是相对路径**：`entry.path()` = `join(传入的 path, name)`，传入的是沙箱相对 ⇒ 输出也是沙箱相对，模型可直接回传 ✅。（`list_files(".")` 会产出 `./a.txt`，剥掉 `./` 前缀更干净。）

**由此改判：`Executor` 现在就该建**（推翻上一轮"两个参数不值一个 struct"）。理由换了 —— 不是"参数个数涨了"，而是 **`root` 必须是"已 canon 的"这条不变量需要一个载体**：

```rust
struct Executor { root: PathBuf, max_bytes: usize }   // root 在 new() 里 canon 一次
impl Executor {
    fn run(&self, inv: &Invocation) -> anyhow::Result<String> {
        // ① 校验（match self 在 Invocation 侧，见上节）
        // ② inv.raw_invoke()  —— 私有分派，仍在 impl Invocation
        // ③ truncate(raw, self.max_bytes)
    }
}
```

`invoke(&self, sandbox: &Path)` 收裸 `&Path` 的话，"它已 canon 吗"全靠调用方记性 ⇒ 和 §3.5 的主题（能靠结构保证的别靠记性）直接冲突。这也顺带解决了"参数迟早涨到三个"。

`raw_invoke` 依然留在 `impl Invocation`（私有，见上节），横切层归 `Executor` —— 两层的归属本来就不同，只是 V0 之前凑在一个方法里。

#### 🔴 校验函数必须**返回**要用的路径，不能返回 `()`（2026-10-22，用户质疑"是否会修改参数"）

用户问：对 target 做 canon 再 `starts_with`，"是否会修改参数？"**—— 问到点子上了：`&self` 改不了参数，但这恰恰是危险的来源。**

先回答字面问题：

- **不修改 `Invocation` 里的字段**：`raw_invoke(&self, ..)` 是 `&self`，本来也改不了；而且 `Invocation` 是协议层数据，执行层不该回写（同 §3.4.4「执行层只在执行那一刻存在」）。
- **但必须让实际 IO 使用 canon 之后那个值**。canon 结果作为**局部变量 / 返回值**流动，不落回数据结构。

**如果写成"先统一预检、再分派执行"，就是经典的"检查 A、使用 B"**：

```rust
self.check_paths(root)?;                     // 检查 root/a.txt → 通过
match self { Invocation::ReadFile(a) => read_file(&a.path) }   // ← 用的是 "a.txt"，相对 CWD 解析
```

实测（`.probe_chk/`，已删：CWD 放一份 `a.txt`、`sb/` 里放另一份内容不同的 `a.txt`）：

```
检查的路径 = \\?\D:\...\.probe_chk\sb\a.txt
实际读的   = \\?\D:\...\.probe_chk\a.txt          ← 相对 CWD，不是沙箱
是同一个吗 = false
read_to_string(模型给的 "a.txt") = Ok("CWD 里的 a.txt")
read_to_string(检查过的路径)     = Ok("沙箱里的 a.txt")
```

**这不是理论问题**：V0 里 CWD = `my_agent`，root = `my_agent/scratch`，两者天然不同 ⇒ 每一个相对路径都会读错文件，且**沙箱检查全部通过**（检查的是 scratch 里的，读的是 CWD 里的）。更糟的是这个 bug 在"CWD 恰好等于 root"的测试里永远看不见。

⇒ **定案：`check_paths(&self) -> Result<()>` 这种预检式写法不要用**，改成**返回式**：

```rust
fn resolve_in(root: &Path, user_path: &str) -> anyhow::Result<PathBuf> {
    let target = if Path::new(user_path).is_absolute() { user_path.into() } else { root.join(user_path) };
    let canon = std::fs::canonicalize(&target)?;      // 解 ..、符号链接、大小写归一、加 \\?\ 前缀
    if !canon.starts_with(root) { bail!("路径越界: {user_path}"); }
    Ok(canon)                                          // ← 返回的就是要用的值
}

impl Invocation {
    fn raw_invoke(&self, root: &Path) -> anyhow::Result<String> {
        match self {
            Invocation::ReadFile(a) => { let p = resolve_in(root, &a.path)?; read_file(&p) }
            Invocation::ListFiles(a) => { let p = resolve_in(root, &a.path)?; list_files(&p) }
        }
    }
}
```

**判据（可复用到任何安全场景）：校验函数的输出必须是后续真正被使用的那个值，而不是一个 `bool`/`()`。** 返回 `()` 的 `check_*` 把"再拼一次路径"的机会留给调用方，而那次拼接就是漏洞本身。

顺带两条：

- **`read_file` / `list_files` 建议改成收 `&Path`**：`resolve_in` 返回 `PathBuf`，`&str` 还得 `to_str()` 多转一次；而且 `&Path` 能表示**非 UTF-8 路径**（之前"非 UTF-8 名字永远打不开"的限制就此解除 —— 模型侧仍是 `String`，但链路内部不再人为加一道限制）。
- **`list_files` 不需要 `root` 参数，也不需要 `strip_prefix`**：`DirEntry::path()` 的定义就是 `dir.join(file_name())`（std 文档原话），所以 **`entry.file_name()` 已经就是"剥掉 `dir` 那一段"的结果**，与 `full.strip_prefix(&dir)` **逐项相等**（2026-10-22 实测）。
  即：`root` 只该出现在 `raw_invoke`（做 `resolve_in`，安全职责），不该出现在 `list_files`（做 IO）。
  ```
  path()       = "\\?\D:\...\sb\a.txt"
  file_name()  = "a.txt"
  strip_prefix = "a.txt"          两者相等 = true
  path()       = "\\?\D:\...\sb\sub"
  file_name()  = "sub"            是目录 = true     ← entry.file_type() 免费给出类型
  ```
  顺带：**`entry.file_type()` 能区分目录/文件且不需要额外 `stat`**（Unix 上 dirent 自带 `d_type`，未知时 std 才 fallback）。输出里给目录加 `/` 后缀，模型就不会对着目录调 `read_file` 白烧一轮。参考实现 MCP filesystem 的 `list_directory` 正是这个形状：`[DIR] name` / `[FILE] name`，**只有名字，没有全路径**。
- `Executor` 就退化成 `{ root, max_bytes }` + 转调，`run` 里只剩 `raw_invoke` → `truncate` 两行。这反而更干净：**横切层是"包一层"，校验层是"每臂自己 resolve"**，因为路径是 per-variant 的知识。

#### 参数归属：一个参数只服务一件事（2026-10-22）

| 函数 | 收什么 | 为什么 |
|---|---|---|
| `Invocation::raw_invoke(&self, root)` | **root** | 每臂先 `resolve_in(root, &args.path)?` |
| `read_file(&Path)` | resolve 后的绝对路径 | 纯 IO，不认识沙箱 |
| `list_files(&Path)` | resolve 后的目录 | 纯 IO；条目名靠 `entry.file_name()` |

判据：**收 `root` 的那个函数必须是"做安全检查"的，不能是"做 IO"的**。写成 `list_files(root, dir)` 等于让 IO 函数知道沙箱边界，代价有两个：① 加第二个调用点（不带沙箱的单测）就得伪造一个 root；② 两个参数里的 `dir` 必须是 `root` 派生出来的，这个不变量没人强制。而 `file_name()` 把它变成"结构上不需要"。

**实测：`entry.path()` vs `entry.file_name()` 的输出差（2026-10-22，`scratch/` 只有 `readme.md`）**

```
[FILE] \\?\D:\WorkSpace\rust\rust\web_learn\my_agent\scratch\readme.md     70 字节
[FILE] readme.md                                                            16 字节
```

4.4 倍，而且这还是**单文件目录**；8 个文件的目录会多出 400~500 字节，全是同一个前缀。JSON 里每个 `\` 还要转义成 `\\`，实际 token 消耗再涨一截。三个后果：

1. **`\\?\` 是 Windows 内部形式，模型不认识**。实测它能被回传并跑通（`is_absolute() = true`、`canonicalize` 成功），所以不是功能坏掉，是**纯粹的浪费 + 困惑**。
2. **历史不可重放**：context 里带上宿主机绝对路径，换目录/换机器同一段历史就失效，prompt cache 也跟着掉。
3. 和「接口层用相对路径」的定案直接冲突 —— 上一轮刚定的，这里漏了。

**注意排序键**：`files.sort()` 作用在 `format!("[{t}] {name}")` 上时，`[DIR]` < `[FILE]` ⇒ **目录天然排前面**，组内再按名字。这个效果不错，但它是渲染顺序的副产品 —— 如果要按 name 排再渲染，就得显式排 `(is_dir, name)` 元组，否则目录和文件会混在一起。

#### 调研：主流 agent 怎么处理工具路径（2026-10-22，外部资料）

用户要求「调研常见 agent 做法，而非自己在这里改来改去」。查了 6 个实现（含源码/官方文档）：

| 实现 | 工具收的路径 | 检查方式 | 检查后返回什么 | 安全边界放在哪 |
|---|---|---|---|---|
| **MCP filesystem server**（参考实现，`src/filesystem/lib.ts::validatePath`） | 相对**和**绝对都收；相对路径**在每个 allowed dir 下逐一 `path.resolve`，取第一个落在白名单内的** | `normalizePath` → `isPathWithinAllowedDirectories`（前缀判断）→ `fs.realpath` 后**再检查一次** | **返回 `realPath`**，后续所有 IO 都用它 | 路径白名单（多根） |
| **Claude Code**（官方 permissions 文档） | 两者都收；规则里四种前缀：`//path` 绝对、`~/path` home、`/path` **相对 settings 来源**、`path` 相对 cwd | 权限规则（gitignore 语法）+ 工作目录 + `additionalDirectories`；**symlink 检两个路径**：请求路径 + 解析目标 | 官方原文：工具打开文件时「**confirms that the path still resolves to the location the permission check approved**」 | 路径白名单 + 可选 OS 沙箱 |
| **OpenAI Codex CLI**（`codex-rs/core/src/safety.rs`） | 相对 cwd；`writable_roots` 配绝对 | `assess_patch_safety` → `AutoApprove / AskUser / Reject`；逐 target `can_write_path` | 判定结果（不是路径），真正执行仍走沙箱 | **OS 级沙箱**（bubblewrap / seatbelt / landlock）为主 |
| **Aider** | 交互/提示里**仓库相对**（"in-chat file paths are all relative to the git repo"） | `/add` 检查在 git repo 内 | **内部一律存绝对**（`abs_fnames`、`abs_root_path()`） | 仓库边界 |
| **Cline** | VS Code 给的绝对 URI；多根用 `@workspace:path` | 默认限制在 workspace 内；`.clineignore` 做访问控制 | 绝对 | workspace 边界 |
| **SWE-agent / mini-SWE-agent** | **纯相对**（repo 根） | **不做路径检查** | — | **容器**（`cwd: /testbed`） |

**三条共识**（4+ 家一致，可直接照抄）：

1. **内部一律规范化成绝对路径再比较**（MCP `realpath`、Aider `safe_abs_path`、Codex `PathUri`）。没有一家是拿两个字符串直接比的。
2. **检查通过后，后续 IO 用的是「解析后的那个路径」，不是原始请求串**。MCP 明确 `return realPath`；Claude Code 明确"打开时再确认一次仍是批准的位置"。⇒ **上一节「校验函数要返回路径」是这个行业的标准做法，不是我自己推的。**
3. **相对路径的基准必须显式定义**。Aider 的历史里就有这个 bug（issue #1080：在子目录启动，`/read /project/server/x` 被拼成 `/project/server/server/x`）—— 基准含糊时必然出现。

**两处分歧，以及我们怎么选**：

| 分歧 | 各家做法 | V0 选择 |
|---|---|---|
| 接口收相对还是绝对 | Claude Code/MCP 两者都收；Aider 内部绝对、展示相对；SWE 类纯相对 | **相对沙箱根 + 接受绝对但必须在沙箱内**（= MCP 的做法，且单根不需要"逐一尝试"） |
| 安全边界放哪 | 路径白名单（MCP/Cline/Aider）vs OS 沙箱（Codex）vs 容器（SWE） | V0 路径白名单；`run_command` 上线时**必须**换 OS 级或白名单命令，理由见下 |

⚠️ **一个必须知道的负面结论（两家官方都写明）**：

- Codex `safety.rs` 注释：*"Even though the patch appears to be constrained to writable paths, it is possible that paths in the patch are **hard links** to files outside the writable roots, so we should still run `apply_patch` in a sandbox"*
- Claude Code 文档：Read/Edit 规则 *"don't apply to a command that reads files without naming them, such as `grep -r pattern .`"*，也不管 *"arbitrary subprocesses that read or write files indirectly"*

⇒ **路径检查只能约束「我们自己实现的工具」，管不住子进程和硬链接。** 所以：V0 只有 `read_file`/`list_files` 两个自己实现的工具，路径白名单**足够**；一旦上 `run_command`，路径检查就失效了，那时必须靠 OS 沙箱 / 命令白名单 / 容器 —— 这也是 Codex 和 SWE-agent 都跳过路径检查、直接上沙箱的原因。

**一处需要补的细节**：Claude Code 对 symlink 检**两个**路径（请求路径 + 解析目标），allow 规则要求**两者都匹配**。我们只检 canon 后的目标，等价于允许「沙箱外的 symlink 指向沙箱内」。因为模型给的是相对沙箱根的路径（请求路径必然在沙箱内），这个差别在 V0 无实际影响；但要知道它是个已知的宽松点。

#### 参数类型：`&Path`，不是 `PathBuf`（2026-10-22，全链路实测）

**签名**：

```rust
fn resolve_in(root: &Path, user: &str) -> anyhow::Result<PathBuf>;   // 返回 owned
pub fn read_file(path: &Path) -> anyhow::Result<String>;             // 只借用
pub fn list_files(root: &Path, dir: &Path) -> anyhow::Result<String>;
```

**为什么是 `&Path` 而不是 `PathBuf`**：`resolve_in` 交出的 `PathBuf` 由调用方持有，工具只需要**借用**——收 `PathBuf` 就是按值消耗，函数返回后调用方再也拿不到这个路径（打日志、`strip_prefix`、错误信息都要用它）。`&PathBuf` 会自动 deref 成 `&Path`，所以 `read_file(&p)` 直接可用（实测通过）。

⚠️ **改成 `&Path` 后 `format!` 会挂**：`Path` 没有 `Display`，`format!("读取 {path} 失败")` 编译不过，要用 `path.display()`。这是唯一必须同步改的地方。

⚠️ **但"能编译"不等于"该这么写"**：`display()` 只是类型桥接，把解析后的绝对路径拼进 error message 就等于把 `\\?\D:\...` 塞进 history。错误文案里的路径应该用**模型给的那个 `args.path`**，详见 §3.5 末「错误文案里的路径用哪个」。

实测（`.probe_pb/`，已删，`resolve_in` + `read_file(&Path)` + `list_files` + `strip_prefix` 全链路）：

```
root = \\?\D:\...\sb
[read_file(&Path)] Ok("沙箱里的 a.txt")        ← &PathBuf → &Path 自动 deref
[list_files(".")]   a.txt / sub                ← 注意：没有 "./" 前缀
[list_files("sub")] sub\b.txt
[逃逸 ../outside.txt]  Err("路径越界: ../outside.txt")
[绝对但在内]           true
```

两个意外收获：

1. **canon 顺带把 `.` 归一了** ⇒ `list_files(".")` 直接输出 `a.txt`，上一轮说的"剥掉 `./` 前缀"这一步**不需要了**（`resolve_in(root, ".")` 的 canon 结果就是 root 本身）。
2. **`PathBuf::into_string() -> Result<String, PathBuf>` 确实存在**（rustc 1.99 实测，Err 里回的是 `PathBuf` 不是 `OsString`）⇒ `list_files` 里现有那句 `path.into_string().unwrap_or_else(|p| p.to_string_lossy()...)` **不用改**，比 `into_os_string().into_string()` 少一层。

**`resolve_in` 的两种失败必须文案分开**（实测都会发生）：

| 情况 | 现象 | 该报什么 |
|---|---|---|
| `canonicalize` 报 ENOENT | 路径不存在 / 父目录不存在 | “读取 `x` 失败: 系统找不到指定的文件” → 模型换路径 |
| canon 成功但 `starts_with` 失败 | 存在但在沙箱外 | “路径越界: `x`（沙箱根 `root`）” → 模型别试了 |

两者混成一句，模型就会去改一个「本来没错、只是出界」的路径，白烧几轮。

🔴 **但 `canonicalize` 那一支现在连路径都没带**（2026-10-22 实测，`resolve_in` 原样跑）：

```
[不存在的文件.txt]   系统找不到指定的文件。 (os error 2)      chain = 1
[../plan.md]        路径越界: ../plan.md                  chain = 1   ← 存在且在沙箱外，正确
[../.env]           路径越界: ../.env                     chain = 1   ← 同上
[D:\Windows\win.ini] 系统找不到指定的路径。 (os error 3)      chain = 1
```

`chain = 1` 说明**整条错误里没有任何"是哪个路径"的信息**。而 `read_file` 里那句 `.with_context(|| format!("读取 {} 失败", ..))` **永远不会触发** —— `resolve_in` 在 `read_file` 之前就失败了，那个 context 挂在更后面的一层。

更要紧的是模型的反应：只有一个 `os error 2`，它不知道是路径写错、还是目录不对、还是文件被删 —— 只能瞎试。修法是一行：

```rust
let canon = std::fs::canonicalize(&target)
    .with_context(|| format!("无法解析路径 {p}（路径相对沙箱根，可用 list_files \".\" 查看目录）"))?;
```

⚠️ **`root.display()` 绝对不能写进这里**（2026-10-23 实测修正，上一版模板写错了）：这条 context 的终点是 `Error: {e:#}` **回灌给模型**，等于把 `\\?\D:\WorkSpace\...\scratch` 塞进 history。实测回灌字符串 139 字节，其中 60 字节是宿主机绝对路径 —— 模型既用不上（它只能传相对路径），换机器后这段历史还指着一个不存在的目录。

#### 错误文案里的路径用哪个：`args.path`，不是解析后的 `Path`（2026-10-23 实测）

两个受众，两种形式，不能混：

| 去向 | 用哪个 | 写法 | 实测输出 |
|---|---|---|---|
| **回灌进 history**（`role:"tool"` / `Error: {e:#}`） | **`args.path` 原样**（模型输入的那个串） | `format!("读取 {p} 失败")` | `读取 nope.txt 失败` |
| 本地 stderr 日志（人看） | 解析后的绝对路径 | `path.display()` | `\\?\D:\...\scratch\tmp` |

关键：**`with_context` 挂在错误链上，而错误链的唯一出口就是回灌** —— 它属于"回灌"，不属于"打印"。「要 `Display` 就用 `display()`」（§2400 那条）说的是类型桥接，跟"该不该显示绝对路径"是两个维度；`display()` 只是把 `OsStr` 变成可打印，**泄漏与否取决于你 display 的是哪个 `Path`**。

实测（`.probe_leak/` 已删，直接调 `Executor::run` 打 `{:#}`）：

```
[read 不存在] 无法解析路径nope.txt，沙箱根\\?\D:\WorkSpace\rust\rust\web_learn\my_agent\scratch: 系统找不到指定的文件。 (os error 2)
              139 字节，含 1 处 verbatim 前缀
[read 目录]   读取 \\?\D:\WorkSpace\rust\rust\web_learn\my_agent\scratch\tmp 失败: 拒绝访问。 (os error 5)
              101 字节，含 1 处 verbatim 前缀
[write 到目录] 路径并不是合法文件!                                 28 字节，0 处前缀  ← 反向问题：缺路径
[list .]      [DIR] test / [DIR] tmp / [FILE] readme.md         ← 正确：file_name() 天生相对
```

⇒ 判据：**`read_file` / `list_files` / `write_file` 不写路径，由 `raw_invoke` 写** —— 它是唯一同时握着「模型输入串」和「解析结果」的地方，也是文案的唯一出口：

```rust
Invocation::ReadFile(args) => {
    let p = resolve_in(root, &args.path)?;
    read_file(&p).with_context(|| format!("读取 {} 失败", args.path))
}
Invocation::WriteFile(args) => {
    let p = resolve_for_write(root, &args.path)?;
    let n = write_file(&p, &args.content)
        .with_context(|| format!("写入 {} 失败", args.path))?;
    Ok(format!("成功写入 {} ：{n} 字节", p.strip_prefix(root).map_err(|_| anyhow::anyhow!("内部路径不在沙箱内"))?.display()))
}
```

`read_file` 里那句 `with_context(|| format!("读取 {} 失败", path.display()))` 因此可以退化成裸 `?`（它只做 IO，`path` 上已经没有"模型认识的名字"）。成功回执里的 `strip_prefix(root)?.display()` **不属于泄漏** —— 剥掉前缀后正好是干净的 `a/b.txt`，这是 `display()` 的正确用法。

⚠️ 顺带一个**已知点**：`starts_with` 检查在 canon 之后 ⇒ **越界且目标不存在**的路径报的是"找不到"而不是"越界"。模型探 `../../etc/passwd`（不存在）时会以为路径写错、继续试别的。MCP 参考实现是"先 normalize + 前缀判断，再 realpath"，正好相反。V0 接受这个顺序（先 canon 才能可靠判断越界，词法判断挡不住 `..` 和符号链接），但要知道它是**已知的文案错配**，不是 bug。

🔴 **`Invocation` 里的 `path` 字段仍是 `String`**，绝不能改 `PathBuf`：它是协议层数据，要和模型 JSON 里的 `arguments` 逐字往返（§3.4.4）。`PathBuf` 只出现在**执行层内部**，从 `resolve_in` 的返回值开始到 IO 结束。

改签名会连带这几处：`tool.rs` 的两个函数与签名、`#\[cfg(test)\]` 里的 `read_file("plan.md")` → `read_file(Path::new("plan.md"))`、`main.rs` 里若有直接调用。

#### `rg` / `jq` / `yq` / `fd` 该写进 prompt，还是做成专用工具？（2026-10-23）

用户问：「本机有 rg/fzf/yq/jq/fd 等命令行工具，是否直接写成 prompt 就可以让 agent 执行？」先测环境，再回答。

**环境实测**：五个全是 scoop shim 的 `.exe`（`C:\Users\jiahaof\scoop\shims\*.exe`）。`Command::new("rg")` **裸名**直接 spawn 5/5 成功、PATH 继承正常、版本号都拿得到（rg 15.2.0 / jq 1.8.2 / yq v4.54.1 / fd 10.5.0 / fzf 0.74.5）。
⇒ **「能不能执行」这层没有障碍，它不是问题所在。**

**逐个判定**（判据照抄 §3.5 调研小节里 Claude Code 的演化理由）：

| 工具 | 做成专用 tool？ | 理由 |
|---|---|---|
| `rg` | **值得**（V1） | 输出要行号、要截断、要权限粒度 —— 正是 Claude Code 把 `Grep` 从 `Bash` 独立出来的那三条。可用 `ignore` + `regex` 纯 crate 实现，不依赖外部 exe |
| `jq` / `yq` | 不必 | 它们只是"读取者"，模型在 `run_command` 里调即可；读 JSON/YAML 本来就是 LLM 的强项，宿主不需要理解其输出 |
| `fd` | V1 再说 | 与 `list_files` 能力重叠，唯一增量是 `.gitignore` 感知 |
| `fzf` | **不能给** | 交互式 TUI。实测非 TTY 下**零输出直接退出**（`printf 'a\nb\n' \| fzf` → stdout 空，exit 0）⇒ 模型只拿到空结果，分不清"没匹配"和"用不了" |

**关键区分：「prompt 声明」与「tools 声明」不是同一强度的契约。**

| | `tools` 数组 | system prompt 文字 |
|---|---|---|
| 参数格式 | JSON Schema 强制 | 自然语言捏的字符串，宿主无法校验 |
| 宿主控制 | 可逐参数审计 / 截断 / 拒绝 | 只能整串接受 |
| 可见性 | 每轮必在请求里 | 多轮之后会被注意力稀释 |

⇒ **要宿主控制的（read / write / list / rg）做成 tool；长尾只读、输出简单的（jq / yq）留在 prompt 里走 `run_command`。**
「写成 prompt 就能跑」在**能力**上成立，在**安全**上不成立 —— 原因见下节。

#### `run_command`：V0 不给 shell，并且要知道路径沙箱在此彻底失效（2026-10-23，六组实测）

命令能跑通（上节），但一旦落到 `run_command` 上，有五个实测确认的坑。

**① 模型写的命令语法依赖 shell，而"给一个 shell"在 Windows 上没有免费选项。**

实测（`.probe_cli/`，已删）：

| 写法 | 结果 | 说明 |
|---|---|---|
| `Command::new("rg").arg("needle \| head -1")` | **code=0，有输出** | rg 把 `needle \| head -1` 当**正则**（交替分支）⇒ 静默给出错误结果。**不报错的错最贵** |
| `cmd /C "rg -n 'needle' a.txt"` | code=1 | cmd 里单引号不是引号，rg 收到 pattern `'needle'`（带引号） |
| `cmd /C "echo $HOME"` | 输出 `$HOME` | cmd 用 `%VAR%` |
| `cmd /C "rg -n \"needle\" a.txt"` | code=1 | Rust → cmd 两层引号解析打架（`Command` 会先替 arg 加引号/转义） |
| `sh -c "cd .. && cat a.txt"` | 读到外层文件 | 语法对，但见下 |
| `Command::new("bash")` | code=1 | 裸名命中的是 **WSL 的 bash**，还被安全策略拦 |

`sh` 确实能用，但 `where sh` 显示它来自 `D:\Program\WorkBuddy\resources\vendor\PortableGit\usr\bin\sh.exe` —— **是宿主应用自带的，不是这台机器的环境**。换一台没装 WorkBuddy 的机器就没有。

⇒ **V0 定案：`run_command` 收 `{ program: String, args: Vec<String> }`，不接受 shell 命令字符串。** 工具描述里显式否定：「没有 shell —— 不支持管道 `|`、重定向 `>`、`&&`、通配符、变量展开」。三条理由：

1. 避开上表第一行那种**静默错误**；
2. 程序名可以精确白名单，命令串不能；
3. `cmd` 的内建命令（`cd`/`echo`/`type`/`dir`）输出是 **GBK**，`from_utf8_lossy` 后是乱码（实测 `'nosuchcmd_xyz' 不是内部或外部命令` → `'nosuchcmd_xyz' �����ڲ����ⲿ���`）；而 exe 自己输出的 UTF-8 正常。只用 exe 顺带解决编码。

代价：模型训练数据里 99% 是 bash 管道，它必然先试错一两次。所以描述要显式否定，并且错误回灌带上「本工具不支持 shell 语法，请拆成多次调用」。

**② 路径沙箱在 `run_command` 上失效 —— 实测三重越界。**

`current_dir` 已设成沙箱内，仍然全部读到外面：

```
cmd /C "cd .. && type a.txt"   -> OUTSIDE        （cd 越界）
rg -n OUTSIDE ..               -> 搜到外层文件     （纯相对路径越界，不需要 cd）
fd . ..                        -> 列出外层全部
Command::new("rg").args(["-n","OUTSIDE",".."])  -> 同样越界（不包 shell 也一样）
```

最后一行是关键：**越界与是否用 shell 无关，只要允许任意 argv 就存在。** 印证 §3.5 调研小节里两家官方说法（Codex 的 *hard links*、Claude Code 的 *subprocesses that read or write files indirectly*），现在有本机复现。

**③ 硬链接连「路径」这一层都能骗过（实测，无需任何权限）。**

```
在 sb/ 内 hard_link 指向沙箱外的文件      -> 成功
canon(sb/inside_hard.txt) 在沙箱内        -> true
父目录 canon 在沙箱内                     -> true
symlink_metadata.is_symlink               -> false
fs::write 通过它写 -> 沙箱外的原文件变成 "OVERWRITTEN-BY-AGENT"
```

硬链接没有"目标"概念，它是同一 inode 的第二个名字，**canon 结果就是沙箱内的路径** ⇒ `starts_with` 通过、`is_symlink` 为 false、任何路径检查都发现不了。
威胁模型上，链接必须事先存在、agent 自己造不出来 ⇒ **V0 无实际风险**；但它证明「路径检查不是安全边界」是真的，Codex 那条注释讲的就是这件事。

**④ 退出码不是「0 = 成功」这么简单。**

```
rg 有匹配   -> code=0   success()=true
rg 无匹配   -> code=1   success()=false   ← 这是正常业务结果
rg 参数错   -> code=2   success()=false   ← 这才是真错
```

宿主要是写 `if !output.status.success() { bail!() }`，**「搜不到」会变成工具失败回灌**，模型会以为 rg 坏了、转去试别的办法。
最初定的规则是「**0 和 1 都算成功**，2 及以上才标 `Error:`」—— **这条被 2026-10-26 的复测推翻了一半**，见下面 `#### 退出码分档是错的`。

**⑤ 两处必须显式设的东西。**

- `.current_dir(sandbox_root)` **必须显式设**：实测子进程默认继承父进程 CWD（父 `my_agent` → 子 `my_agent`）。不设的话模型写 `cat a.txt` 读的是 `my_agent/a.txt`。与「检查 A 使用 B」是同一个 bug 家族。
- **白名单只允许裸名**：拒绝绝对路径、拒绝 `..\..\bin\rg`、拒绝 `cmd`/`sh`/`powershell`/`wsl` 本身 —— 否则 `cmd /C ...` 就是万能逃逸，前面全白给。

**⑥ 输出合并 + 截断**：stdout 与 stderr 都要（实测 `rg needle no_such_file.txt` 的错误全在 stderr、stdout 为空），合并后走现成的 `truncates`，并附退出码。

#### `run_command`：退出码分档是错的，诊断信息可能整个丢光（2026-10-26，v2 复测）

实现第一版按「0/1 = 成功、2+ = 失败」分档，并且**成功只取 stdout、失败只取 stderr**。两组实测证明这个分档在原理上就不成立。

**① 退出码 `1` 的语义是程序特定的，宿主没法编码。**

| 调用 | 退出码 | 输出去哪 |
|---|---|---|
| `rg zzz_nomatch .`（无匹配） | **1** | stdout 空 —— 这是**正常结果** |
| `yq . nosuch.yaml`（文件不存在） | **1** | **stderr**: `Error: open nosuch.yaml: The system cannot find the file specified.` |
| `yq . bad.yaml`（YAML 语法错） | **1** | **stderr** |
| `jq . <<< 'not json'` | **5** | **stderr**: `jq: parse error: Invalid numeric literal at line 1, column 4` |
| `jq . <<< '{"a":1}'` | 0 | stdout |

`yq` 的两行是关键：**真错误也是 1**。分档把它判成成功 ⇒ 回执是 `命令退出码 1:\n`（stdout 空、stderr 被丢）—— **模型收到一条零信息量的回执**，只能瞎猜。

**最硬的取证（2026-10-26，走真实 `Executor::run` → `format!("Error: {e:#}")`）：两条性质完全相反的命令，回执逐字节相同。**

```
yq . nosuch.yaml   （文件不存在，真错误）  ->  "命令退出码 1:"   19 字节
rg  zzz_nomatch .  （正常业务结果，无匹配）->  "命令退出码 1:"   19 字节
```

⇒ 问题不是"少补一个字段"，而是**这两个结果在此设计下不可区分**。任何"给回执加点信息"的补丁都救不了，只能改流划分。

**② 诊断信息经常在「不对应的那个流」里。**

`cargo test` 有一个失败用例时：退出码 **101**，`running 1 test / test bad ... FAILED / panicked at src\lib.rs:2:12 / test result: FAILED` 全在 **stdout**，stderr 只有 **314 字节**的 `Compiling… / error: test failed, to rerun pass --lib`。

按「失败只取 stderr」的写法，模型永远看不到**哪个测试失败、断言是什么、在哪一行**。而 V0 的验收任务正是「让 agent 在 `my_api` 副本上跑编译/测试并修」⇒ 这是对目标最致命的一条。

**③ 顺带量到截断缺口。**

同一个 80 个类型错误的 crate：

```
cargo check --message-format=short   -> stderr 6,533 字节
cargo check（默认 rich 渲染）        -> stderr 17,213 字节 / 643 行
MAX_TOOL_OUTPUT                      -> 8,192 字节
```

两件事：① 失败分支走 `bail!` ⇒ 绕过 `truncates`（`Executor::run` 的 `?` 直接上抛，`agent.rs` 只做 `format!("Error: {e:#}")`），17KB 原样进 history；② 即使截断生效，`error: could not compile … due to N previous errors` 这句在**尾部**，head-only 策略会把它切掉。

**定案（取代 0/1 分档）：**

- `run_command` **只把「进程没能启动」当 `Err`**（`spawn()` 失败：程序不在 PATH、参数非法）。退出码 0/1/2/101 全部走 `Ok`，回执里三段带标签：`退出码 N` + `--- stdout ---` + `--- stderr ---`（空的流省略）。理由：退出码是**程序的**语义，不是**宿主**的；而且这样只有一条路经过 `truncates`。
- `Error:` 前缀保留给真正的工具失败（白名单、裸名、路径越界、程序起不来），**编译失败/测试失败是任务信息，不是工具故障** —— 与「模型能不能靠改变下一次输出来修复」那条分流判据一致。
- 仍建议 `Executor::run` 对 `Err` 也截一次（`map_err(|e| anyhow!("{}", truncates(format!("{e:#}"), self.max_bytes)))`）作为兜底网，V1 再说。
- 实用建议写进 system prompt：编译检查用 `cargo check --message-format=short`（实测 6.5KB vs 17KB，同样的信息量）。


#### `write_file` 是整文件覆盖，不是插入（2026-10-23，实测）

用户问「writefile 本质上是覆盖整个 file 而非插入对吧」——**对，就是 `fs::write` 的语义**。实测：

```
fs::write("t.txt", 20 字节)  -> "AAAAAAAAAAAAAAAAAAAA"
fs::write("t.txt", "BB")     -> after = "BB"（2 字节，截断覆盖，无残留）
父目录不存在                  -> NotFound「系统找不到指定的路径。(os error 3)」
```

这就是主流 agent 的 `Write` / `write_file`，与「替换」是两个工具（Claude Code 的 `Write` vs `Edit`，Gemini CLI 的 `write_file` vs `replace`）。V0 只做前者：

| | 整文件覆盖 | 插入/替换（`Edit`） |
|---|---|---|
| 实现 | `fs::write` 一行 | 要唯一匹配、要 diff、要 CRLF 归一、要防「匹配到多处」 |
| 模型负担 | 必须先读、再输出全文 | 只给 old/new 片段 |
| token | 改一行 = 重写全文 | 只传片段 |

代价是 token。对 V0 的 `scratch/` 小文件可接受；V1 有真实项目时 `Edit` 才划算。

⚠️ **覆盖语义带来两个必须处理的点**：

1. **必须先读**：没读过就写 = 把文件其余内容静默抹掉。Claude Code 的做法是「文件已存在但本会话没读过 → 拒绝并要求先 Read」。V0 可以退一步：返回里带一句「已覆盖 N 字节」让模型自己核。
2. **目标是已存在的 symlink 时会写穿到沙箱外**（`fs::write` 跟随链接）。本机**无法实测** —— `symlink_file` 报 `os error 1314 客户端没有所需的特权`（未开开发者模式）。按 std 文档「`write` 会 truncate 已存在文件」推断成立。
   ⇒ 检查规则分两步：**父目录 canon + `starts_with`（目标可能不存在）+ 目标若存在则目标也 canon + `starts_with`**（挡 symlink）。硬链接挡不住（上一节实测），但那要求链接事先存在。

顺带：`write_file` 的错误文案要像 `read_file` 一样带路径 —— `fs::write` 自己的 `os error 3` 不说是哪个路径。

#### 要不要一个独立的 `mkdir` 工具：**取决于你有没有 shell**（2026-10-23，查证五家）

用户质疑：「一般 agent 也没有 mkdir 工具吧？我还以为都是使用 bash/pwsh 命令呢」。查了五个实现，结论比"有/没有"更细 —— **没有独立 mkdir 工具是多数派，但每个都留了别的路能建目录**：

| 实现 | 有独立 mkdir 工具？ | 那目录由谁建 |
|---|---|---|
| **Claude Code** | 无 | `Bash` 里 `mkdir -p`（有 shell 就有兜底） |
| **Codex CLI** | 无 | `apply_patch` 里的 `write_file_with_missing_parent_retry`：**先写，遇 `ErrorKind::NotFound` 才 `create_directory(recursive: true)` 然后重试** |
| **Gemini CLI** | 无 | `write-file.ts` 的 `applyWrite`：`try { await fsPromises.access(dirName) } catch { await fsPromises.mkdir(dirName, { recursive: true }) }` |
| **MCP filesystem**（参考实现） | **有 `create_directory`** | `write_file` **不建目录**，直接报 `Parent directory does not exist: <dir>`；注释明写 `mkdir -p` 归 `create_directory` 管 |
| **Aider** | 无 | 走 shell / `utils.touch_file`（未追到最终落盘处，标注为未核实） |

⇒ 真正的规律：**只有当"既没有 shell、`write_file` 又不自动建目录"时才需要一个 `create_directory` 工具** —— 五家里只有 MCP filesystem 是这个组合，所以也只有它加了。
反过来说：**「没有 mkdir 工具」不等于「不能建目录」**，它一定是在别处兜住了。如果两条路都堵死，模型就真的完不成"新建 `src/foo.rs`"。

**这对 V0 是决定性的**：V0 的 `run_command` 定案是**无 shell**（上节），所以模型手里**没有 `mkdir -p` 这条兜底**。于是：

- ❌ 「父目录必须已存在」在 V0 里是死路（上一轮提议的 A 方案，**撤回**）—— 模型撞墙后没有任何别的工具能救，只能反复重试同一个路径。
- ✅ 必须让 `write_file` 自己建目录（Gemini / Codex 路线），或者补一个 `create_directory` 工具（MCP 路线）。V0 选前者：**工具数量是成本**，4 个工具里塞第 5 个只为建目录不划算。

**两种自建目录的实现，V0 选 Codex 那种「写失败再重试」**，因为不用提前判断目录存不存在：

```rust
match fs::write(&target, content) {
    Err(e) if e.kind() == ErrorKind::NotFound => {
        /* 沙箱检查 → create_dir_all(target.parent()) → 重试 fs::write */
    }
    other => other,
}
```

⚠️ 但**沙箱检查卡在这里**：父目录不存在 ⇒ `canonicalize` 必然 ENOENT ⇒ 上一节的「父目录 canon + starts_with」用不了。三种解法：

| 解法 | 说明 | 评价 |
|---|---|---|
| 词法 normalize + 前缀比较 | 栈式消掉 `..` 后做组件前缀判断 | 挡得住 `../../`，挡不住 symlink 祖先 |
| **找最深的已存在祖先再 canon** | 从 `target.parent()` 往上找到第一个 `try_exists()` 为真的目录，canon 它 + `starts_with`，再 `create_dir_all` | **MCP 的 `resolveUnicodeEquivalentPath` 就是这个**（realpath 已存在部分 + append 不存在的尾巴 + 前缀检查） |
| 先 `create_dir_all` 再 canon | 顺序反了 | ❌ 等于先在沙箱外把目录建出来了 |

选第二种。两个细节：用 `try_exists()` 不用 `exists()`（后者在权限错误时也返回 `false`，std 文档明确警告）；找到祖先之后**还要对最终路径再 canon 一次**（挡 symlink），因为 `create_dir_all` 之后路径已存在。

**顺带一个必须自己检查的理由**：`fs::write` 指向目录时的报错是 `PermissionDenied「拒绝访问。(os error 5)」`，**完全看不出是"目标是目录"**（实测）。所以写之前用 `symlink_metadata(&target)` 判一下：

```
symlink_metadata(dir).file_type()  -> is_file=false is_dir=true     ← 能拒
create_new 对已存在文件             -> ErrorKind::AlreadyExists      ← 可用于原子独占创建
create_dir_all 对已存在目录         -> Ok（幂等）
```

这正是 MCP `writeFileContent` 的做法：先用 `flag: "wx"`（Rust 的 `.create_new(true)`）**原子独占创建**，`EEXIST` 时再 `lstat` 检查是不是普通文件，不是就 `Refusing to write: not a regular file`。两个好处：① `create_new` 不跟随 symlink，天然挡住"写穿链接"；② 原子，没有"检查完被换成 symlink"的窗口（TOCTOU）。V0 可以只取 ①（先 `symlink_metadata` 判 `is_file()`），TOCTOU 留给 OS 沙箱。

#### 🔴 实测：纯词法 normalize 挡不住 junction，`write_file` 能写穿（2026-10-23）

上表里「词法 normalize 挡不住 symlink 祖先」不是理论 —— **Windows 的 junction 不需要管理员也不需要开发者模式**（`mklink /J`，普通用户即可），实测直接写穿：

```
root                                   = \\?\D:\...\sb
target                                 = \\?\D:\...\sb\link\escape.txt     (link 是指向 ..\outside 的 junction)
词法 normalize                          = \\?\D:\...\sb\link\escape.txt
starts_with(root)                      = true      ← 现实现据此判定，放行
fs::write                              = Ok
沙箱外 D:\...\outside\escape.txt         = 出现，内容 "ESCAPED"
```

同一份路径下，两条分支的结论**完全相反**：

| 分支 | 用什么判 | 结果 |
|---|---|---|
| `resolve_in`（读） | `canonicalize(target)` → `\\?\...\outside\escape.txt` | `starts_with` = **false**，被挡住 ✔ |
| `resolve_for_write`（写） | 只做词法 normalize | `starts_with` = **true**，写穿 ✘ |

⇒ **同一个沙箱，读是安全的、写不安全**，而读之所以安全纯属「目标必须存在才能 canon」这个巧合，不是设计出来的。

修法就是上表第二种：write 侧也做 canon，只是 canon 的对象是**最深的已存在祖先**而不是目标本身。实测这样能挡住：

```
canon(sb\link)          = \\?\...\outside
starts_with(root)       = false     ← 修法据此判定，拒绝
```

落地要点：`normalize_path` 之后，从 `lexical.parent()` 往上逐个 `try_exists()` 找到第一个存在的祖先 → `canonicalize(ancestor)?` → `starts_with(root)` → `Ok(canon_ancestor.join(lexical.strip_prefix(ancestor)?))`。**目标存在时**（覆盖已有文件）直接 canon 目标即可，那一支天然安全。

⚠️ 已知残留（V0 接受）：**硬链接**连这个也挡不住 —— canon 在沙箱内、`is_symlink`=false（2026-10-23 实测），只能靠 OS 沙箱；这与 Codex 官方那句 *"paths in the patch are hard links to files outside the writable roots"* 是同一个结论。

#### 复测 v3：加了「往上找已存在祖先」的循环，但仍然一次 canon 都没有（2026-10-23）

v3 的写法是「从 `lexical` 往上 `try_exists()` 找到第一个存在的祖先，然后对**它**做 `starts_with(root)`，返回原来的 `lexical`」。**循环白写了** —— 找到的是词法祖先，比较的也是词法，`fs::canonicalize` 一次都没调用：

```
[write link/escape.txt]          Ok("成功写入文件link\escape.txt : 9 字节")
  沙箱外 outside/escape.txt exists = true   content = "ESCAPED-1"
[write link/deep/nest/x.txt]     Ok("成功写入文件link\deep\nest\x.txt : 9 字节")
  沙箱外 outside/deep/nest/x.txt exists = true     ← 连多级目录都建到沙箱外
[write ok.txt]                   Ok("成功写入文件ok.txt : 4 字节")   ← 正常路径没坏
```

⇒ **缺的就是那一行 `canonicalize(ancestor)`**，不是缺循环。

同一个「没 canon」还制造了第二个 bug —— **误报越界**（模型给沙箱内的绝对路径时）：

```
root  = \\?\D:\WorkSpace\...\sb          components[0] = Prefix(VerbatimDisk('D'))
plain = D:\WorkSpace\...\sb\a.txt        components[0] = Prefix(Disk('D'))
plain.starts_with(root)       = false    ← 误报越界
canon(plain).starts_with(root) = true    ← canon 之后就对了
```

这就是 §七 ②「**两边都 canon 才可比**」的第二个实例：`VerbatimDisk` 与 `Disk` 是两个不同的 `Prefix` 变体，逐字节比较必然 false。`resolve_in` 因为 canon 过所以没这问题，`resolve_for_write` 有。

🔴 **修的时候有个坑：祖先 == 目标时尾巴是空串，`join("")` 会加尾分隔符**：

```
PathBuf::from(r"\\?\D:\a\b.txt").join("")  = "\\?\D:\a\b.txt\"     ← 尾部多一个分隔符
Path::new("a.txt\").strip_prefix(自身)      = Ok("")                ← 空尾巴的来源
```

`join("")` **不是 no-op**，它会补一个分隔符。而这个分隔符在 Win32 语义里等价于「**按目录打开**」，实测（`.probe_trail/` 已删，`a.txt` 是已存在的普通文件）：

| 调用 | 结果 |
|---|---|
| `fs::symlink_metadata("a.txt")` | `Ok(true)` |
| `fs::symlink_metadata("a.txt\")` | **`Err(267, NotADirectory, "目录名称无效。")`** |
| `fs::metadata("a.txt\")` | 同上 267 |
| `fs::canonicalize("a.txt\")` | 同上 267 |
| `fs::write("a.txt\", "NEW")` | 267，**原文件内容不变**（没被截断） |

⇒ 覆盖已有文件这条路径会 100% 失败，回灌是 `写入 ok.txt 失败: 无法读取文件状态: 目录名称无效。 (os error 267)` —— 模型会以为路径有问题，去换名字重试。

⚠️ **这个 bug 在类型上完全看不见**：`Path` 的 `PartialEq` **按组件比较**，而 components 会忽略尾分隔符，所以

```
Path::new("D:\\t\\a.txt\\") == Path::new("D:\\t\\a.txt")   ->  true      ← Path 相等
   .as_os_str() == .as_os_str()                            ->  false     ← 字节不等
file_name() 两者都是 Some("a.txt")，components().count() 都是 9
```

也就是说 `starts_with` / `strip_prefix` / 测试里的 `assert_eq!(p, expect)` / `Debug` 打印**全都看不出差别**，只有 `OsStr` 逐字节比才暴露。这就是它只能靠实测发现的原因。

所以必须 `if tail.as_os_str().is_empty() { canon_ancestor } else { canon_ancestor.join(tail) }` —— 不是洁癖，是 267 的硬要求。

正确顺序（4 行，关键在第 3 步）：

```rust
let lexical = normalize_path(&target)?;                                  // 1. 消 ..
let ancestor = 从 lexical 往上找到第一个 try_exists() 为真的路径;          // 2. 循环
let canon_ancestor = fs::canonicalize(ancestor)?;                        // 3. ★ 缺的就是这行
if !canon_ancestor.starts_with(root) { bail!("路径越界: {p}") }           // 4. 比 canon 过的
// 5. 返回 canon_ancestor（尾巴为空）或 canon_ancestor.join(lexical.strip_prefix(ancestor)?)
```

一个可选的加固：`try_exists()` 返回 `Err`（权限拒绝）时现在被 `is_ok_and` 当成「不存在」继续往上走，最后错误会在 `fs::write` 处以 `PermissionDenied` 冒出来 —— 能接受，但更干净的是 `Err` 就直接 break，让 `canonicalize` 把真实原因报出来。

#### ✅ 复测 v4：canon 补上了，junction 挡住；但「方案 A」在回执上挂了（2026-10-23）

v4 的写法是**方案 A** —— canon 祖先做检查，但**返回词法 `canon`**（不 join 尾巴）。实测四类场景：

| # | 场景 | 结果 |
|---|---|---|
| 1 | `link/escape.txt`（junction 单层） | `Err("路径越界: link/escape.txt")`，**沙箱外没多文件** ✔ |
| 2 | `link/deep/nest/x.txt`（junction 多级） | `Err("路径越界: ...")`，沙箱外没建目录 ✔ |
| 3 | `ok.txt`（普通相对） | `Ok("成功写入文件ok.txt : 4 字节")` ✔ |
| 4 | `ok.txt`（**覆盖**已有文件） | `Ok("成功写入文件ok.txt : 9 字节")`，内容被替换 ✔ |
| 5 | `D:\...\sb\abs.txt`（**沙箱内、普通绝对路径**） | 🔴 `Err("prefix not found")`，**但 `abs.txt` 真的写进去了** |

**安全侧全对，回执侧挂了一个**：第 5 条是「写成功却报失败」—— 最坏的一类。原因在 `raw_invoke` 的回执：

```rust
Ok(format!("成功写入文件{} : {s} 字节", p.strip_prefix(root)?.display()))
//                                        ↑ p 是词法路径 D:\...\sb\abs.txt
//                                          root 是 canon 路径 \\?\D:\...\sb
```

`D:\...`.strip_prefix(`\\?\D:\...`) → `Err(StripPrefixError)`，Display 就是 `prefix not found`（**一句不提路径**）。而写已经完成了。

⇒ 这正是 §七 ②「**两边都 canon 才可比**」的第三次出现，也是「方案 A：检查 A 使用 B」的代价 —— 我上一轮说"A 能用"只对了一半：**安全性上 A 没问题（尾巴不存在⇒无链接），但它把词法路径交回给了下游**，下游任何 `starts_with` / `strip_prefix` 都会撞上前缀不匹配。

**两条出路，选 B**：

- **B（推荐）**：返回 `canon_祖先.join(词法尾巴)` → 交出去的永远是 canon 前缀 + 词法尾巴，`strip_prefix(root)` 恒成立。代价是**必须处理空尾巴**（上一小节那个 267）。
- A + 补丁：`strip_prefix(root).unwrap_or(&p)` → 不报错了，但回执会带 `D:\...`（宿主机绝对路径进 history，正是要避免的）。**不选**：修好一个洞又开一个。

✅ 一句话：**"检查过的路径"和"交出去的路径"必须是同一个值** —— canon 只解决"怎么检查"，返回什么才决定"下游能不能用"。

#### ✅ 复测 v6：方案 B 落地，11/11 通过（2026-10-26）

v5 已经补了 `canon_ancestor.join(tail)`，但仍然 `prefix not found`。**根因不在拼接，在 `strip_prefix` 的左手**：

```rust
let tail = canon.strip_prefix(&canon_ancestor).unwrap_or(&canon);   // ✘ v5
let tail = canon.strip_prefix(canon_tmp)?;                          // ✔ v6
```

`canon` 是**词法**路径，`canon_ancestor` 是 **canon 后**的。模型给普通绝对路径（`D:\...`）时，`canon` 是 `Prefix(Disk('D'))`、`canon_ancestor` 是 `Prefix(VerbatimDisk('D'))` ⇒ strip 必失败 ⇒ `unwrap_or(&canon)` 把**整条绝对路径**当尾巴 ⇒ `join` 遇绝对路径**静默替换** ⇒ 返回词法路径，等于退回方案 A。

左手换成 `canon_tmp`（循环里那个**词法**祖先）就必然成功：`canon` 正是沿它的 `parent()` 链派生的，同源同形态。

v6 实测 11/11（含 junction 用 `mklink /J` 真建）：

| # | 输入 | 结果 |
|---|---|---|
| 9a / 9b | `D:\...\sb\abs.txt`（**沙箱内普通绝对路径**，新建 / 覆盖） | `Ok` ✔ —— 上两版的 `prefix not found` 消失 |
| 1 / 4 | `new.txt` / `ok.txt`（覆盖） | `Ok`，内容替换 ✔ |
| 7 / 8 | `sub/case.txt`（真实名 `Sub`），覆盖同上 | `Ok`，回执是 **`Sub\case.txt`**（真实大小写）⇒ 证明走的是真拼接，不再靠 `unwrap_or` 退化 |
| 2 / 3 | `link/escape.txt`、`link/deep/nest/x.txt` | `Err 路径越界`，沙箱外无文件、无目录 ✔ |
| 5 | `../x.txt` | `Err 路径越界`，无副作用 ✔ |
| 10 / 11 | verbatim 绝对路径 / `d1/d2/d3/deep.txt` | `Ok` ✔ |

第 7、8 条是**判断"到底走了哪条分支"的探针**：`unwrap_or` 那条会原样返回词法大小写（回执 `sub\...`），真拼接这条返回 canon 前缀（回执 `Sub\...`）。只断言 `is_ok()` 永远看不出区别。

⚠️ `unwrap_or` 这形态在本项目已第三次出现（`to_str()` 的 `if let`、`is_ok_and` 吞 `Err`、这里的 `unwrap_or(&canon)`）—— **失败被静默降级成"看起来成功"**，是这个项目最容易复发的一类 bug。

#### `\\?\` 到底是什么，为什么每次都在（跨平台对照）

`\\?\` 是 Win32 的 **verbatim path（逐字路径）** 前缀，Rust 里是 `Prefix::VerbatimDisk`。名字里的 `?` 语义就是「**这串别再解析了，直接往下传**」。

Win32 拿到路径后做两件不同的事：

```
plain:    D:\a\..\b
            ↓ RtlGetFullPathName_U ← 拼 CWD、消 . / ..、把 / 改成 \、剥尾随点和空格
          D:\b
            ↓
          \??\D:\b                    ← 对象管理器的 DOS 设备目录

verbatim: \\?\D:\a\..\b
            ↓ 只做一件：\\?\ → \?\
          \??\D:\a\..\b               ← 一个字节都不解释
```

`\??\` 是 NT 对象命名空间里真实存在的目录（`\??\C:` 就是盘符），`\\?\` 是它在 Win32 侧的别名。`\\?\UNC\server\share\x` 是同一条规则用在网络路径上（`\\?\` 后面必须接卷，UNC 没有卷，所以补一个 `UNC` 段）—— 实测 `canonicalize("\\\\localhost\\C$\\Windows")` = `\\?\UNC\localhost\C$\Windows`。

**它关掉的四件事**（实测 2026-10-26）：

| 关掉什么 | 实测（同一路径两种写法） |
|---|---|
| `..` / `.` 解析 | `sb\a\..\b.txt`：plain 写成 **`sb\b.txt`**；verbatim **报 123**（把 `..` 当字面目录名） |
| `/` → `\` 转换 | `sb/slash/x.txt`：plain 当分隔符；verbatim **报 123**（`/` 只是普通字符） |
| 剥尾随点和空格 | 写 `trail.`：plain 落盘 **`trail`**；verbatim 落盘 **`trail.`**（两个文件并存） |
| 260 字符上限 | 本机 `LongPathsEnabled = 1`（Win11 build 26300）⇒ 355 字符 plain 也成功，**这条测不出来** |

最后一行说明：`\\?\` 从来是"无条件解药"，但 Win10 1607+ 的 `LongPathsEnabled`（需程序 manifest 声明 `longPathAware`）让它对长度不再是唯一手段。**我们现在需要它的理由是前三条语义，不是长度。**

**为什么 canonicalize 必然给 verbatim**：`GetFinalPathNameByHandleW` 默认 `VOLUME_NAME_DOS` 就返回这个形式 —— 因为只有它能把真名 1:1 表示出来（plain 形式会剥尾随空格、展开 8.3 短名、改大小写）。⇒ **只要 canon，就必然撞 `VerbatimDisk` vs `Disk`**，`starts_with` / `strip_prefix` 的两次坑都源于此。

**⚠️ 它不是安全特性，是"更字面"**：`\\?\` 下 `NUL` / `CON` 这类 Win32 保留设备名**真的能被创建成普通文件**（实测落盘条目出现 `CON`、`NUL`），而 plain 写法是写进设备黑洞（返回 `Ok` 但什么都没创建）。它不做任何访问检查，沙箱仍得自己 canon + `starts_with`。推论：`normalize_path` **必须自己消掉 `..`**，不能指望系统 —— 漏了会以 `123 文件名、目录名或卷标语法不正确` 出现，模型完全猜不到是 `..` 的锅。

**Linux / POSIX 对照**（换平台时不用重新推理）：

| 现象 | Windows | Linux |
|---|---|---|
| canon 输出形态 | `\\?\D:\...` verbatim | 纯绝对路径，无前缀 |
| 盘符前缀变体 | `Disk` vs `VerbatimDisk` ⇒ 两边必须都 canon | 不存在，`starts_with` 不会因形态不同失败 |
| 大小写 | 不敏感，canon 归一成真实大小写（`sub` → `Sub`） | 敏感，`sub` 和 `Sub` 是两个目录 |
| `..` 谁解析 | Win32 层（verbatim 下不解析，报 123） | **内核**解析，且是在**跟随 symlink 之后**应用 ⇒ `sb/link/../x`（`link`→ 外面）会逃逸 |
| `/` 与 `\` | `/` 被转换；`\` 是分隔符 | `/` 是分隔符；**`\` 是合法文件名字符** |
| 尾随分隔符 | 267 `NotADirectory` | `ENOTDIR`（同义不同 errno） |
| 长度上限 | 260（或 32767） | 单段 255 字节，总长 4096 |

结论：**`\\?\` 这一整套（前缀变体、大小写归一、尾随点剥离、`..` 报 123）在 Linux 上全部消失**，这是真实简化 —— v6 里 `strip_prefix(canon_tmp)` 那类坑不会出现。但**沙箱的两条根本洞（symlink、硬链接）在 Linux 上照样存在**，因为那是文件系统语义而非 Win32 层；而且 Linux 的 `..` 在 symlink 之后应用，**词法 normalize 反而更不可省**（只靠内核会把 `sb/link/../x` 解析到沙箱外）。

#### `run_command` v3（2026-10-26）：退出码问题已解，暴露出「安全检查被拆成两层」

v3 把退出码全走 `Ok`（只有 `spawn`/`wait_with_output` 失败才 `Err`），格式改成
`命令退出码: N` + `返回结果:` + `错误信息:` 三段 —— 上面那节的两个「零信息量回执」问题消失。
但新版本有四处：

**🔴 ① 编译不过：逗号落进了块里**

```rust
run_command(&args.program, &args.args, root)
    .with_context(|| format!("执行命令 {} 失败", args.program)),}   // ← 这个逗号在 } 之前
```

报 `expected one of '.', ';', '?', '}', or an operator, found ','`。`),}` 的读法是「表达式 + 逗号」然后 `}`
关块 —— 块内 `expr,` 非法。逗号要放在 `}` **之后**。

**🔴 ② 裸名检查成了死代码（两层检查顺序反了）**

```
raw_invoke:  白名单（ALLOW_CMD.contains）   ← 先跑
run_command: 裸名（file_name() == program）  ← 永远到不了
```

实测 `file_name()` 判定（`.probe_bare`，已删）：

| `program` | `file_name()` | 裸名检查 |
|---|---|---|
| `rg` | `Some("rg")` | 过 |
| `rg.exe` | `Some("rg.exe")` | **过** ⇒ 由白名单兜住，**两个检查互补，不冗余** |
| `./rg` | `Some("rg")` | 拦 |
| `C:\…\rg.exe` / `""` / `..` / `sub/../rg` | `None` 或不等 | 拦 |

因为白名单在前，`rg.exe` / `./rg` / 绝对路径**三条都撞同一句**「不是允许的合法命令」，
裸名那句「必须是裸名」一次都不出现。

**定案：两个检查必须在同一层**，不能一半在分派层、一半在工具里。

- **A（推荐，与 `resolve_in` / `write_file` 的先例一致）**：都放 `raw_invoke`，**裸名在前、白名单在后**；
  `run_command` 只留注释「无条件信任 `program` 已过校验」—— 跟 `write_file` 那句契约同一个模式。
- **B**：都放 `run_command`，`raw_invoke` 只调不分派。

必须选一个的硬理由：**V1 换 `Vec<Box<dyn Tool>>` 时 `raw_invoke` 的 `match` 会整个消失**
⇒ 白名单留在分派层就等于**静默丢掉安全边界**（`write_file` 的 `_ => bail!` 那次是同一个教训的镜像）。

**🟠 ③ `stdin` 默认是 inherit**

`Command` 的 stdin/stdout/stderr **默认全部继承父进程**（std 文档），v3 只 pipe 了后两个。
⇒ `cargo run`、任何读 stdin 的程序挂在**你的终端**上等输入，`wait_with_output()` 无限等、agent 卡死。
`.stdin(Stdio::null())` 一行，把它们从「永久阻塞」降成「立刻 EOF」。
（这条是文档结论，未实测 —— 演示阻塞需要一个真 TTY，探针环境里 stdin 本来就是 EOF。）

**🟡 ④ 其余小处**

- `RunCommandArg` 缺 `#[serde(deny_unknown_fields)]`（另外三个 Arg 都有）⇒ 模型传 `timeout` 被静默忽略。
- `use std::ffi::OsStr` 未使用（`cargo check` 在报）；正好能把两层 `match` 压成
  `if Path::new(program).file_name() != Some(OsStr::new(program))`。
- `bail!(format!(…))` 是 clippy `useless_format`；`{}` 换 `{:?}`，空串/怪字符才看得见。
- `resolve_in` 文案 `"无法解析路径{p}"` 缺空格 ⇒ 模型看到 `无法解析路径nope.txt`。
- 回执总是打三段，`rg zzz_nomatch .` 得到 `命令退出码: 1` + 两个空标签。建议空段不打 ——
  但**标签必须留**，它是 `role:"tool"` 没有 `is_error` 时唯一能区分 stdout / stderr 的东西。
- 外侧 context「执行命令 `{}` 失败」→「无法执行 `{}`」：退出码全走 `Ok` 之后，`Err` 只剩
  「拒绝执行 / 起不来」，命令根本没跑。

#### 🔴 `env_clear` 白名单被推翻：改「继承 + 摘掉自己的 key」（2026-10-27，三派调研 + 端到端实测）

**起因**：跑真实 `cargo test` 时链接失败，报

```
note: link: missing operand after '\377\376'
      Try 'link --help' for more information.
```

`\377\376` 是 UTF-16 BOM ⇒ 跑到的是 PATH 上的 **GNU coreutils `link`**（`PortableGit/usr/bin/link.exe`），
不是 MSVC 的 `link.exe` ⇒ **rustc 没找到 MSVC 工具链，退回裸名搜索**。本机 PATH 上根本没有 MSVC 的
`link`/`cl`,也没有 `CC`/`LIB`/`INCLUDE`/`VCINSTALLDIR`,rustc 全靠环境变量 + 注册表探测 ⇒ **`env_clear` 把探测条件清掉了**。

分组实测（每组先删 `target` 强制重链，跑两轮结果一致）：

| 环境 | 链接 |
|---|---|
| 全量继承 | OK |
| `env_clear` + 6 变量（当时现状） | **FAIL → 撞 GNU link** |
| 6 + `ProgramFiles(x86)` / `ProgramFiles` / `ProgramW6432` / `COMSPEC` / `windir` | 全 FAIL |
| 6 + `ProgramData` | **OK** |
| 6 + `SystemDrive` | **OK** |
| `cargo check`（不链接）+ 6 变量 | **OK** |

⇒ **只有链接阶段受影响**。V0 验收用 `cargo check` 碰不到，但模型一跑 `cargo test`/`cargo run` 就废 ——
而「跑编译/测试并修」正是 V0 的目标任务，几乎必然踩到。

**第一步的修法是错的**：往白名单里补 `SystemDrive` + `ProgramData`（各自单独就能救回来）。
错在两点：① 这是 rustc 黑盒探测的副产物，跨版本、跨机器（装了 VS / 没装 VS）会变，**永远收敛不了**；
② 它掩盖了真正的病根 —— 白名单是**从零构造正集**。

**三派调研（这一轮才有结论）**：

| 派别 | 代表 | 默认 |
|---|---|---|
| 全量继承 | `system()` / Node `spawn` / Python `subprocess` / Rust `Command`（不传 `env`） | 什么都不做 |
| **继承 + 按名字减**（主流 agent CLI） | **Codex** `shell_environment_policy`、**Gemini CLI** `sanitizeEnvironment` | 先继承，再按模式剔 |
| 从零构造白名单 | `sudo` `env_reset`+`env_keep`、`cron`、`systemd`、`ssh SendEnv`、Bazel `--action_env`、`docker build` | 默认拒绝 |

Codex 的配置本身就是证据 —— **`inherit="all"` 是默认值，而默认剔除仍然开着**：

```toml
[shell_environment_policy]
inherit = "all"                  # 默认：克隆完整父环境
ignore_default_excludes = false  # 默认：剔除名字含 KEY / SECRET / TOKEN 的变量
exclude = ["AWS_*", "AZURE_*"]   # 追加 glob
include_only = ["PATH", "HOME"]  # 可选，最后一道白名单
set = { CI = "1" }               # 永远覆盖
```

Gemini CLI 同构：`const sourceEnv = shellExecutionConfig.env ?? process.env;` → `sanitizeEnvironment(...)`
→ 再补 `TERM`/`PAGER`/`GIT_*`，并强制 `GIT_TERMINAL_PROMPT=0`、`GIT_ASKPASS=''`。

⇒ **业界常见做法不是「去掉 config 里那几个」，是「按名字模式批量减」。**
但注意 Codex 对**外部进程**（MCP server）反过来用**白名单**（`env_vars`）⇒ 真正的分界是**信任边界**，
不是严格程度：同 UID 的本地工具链 → 继承+减；外部/不可信进程 → 白名单。

**为什么「清空+加」必然翻车 —— 这是集合封闭性问题**：

```
白名单（正集）  上界 = 「地球上所有工具链需要什么」  不可枚举 ⇒ 永远追不全
黑名单（负集）  上界 = 「我自己的凭证」              可枚举、闭合
```

`sudo`/`systemd` 敢用白名单，是因为它们跨的是**特权边界**（环境变量在那个边界上是攻击者的输入）。
agent 的子进程跟父进程**同 UID、同会话**，这里的白名单给的「最小权限」是错觉。

**env 清理的真实目标（这条想清楚了才算懂）**：不是「构造最小权限环境」，而是

> **让自己的凭证不出现在「会被回显的地方」**：子进程 stdout → 回执 → history → **每一步都重新发给模型服务商**。

任何测试里写一句 `dbg!(std::env::vars())`，key 就进了 history 并长期驻留。所以目标是那 3 个**已知** key
（封闭集），不是无界集合。至于蓄意 exfiltration —— 挡不住：`run_command` 的子进程**没有文件系统沙箱**，
`cargo` 能直接读 `my_agent/.env` 拿走 key；那要靠 V1 的 OS 沙箱 + 网络禁用。

**定案（已落地）**：

```rust
// config.rs —— 单一来源，紧挨 from_map
pub const LLM_ENV_KEYS: &[&str] = &["LLM_BASE_URL", "LLM_MODEL", "LLM_API_KEY"];

// tool.rs::run_command —— env_clear 与 8 项白名单整段删除
for key in config::LLM_ENV_KEYS {
    cmd.env_remove(key);
}
```

`env_remove` 收 `AsRef<OsStr>`，`for key in &[&str]` 得到 `&&str` 也能过（`&T: AsRef<U> where T: AsRef<U>` 的
blanket impl），已实测编译通过。

**端到端实测（走 `Executor::run` 公开入口，父进程先 `set_var("LLM_API_KEY", ...)`）**：

| 用例 | 结果 |
|---|---|
| `cargo run` 打印环境 | `LLM_API_KEY`/`LLM_BASE_URL`/`LLM_MODEL` 全 **absent**；继承 **254** 个变量；**链接成功**（`cargo run` 必须链接才跑得起来） |
| `cargo test`（1200 行日志 + 末尾断言） | 回执 8242 字节 / 598 行，`这就是真正的诊断` / `panicked` / `test result: FAILED` / `中间省略` 全在 |
| `cargo check --message-format=short`（200 个 E0308） | 回执 8199 字节 / 105 行，可见 **99** 个错，尾部还有 `due to 200 previous errors` |
| `jq --version` / `rg --version` | 正常，44 / 194 字节，小输出不截断 |

**残留风险（记录，不改）**：

1. **`dotenvy::dotenv()` 会写进程环境** —— 源码 `iter.rs:35`：
   ```rust
   if env::var(&key).is_err() { env::set_var(&key, value); }   // 只有「键原本不存在」才写 = 「真 env 优先」
   ```
   ⇒ `.env` 内容进入进程环境。今天 `.env` 只有那 3 个 key（已核实），所以 `env_remove` 3 个就闭环；
   **但将来往 `.env` 加任何别的 key（如 `GITHUB_TOKEN`）就会静默漏**。
   彻底解 = `dotenvy::dotenv_iter()` / `from_filename_iter()`（源码里只有 `load()` 才 `set_var`），
   让 secret 根本不进环境。代价两条：① `dotenv_iter()` **不调 `remove_bom()`**（BOM 会污染第一个 key 名）
   ② 「真 env 优先」要自己实现（`Config::from_map(&get)` 正好是这个注入点 —— functional core 的价值在这里兑现）。
   ⇒ **列为 V1 前可选升级，V0 不做。**
2. **回执比名义预算多 ~84~120 字节**（2026-10-30 四分支实测：`flat` 8245 / `longline` 8312 / `oneline` 8294 / `crlf` 8276
   vs `max_bytes` 8192）：省略标记是切完之后**追加**的 ⇒ `max_bytes` 的语义是「**正文预算**」，不是「整份回执硬上限」。
   **2026-10-30 定案（用户指出，推翻「是个待修的小问题」）**：这不是问题 —— 该常量管的是「工具结果本身能占多少字节」，
   尾注是元信息，长度由我们自己控制且有界（≤~120B），不该为它引入「先算文案长度 → 再按剩余预算切正文」的两遍耦合。
   真要硬上限，**调小常量**比在截断里预留更直白。⚠️ 唯一耦合：`write_file` 的大文件拒写守卫读的是**同一个** `max_bytes`，
   调小它会连带压低「整文件重写」的允许阈值 ⇒ 真要动这个数，得先把它拆成两个参数（正文预算 / 拒写阈值）。
   代码侧只需一句注释写明「这里是正文预算」；改名 `max_body` 属可选美化。
3. `LLM_ENV_KEYS` 与 `from_map` 的键名仍是**两处写**。可以加一条测试钉住一个方向（清单里每个 key 都能让
   `from_map` 报 `Missing`），但危险的那个方向（新增字段忘了加清单）挡不住 ⇒ 只能靠**紧挨着写 + 注释**。
   别为这个把 `from_map` 改成 `ENV_KEYS[i]` 索引数组，更难读且加第 4 个字段照样忘。

### 3.6 验收标准

**功能**：在 scratch 里放一段编译不过的代码（用 `my_api` 的副本），对 agent 说"让它通过编译"，它自主完成
**读文件 → 改 → `cargo check` → 读报错 → 再改**，**中途不需要你介入**。

**"能真实用起来"是验收标准，不是范围驱动** —— 它只问一件事：**做完之后，你愿不愿意每天真的打开它？**
门槛比想象的低：**比的不是功能数，是可靠性**。一个只会做一件事但每次做对的 agent，胜过 20 个工具却总改错文件的框架。

> 需求的正确来源是 **dogfooding，不是设计**。三个小时之内你会亲自撞到真正的需求：上下文塞不下、它改了错的文件、
> 它在同一个错误上循环。**这些是设计不出来的。**（就像你在 T2 里第三次才撞到"提权时序"一样。）

#### 3.6.1 靶场就绪（2026-10-27）

`scratch/my_api` = `my_api` 本体副本（含 `.env` / `data/users.db` / `migrations` / `target`，`scratch/` 已 gitignore）。
开关脚本在**工作区**（不污染沙箱）：`D:\WorkSpace\AICode\WorkBuddy\rust_learn\tools\scratch_bugs.py`

| 目的 | 命令 |
|---|---|
| 注入 3 个编译错误 | `uv run tools/scratch_bugs.py inject` |
| 复位成可编译 | `uv run tools/scratch_bugs.py clean` |
| 查看当前靶状态 | `uv run tools/scratch_bugs.py status` |

注入的 3 个错误（**全部落在 8192 字节以下的文件里**，理由见 3.6.3）：

| # | 位置 | 编译器报错 | 修法 |
|---|---|---|---|
| 1 | `handler.rs:22` | E0308 `expected &Pool<Sqlite>, found Pool<Sqlite>` | 补 `&` |
| 2 | `models.rs:52` | E0063 `missing field email in initializer of PublicUserResponse` | `From` 里补 `email: value.email` |
| 3 | `response.rs:56` | E0308 `expected String, found &str` | `message: message.to_string()` |

三条都是「重构做到一半」的真实形态，彼此独立、互不遮蔽。`--message-format=short` 全量回执 **451 字节**，
一次看全、不触发截断；长格式下第 1 条还会在 `note:` 里打出 `auth.rs:198` 的完整签名 ⇒
**模型不必读 `auth.rs` 也能修**（刻意避开长文件）。已验证：按上表修完 → `cargo check` **EXIT=0**，无连锁错误。

#### 3.6.2 开跑前必须确认的 4 条（任一不满足，验收结果作废）

1. **`max_steps` 够用** — `main.rs` 现为 50（原来 5 连「读→改→check→读错→改」都不够）。步数耗尽时结论会丢，是已知 V1 债。
2. **副本必须带 `.env`** — `my_api` 有 **8 处 `sqlx::query!`** 编译期宏，`DATABASE_URL` **只能**来自 `my_api/.env`
   （实测：父进程 env 为空、无 `.sqlx` 缓存、**移走 `.env` 立刻炸 7 条 `set DATABASE_URL to use query macros online`**）。
   这类错误**模型改代码永远修不好**，会死循环，最后被误判成「agent 不行」。
3. **cwd 无关，但 manifest 要对** — 实测 sqlx 从 **manifest 所在目录**读 `.env`，不是进程 cwd ⇒
   从 `scratch` 根跑 `cargo check --manifest-path my_api/Cargo.toml` **可行**。
   这一条必须成立：`run_command` 有 `.current_dir(root)` 而 `{program,args}` 无 shell、**不能 `cd`**，
   所以 agent 只能用 `--manifest-path`。
4. **`target` 要么带要么预热** — 没有 `target` 时首次 check 要编 200+ crate、几分钟；而 `wait_with_output()`
   **没有超时**，看起来就是卡死。现副本随 `target` 一起拷贝（2.6G），实测冷启 1.6s、增量 1.3s。

#### 3.6.3 已知限制：长文件「读一半再整文件写回」会静默丢内容

`read_file` 是**头部**截断（`MAX_TOOL_OUTPUT` = 8192），`write_file` 是**整文件覆盖** ⇒
对一个 >8192 字节的文件「读一半 + 写回」= 后半段消失，而回执仍然报「成功写入 N 字节」，看不出异常。
实测 `my_api/src/auth.rs` = **18,488 字节**，`truncate_head_tail(8192)` 后中间 **10,368 字节**不可见
（该区段里 `SqlitePool` 出现 7 次）。

⇒ **本次验收只挑 8192 以下的文件注入**（`auth.rs` 是唯一雷区，其余 12 个文件全在 4828 及以下）。

##### 为什么「只给 `read_file` 加 `offset`」是半个解（2026-10-27 用户指出）

这是两个**独立**的缺口，`offset` 只补前一个：

| 侧 | 缺口 | 谁补 |
|---|---|---|
| **读** | 8192 之后看不见 | `read_file` 的**行号** `offset`（可选） |
| **写** | **只能整文件覆盖**，没有局部修改能力 | **`edit_file(old,new)`（关键，新工具）** |

只加 `offset` 时，模型会走一条**语义正确但代价不可接受**的路：分三块读完 18KB → 把 18KB **逐字回写**。
- 输出侧：≈6k token 只为抄一遍原文；
- 正确性侧：转写时漏一行、改个缩进，**没有任何地方能发现**；
- 观测侧：回执仍是「成功写入 18488 字节」，与「抄对了」逐字相同。

⇒ **承重的是写侧**。`edit_file` 让「写」不再要求「手里有全文」，这才是闭合条件。

**「插入」不需要独立工具**：它就是 `old_string` 挑锚点、`new_string` 里多写一段。
⇒ 也**不要**给 `write_file` 加 `mode: append / insert / replace` 参数（那是把三种语义塞进一个工具，
调用点读不出 `mode` 的取值，同 §3.4「单一策略决定点」那条）。

`edit_file` 的形状 = **三态匹配**，回执必须各自可行动：

```rust
let n = content.matches(old_string).count();
0 => bail!("未找到要替换的文本（逐字节匹配，含缩进；可能是行尾符差异）"),
1 => 写回 content.replacen(old_string, new_string, 1),
_ => bail!("匹配到 {n} 处，请把 old_string 扩大到能唯一定位的上下文"),
```

三个坑（前两个是设计，第三个是平台）：
1. **必须唯一匹配**，不是「替换第一个匹配」——否则模型永远不知道它改的是不是那一处；
2. `new_string` **可以等于** `old_string` 前缀（插入语义天然如此），不要加「有变化」的校验；
3. **行尾符**：`old_string` 用 `\n`、文件是 CRLF ⇒ 0 匹配。实测 `my_api/src` **13 个文件全是 LF**（CRLF 计数 0），
   所以当前不踩；但 0 匹配的文案里必须提到这个可能，否则模型会去重读文件、再失败、循环。
   （业界对照：Claude Code `Edit` / Gemini `replace` 都走参数化 `old/new`；Aider 走 SEARCH/REPLACE 文本区块，
   我们走前者是因为工具调用**已经是结构化 JSON**，不需要再解析文本区块。）

##### 只有 `edit_file` 仍不够：`write_file` 的拒写守卫

`edit_file` 存在 ≠ 模型会用。它可能在 >8192 的文件上继续 `write_file` 盲写。故在 `write_file` 的
`Ok(meta)` 分支（那里已有 `meta.len()`，零额外 syscall，且紧跟在 `is_file()` 之后）加一道
（签名已改成三参数：`fn write_file(path: &Path, content: &str, max_bytes: usize) -> Result<usize>`）：

```rust
// Ok(meta) 分支内、is_file() 判断之后
if meta.len() > max_bytes as u64 {
    bail!("文件过大，超过 {max_bytes} 字节，请使用 edit_file 工具");
}
```

##### 守卫阈值：传参还是引常量（2026-10-27 用户质疑后**改判**）

旧结论「判据**直接引常量**、不传参，否则改一个忘另一个」**作废** —— 那是**纪律型**论证：它没有指出任何
类型 / 结构层面的强制，只是「希望你记得」。可判定的规则只有一条：

> **这个值将来会不会变？看它是 `const` 还是 `Executor` 的字段。字段的存在本身就是「它会变」的授权 ⇒ 读字段。**

审计（当时 `max_bytes` 全文 4 处）：声明 `tool.rs:314`、`new()` 里赋常量 `323`、`run()` 里读两次 `332/333`；
**无 setter、无 CLI/env 入口、`write_file` 看不到它**。⇒ 那一刻「引常量」与「传字段」**运行期完全等价，一个字节不差**。
但它既然被写成 `Executor` 的字段，就该读字段 —— 否则它不配做字段，该是 `const`。**用字段的意图去定参数的真假。**

⇒ 落地为 **B**：`write_file(path, content, max_bytes)` + `raw_invoke(&self, root, max_bytes)`（`max_bytes` 与 `root`
同属「执行环境」，加进签名不破坏形状；不变量从「两处写同一个数」变成「一个字段被读两次」，后者是结构性的）。

一致性论据也支持 B：`truncate_head` / `truncate_head_tail` 的参数之所以**必须**是参数，是因为测试要传 64 / 128 这种小值；
守卫同理 —— 传 `max_bytes` 后测试可构造 `max_bytes = 16` + 20 字节文件验证拒写；写死常量则只能真的造 8193 字节文件，
**那时测的是常量值，不是逻辑**。

⚠️ **两个参数同名 ≠ 同一件事**：`truncate_*` 收的是**预算**（这次最多放多少字节出去），守卫收的是**阈值**
（多大就算「你不该见过全文」）。今天数值相同，靠的是一条**语义等价**：`read_file` 会截断 ⟺ 文件超过单次可见上限
⟺ 模型不可能见过全文。这条等价在加 `offset`/`limit` 之后会漂 —— 用 offset 分块读完 18KB 的模型**其实见过全文**，
守卫照拒。这正是下面「代价」那条、也是 V1 换覆盖率追踪的**同一个缺口**。

其余三条设计点：
- **`Err(NotFound)` 分支不守卫**：新文件没有「现有内容被截掉」的风险，拦住它只会妨碍建文件。
  守卫的定位是「静默毁 10KB → 明确拒绝 + 指路」，不是「不许写大文件」。
- **代价**：合法的大文件整体重写也会被拒（模型分块读完想整体写回）。V0 的任务是「修编译错误」，这条路本来不该走；
  V1 的正确形态是 Claude Code 那种「Edit/Write 前必须 Read 过」的**覆盖率追踪**，那时才有状态可判。
  **现在这条守卫的收益是把「静默毁 10KB」换成「明确拒绝 + 指路」**，这是纯增益。
- ⚠️ **现文案缺两样可行动信息**：① 文件实际多少字节（模型据此判断「差一点」还是「差一倍」）；
  ② **为什么**不能写（「你只可能看过前 `max_bytes`，尾部没见过」）—— 没有 ②，模型可能改成「先 `read_file` 再写」的
  同一错误循环。两句都能从现有变量拼出来，属纯文案，待补。

##### edit_file 的代价：它买的是**可验证性**，不是内存（2026-10-28 实测）

**机制**：`read_to_string` → 内存替换 → `fs::write` 全文写回。用户问「那对大文件还是得过内存的，只是不让 LLM 接触过多文本？」
—— 机制对，**数量级不对**。实测（18,488 字节，release，200 次平均）：

| 步骤 | 单次耗时 |
|---|---|
| `read_to_string` | 146 µs |
| `matches().count()` + `replacen()`（纯内存搬运） | **6.2 µs** |
| `fs::write` 落盘 | **22 ms** |

内存搬运占 **0.03%**，耗时几乎全在写盘（22ms 是本机 Defender + D: 盘的值，不代表普遍；但**比例**成立：I/O 比内存贵 3~4 个数量级）。
峰值分配稳定 **2.00×** 文件大小（`read` 的 `String` 与 `replacen` 新分配的 `String` 同时存在）：
18.5KB → 37KB / 23ms；10MB → 21MB / 34ms；50MB → 105MB / 304ms。
要压到 1× 可用 `String::replace_range(pos..pos+old.len(), new)` 原地改不重分配；但 `replace_all` 多处索引会移位、得倒序处理，V0 不值得。

**真正的收益不是省 token，是「谁在执行搬运」**：

| | `write_file` 整文件覆盖 | `edit_file` 局部替换 |
|---|---|---|
| 未修改部分经过谁 | 模型**逐字重新生成** | **进程内存**，逐字节就是原值 |
| 出错位置数 | 全文每个字节 | 只有匹配点与 `new_string` |
| 出错有信号吗 | **没有**（回执永远「成功写入 N 字节」，与抄对了逐字相同） | 有：0 处 / n>1 处直接 bail |
| 模型必须「见过全文」吗 | **必须** | **不必** |

最后一行是承重的：`write_file` 守卫能拒掉 >8192 的文件，正因为整文件覆盖的正确性**依赖「模型手里有全文」**；`edit_file` 的正确性不依赖这个。
⇒ **它不是「能读更多」，而是「不需要看见全部也能改对」。** 省下的 token（18KB 逐字回写 ≈5-6k vs 局部改 ≈40，差 150×）是让正确做法
在经济上可行的**条件**，不是目的本身。

边界两条：① 只有文件大到 `read_to_string` 装不下（GB 级）时「过内存」才成问题，那时该上 `memmap2` / 流式，代码沙箱不会出现；
② `fs::write` 是 **truncate 后写、非原子**，中途崩 = 文件残缺（`write_file` 一模一样，是两者共同欠的债）。

##### 回执的行号：为什么只报「起始行」（2026-10-28 实测）

**不变量**：替换区之前**一个字节都没动** ⇒ 前缀里 `\n` 的个数不变 ⇒ **起始行号在旧文本与新文本里是同一个数**。
这是唯一不漂移的锚点，也正是「选起始行而不是结束行」的理由。

| 场景 | 在旧文本上算 | 替换后同偏移算 | 新文本里的真实位置 |
|---|---|---|---|
| 单处替换（换成 3 行） | 4 | 4 | 4 |
| `replace_all` 第 1 处 | 2 | 2 | 2 |
| `replace_all` 第 2 处 | 4 | **3** | **5** |

⇒ 三条落法（已实现）：
- `replace_all = false`：报**起始行号**，取自 `base_file.find(old_string)`（此时 bail 已保证唯一）。
- `replace_all = true`：**不报行号** —— 改多处，一个行号会让模型以为「都在这附近」，只报处数 + 总行数变化。
- 🔴 **绝不在 `new_file` 上用 `new_string` 反查位置**。实测两个反例：`new_string = ""`（删除）时 `find("")` 得 0 ⇒ 报「第 1 行」
  （实际第 3 行）；`new_string` 是文件里早就出现过的常见串（如 `Ok(())`）时，报的是**文件里第一处** —— 实测报第 2 行、实际第 305 行，差 303 行。
  （2026-10-28 首版实现踩过这个坑，已改为数旧文本。）

##### edit_file 落地后的 review（2026-10-28）

逻辑已正确：空串先判 → 0 处 / n>1 都在**写盘之前** bail → 行号取自 `base_file`。余下：

| 类 | 项 |
|---|---|
| **文案（影响自纠）** | ① 0 匹配缺行尾符提示（本章硬要求：不提 CRLF，模型会重读再失败再循环）② n>1 文案错别字「已确保」→「**以**确保」，且缺第二条出路「若确实要全部替换，把 `replace_all` 设为 `true`」③ `raw_invoke` 仍打 `diff 为 :`，而回执已是「替换 N 处; 总行数 …」—— 名不副实 |
| **语义（现在对，靠巧合）** | `base_file.replace()` 是**全替换** API，却用在 `replace_all = false` 分支；结果正确只因 bail 保证了 n==1。应显式 `if replace_all { replace } else { replacen(.., 1) }`，否则将来放宽唯一性检查会**静默变成全改** |
| 风格 | `re_line` 在 `replace_all` 时白算一次（挪进 else 分支即可）；`&base_file[..pos].bytes().filter(..).count() + 1` 里的 `&` 是噪声 |
| 可留 | 目录经 `read_to_string` 报 os error 5（文案「读取文件失败」给了线索，但看不出是目录）；末尾追加换行不改变 `lines()` 计数 ⇒ 这类改动报「总行数 0 变化」 |

#### 3.6.4 验收记录：**通过**（2026-10-27，15 步）

任务横幅（`main.rs`）：`查看并尝试修复沙箱下的my_api文件夹内的rust错误,临时文件请放在沙箱下的tmp文件夹内。
可以使用"cargo check --message-format=short"命令减少输出。`　`max_steps = 50`（实际用 **15 步**）。

**结果（我方独立复核，不采信 agent 自述）**：

```
cargo check --all-targets --message-format=short   → EXIT=0，零代码告警
cargo test                                        → 21 passed + 17 passed = 38 passed，0 failed
diff -rq my_api/src  scratch/my_api/src           → 只有 models.rs 不同（+1 行注释，59 字节）
```

| 注入的错误 | agent 的修法 | 判定 |
|---|---|---|
| `handler.rs:22` E0308 | 补 `&`（并对照同文件 `get_me`/`delete_me` 的写法） | 与本体**逐字节相同** ✔ |
| `models.rs:52` E0063 | **删掉 `email` 字段** + 补注释，而非补 `email: value.email` | 与本体差 1 行注释 ✔（见下） |
| `response.rs:56` E0308 | `message: message.to_string()` | 与本体**逐字节相同** ✔ |

**`auth.rs`（18,488 字节，§3.6.3 的长文件雷区）一个字没动** —— 3.6.3 预判的风险没有发生。

**三个超出"让编译器闭嘴"的行为（值得记，因为它们决定 V1 的设计）**：

1. **第 2 个错误它没走最短路径**。它读了 `tests/http.rs::test_get_user`，判断「公开接口 `GET /users/{id}` 不该返回邮箱」，
   于是**删字段**而不是补字段，并主动写出反向改法（「如果你的本意是公开接口也要返回邮箱，那么反向改一行即可」）。
   ⇒ **V0 的验收目标「通过编译」不足以约束语义**：它改的是**接口契约**，不是恢复原状。这次它猜对了（本体本来就没有 email，
   我注入的 `email` 字段是凭空加的），但这条说明「语义正确性」不在 V0 的能力范围内 —— V1 要补的是「改契约前先确认意图」。
2. **自己决定跑 `cargo test`**（38 个测试、约 20 秒），不止于 `cargo check`。这同时验证了两个既有决定：
   「退出码全走 `Ok`」（否则 test 失败会被当工具故障）+ 「头尾截断」（测试失败诊断在尾部）。
3. **意识到「没有删除工具」并自行绕过**：原文 *"can't delete files with tools (no rm). I'll overwrite it with a useful note."*
   ⇒ 这是 V0 工具集缺口的直接证据（`tmp/fix_handler.rs` 最后是一行笔记，98 字节，没有残留垃圾）。

**代价与遗留**：15 步里它把 `cargo test` 跑了不止一次（约 20s/次），用时主要花在测试而非编译上。
`tests/` 目录在 `scratch/my_api` 里也一并被拷进来了 —— 这次是好事（给了它语义线索），但也意味着**靶场不是"只有 src"的最小面**。

**靶子设计教训（下次改进）**：错误 2 有**两条合法修法**（补 `email: value.email` / 删掉 `email` 字段），
所以它**不能用来判「改得对不对」**，只能用来观察「它会不会自己找语义依据」。
`handler.rs`（补 `&`）、`response.rs`（`.to_string()`）那两个才是**唯一解**的靶子。
⇒ 下次要测「意图确认」，应该**单独设计一个必须提问的场景**，而不是靠歧义错误碰运气。

#### 3.6.5 验收之后：V0 收尾定序 + 「意图确认」为什么必须拆成两件事（2026-10-27）

**① 收尾顺序（覆盖 §〇 早先的「日志 → 单测 → offset」）**

| 序 | 动作 | 理由 |
|---|---|---|
| 1 | **`edit_file` + `write_file` 大文件拒写守卫** ✅ **均已落地**（守卫 2026-10-27 / `edit_file` 2026-10-28） | 这两条**必须一起**，才把 §3.6.3 的静默丢内容**结构性**闭合（不是靠文档嘱咐） |
| 2 | `read_file` 的**行号** `offset` ✅ **2026-10-30 已落地** | 解锁「尾部看不见」；在 1 之后它才是可选的加分项。设计见 ⑤，四分支尾注见 ⑦ |
| 3 | ~~日志 `→ 工具名(参数)` / `← N 字节`~~ ✅ **2026-10-30 已做、形态改判**：`Logger` + `LogKind` + `LogSink`，文案集中在 `log.rs`、`Executor::run` 保持静默（见 **§3.6.6**）；`truncate_middle` / `resolve_in` 单测**仍未做** | 可观测性 ✅ / 防回归 ⬜ |
| 4 | **操作审批钩子**（见 ②） | 需 3 完成后才好观测「拒绝了什么」 |
| 5 | 交互式 REPL + 语义追问 | V1 |

**② 「意图确认」不是一件事，是两件**

| | 是什么 | 现在能做吗 | 测试怎么做 |
|---|---|---|---|
| **(a) 操作审批** | host 在 `write_file`/`run_command` **之前**拦住，打 diff / 待执行命令，等 y/n | **能**，且**不需要终端也能测** | 注入 `confirm: impl Fn(&Invocation) -> bool`，测里传 `\|_\| false` |
| **(b) 语义追问** | 模型**自己**停下来问「要补 `email` 还是删字段」 | 不能，要等多轮 REPL | 单发运行时它问完进程就结束了，无法追问 |

(a) 的形状（关键在**注入**，不在读 stdin）：

```rust
struct Executor { root: PathBuf, max_bytes: usize, confirm: Box<dyn Fn(&Invocation) -> bool> }
// CLI 传「打印 + 读一行 y/n」；测试传 |_| false
```

⇒ **审批逻辑这一层完全无交互可测**，缺的只有读 stdin 那 4 行。所以「必须先做交互式 CLI」只对 (b) 成立。

**③ 更根本的一条：审批不能靠模型自觉**

Codex 是 `approval_policy`（`untrusted` / `on-request` / `never`）+ sandbox、Claude Code 是 permission prompt / plan mode，
**都是 host 强制**。「靠 prompt 让模型自己决定要不要问」不是可测的工程属性，成功率随模型波动 —— 它只能缓解，
不能作为验收项。故 (b) 的正确形态也不是 prompt，而是「循环停在某个状态、把控制权交回人」。

⇒ 文档口径修正：§3.7 分档表里「dry-run 确认」原本笼统列在「V0 就做」，现拆为
**(a) 操作审批 = V0 收尾**、**(b) 语义追问 = V1–V2**。

**④ 单测怎么写：`FileHelper` 与「准备/清理不做断言」（2026-10-28 落地）**

给 `edit_file` 铺用例时，第一行测试就暴露**测试本身不可信**：`test_resolve_write` 硬编码 `scratch/tmp/new.txt`，
跑完留残留 ⇒ 第二次运行时 `resolve_for_write` 走「目标已存在」分支，`write_file` 的 `create_dir_all` 重试分支
**再也执行不到**。测试绿着，覆盖的分支已经换了。

实证（同一个测试、同一份代码，只改前一次的残留）：

| 前一次留下的状态 | 本次结果 |
|---|---|
| 干净 `scratch/tmp` | ok |
| 上一轮的 `new.txt` 被设为只读 | **FAILED**：`assertion failed: s.is_ok()`（`tool.rs:388`）|
| 清干净后复跑 | ok |

⇒ **helper 要解决的是「从已知干净状态开始」（幂等性），不是「创建和删除文件」。**

形状（`mod test` 内，不引 `tempfile`）：

```rust
struct FileHelper { path: PathBuf }
impl FileHelper {
    const TMP_ROOT: &str = "tmp";
    fn new(case: &str) -> Self {
        let dir_path = PathBuf::from(Self::TMP_ROOT).join(case);
        let _ = fs::remove_dir_all(&dir_path);          // 不存在/是文件都忽略
        fs::create_dir_all(&dir_path).expect("初始化临时目录失败");
        let root = fs::canonicalize(&dir_path).expect("临时目录 canonicalize 失败");
        Self { path: root }                             // 存 canon 过的
    }
    fn root(&self) -> &Path { &self.path }
    fn get_path(&self, p: &str) -> PathBuf { self.path.join(p) }
    // create_dir / delete_dir / create_file / delete_file：一律 let _ = ，只有 create_dir_all 保留 expect
}
impl Drop for FileHelper { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.path); } }
```

四条判据：

| 判据 | 理由 |
|---|---|
| **`canonicalize` 收进 `new`，只算一次** | 把「测试形状 ≠ 生产形状」这个已踩过三次的坑**结构性**消掉；`root()` 直接给 canon 过的路径，用例不用再自己 canon |
| **目录名 = 用例名，不用随机名** | 唯一性靠用例名（`cargo test` 默认并行）；**同名 + 开头清空是自愈的** —— 进程被 kill 时 `Drop` 不跑，随机名会永久积累 |
| **`Drop` 用 `let _ =`** | 见下「陷阱二」：Drop 里 panic 在 unwind 中会 abort |
| **准备/清理只「搬状态」，不对搬的结果下断言** | 「确保不存在」「确保存在且为空」都是幂等操作，`NotFound` 是无害的；只有 `create_dir_all`（后续全靠它）才值得 `expect` |

**陷阱一：`let _ = expr.expect(..)` 不是「忽略错误」**（2026-10-28 实际踩到）

```rust
let _ = fs::remove_dir_all(&dir_path).expect("清除路径失败");   // ✗ 等于没改
```

`expect` 签名 `fn expect(self, msg) -> T`：吃 `Result<T,E>`、**返回 `T`**。⇒ `Err` 在到达 `let _ =` 之前就 panic 了。
**自查判据：`let _ =` 后面的表达式里不该再出现 `.expect()` / `.unwrap()` / `?`**，三者都比 `let _ =` 先作用。
正确写法就是 `let _ = fs::remove_dir_all(&dir_path);`。

**陷阱二：`Drop` 里 `expect`，在断言失败时会 abort 整个进程**（实测）

| 情形 | 表现 |
|---|---|
| 测试正常结束、仅 `Drop` panic | 正常失败，报 `Drop 里的 panic`，exit 101，其余用例照跑 |
| **断言失败（正在 unwind）+ `Drop` panic** | **abort**：`exit code: 0xc0000409 STATUS_STACK_BUFFER_OVERRUN` |

第二种的完整输出只有三行：断言信息**一个字都没打出来**、用例名后面连 `FAILED` 都没有、**排在后面的用例根本没跑**。
`let _ =` 对它零成本，`expect` 的代价是「失败时丢掉失败信息」—— 代价不对称到不需要讨论概率。

**陷阱三：`create_dir` 在「新克隆的仓库」里必然炸**

`tmp/` 被 `.gitignore` 忽略 ⇒ 新克隆下来**必然没有**。实测 `rmdir tmp` 后 4 个用例全部
`panicked at tool.rs:392: 初始化临时目录失败: Os { code: 3, kind: NotFound }`。
`create_dir_all` 同时解决三件事：父目录自动建、已存在返回 `Ok(())`（避开 `AlreadyExists(183)` 的并发竞态，实测）、
不需要「先探存在再建」的辅助函数。

**断言要求（两条硬规矩）**

1. **断言值，不断言 `is_ok()`**：`write_file` 返回 `usize` 就是为了让用例写 `assert_eq!(s.unwrap(), 1)`
   ——「成功但写了 0 字节」是它唯一要防的情况；写 **`assert!(!is_ok())` 等于什么都没测**。
2. **否定路径要断言「原文未变」+ 错误关键词**：现有 `assert!(p.is_err())` 只测了「有错」，哪天 `resolve_for_write`
   因为**别的**原因也 `Err`，用例照样绿。正确形态 `let err = s.unwrap_err(); assert!(format!("{err:#}").contains("越界"))`。

**用例分类（先说清哪些根本不用碰文件）**

| 类别 | 用例 | 需要造文件吗 |
|---|---|---|
| 纯路径代数 | `../` 越界 / 绝对路径 / 空串 | **否**，只要一个真实存在的 root |
| 需要文件系统 | 写入成功 / 父目录不存在 / 目标是目录 / 守卫触发 | 是 |
| `edit_file` 待铺（8+2） | 唯一替换 · 插入 · `replace_all` 全替换且回执含处数 · 0 处 **且原文未变** · n>1 且 `replace_all=false` **且原文未变** · 空 `old_string` · 文件不存在 · `new_string=""` 删除成功**且回执行号 ≠ 1** · 中文多字节 · 回执含 `起始第 N 行` | 是 |

正向用例的期望值必须**独立算出**（`input.replacen(old, new, 1)`），不复用被测代码的中间变量，否则测试与实现同错。

**⑤ `read_file` 的 `offset`：五家调研 + `truncate` 该返回结构体（2026-10-29）**

给 `read_file` 加 `offset` 看着是「切一下」，实际撞上一处接缝：`truncate_head` 的文案里写着「原文共 M 行」，
而它的 `total_lines` 是**传进来那份 `String`** 的行数 —— 一旦 `read_file` 先按 offset 切片，这句就变成**假话**
（模型读到「原文共 567 行」，文件其实 589 行 ⇒ 它会认为读完了）。**这是结构问题，不是文案问题。**
故先调研五家，再定实现。

**主表**

| | 参数 | offset 基准 | 输出行号前缀 | 默认上限 | 越界 |
|---|---|---|---|---|---|
| **Claude Code** | `file_path` `offset` `limit` | **1-based** | ✅ `cat -n`：空格+行号+Tab（正则在用的形态 `/^\s*\d+\t(.*)$/`） | 2000 行 / 单行 2000 字符 | 空文件返回空标记 |
| **Gemini CLI** | `file_path` `start_line` `end_line` | **1-based**，**end 含在内** | ❌ **不加** | `DEFAULT_MAX_LINES_TEXT_FILE` | `start>end` 报错；越界 `Math.min` **静默收敛** |
| **opencode** | `filePath` `offset` `limit` | 🔴 **0-based** | ✅ `00001\| 内容`（5 位补零） | 2000 行 / 单行 2000 字符 | 未确证 |
| **pi** | `path` `offset` `limit` | **1-based** | ❌ **不加** | **2000 行 / 50KB，先到先得** | **抛错**：`Offset N is beyond end of file (M lines total)` |
| **Codex** | ⚠️ **未见原生 read 工具** | — | — | — | — |

Codex 这一格是**存疑**而不是「没有」：`codex-rs/core/src/tools/handlers/` 里实际只有 `apply_patch.rs`、`view_image.rs`、
`unified_exec.rs`、`shell_spec.rs`、`mcp.rs`，**没有 `read_file.rs`**；第三方资料说某版本有 `read_file(path, limit, offset, timeoutMs)`，
但读不到源码 ⇒ 基准与格式不可信。能确认的是它**主线 shell 优先**（读文件靠 `sed -n` / `cat`），这也解释了为什么它给了
`apply_patch` 却没有原生 read handler。

**三个分歧点**

| 分歧 | 分布 | 判据 |
|---|---|---|
| **1-based vs 0-based** | 4:1（opencode 是唯一 `args.offset \|\| 0`） | 五家**都把 `1-indexed` 写进 description** —— 因为训练数据里两派混着，不明说模型就会把 22 传成 21 |
| **行号前缀** | 带：Claude Code、opencode；不带：Gemini CLI、pi | 带的这两家有**下游引用需求**（Claude Code 的 permission/UI 要定位；opencode 接了 **LSP**，诊断天然带行号）。**但五家全部在尾注报 `A-B of N`** ⇒ 行号前缀是可选装饰，尾注是必需品 |
| **越界** | pi 抛错带总行数 / Gemini 静默收敛 | **pi 对**：空返回让模型分不清「文件是空的」和「跳过头了」 |

**🔴 承重发现：pi 的 `truncate` 返回结构体，完全不拼文案**

```ts
export interface TruncationResult {
  content: string;
  truncated: boolean;  truncatedBy: "lines" | "bytes" | null;
  totalLines: number;  totalBytes: number;
  outputLines: number; outputBytes: number;
  lastLinePartial: boolean;  firstLineExceedsLimit: boolean;
  maxLines: number;  maxBytes: number;
}
```

文案在 `read.ts` 里现拼：`[Showing lines {a}-{b} of {total}. Use offset={next} to continue.]`；
而 `bash.ts` 调**同一个** `truncateHead`/`truncateTail`，拼**自己**的文案。

⇒ 我上一轮给的「B 方案（把截断下沉进 `read_file`）」**只对了一半**。pi 的划法更干净：

> **截断函数只算数（返回结构体），文案由知道 offset 的调用方拼。**

这正是「core 里不许按策略分支」那条原则推到底：**谁产生文案，谁必须知道策略**。文案里带 `start_line`/`total`、
带 `RunCommand` 的「头 3/4 + 尾 1/4」，就说明文案不属于截断函数。三个可直接搬的细节：

- **Never returns partial lines**：头截断宁可少给一行，不给半行（现有 `LINE_SNAP_MAX = 256` 是同一意图的宽松版）
- **`firstLineExceedsLimit`**：返回**空 content + 标志位**，由调用方提示 `[Line 42 is 200 KB, exceeds 50 KB limit...]`
  —— 正好覆盖我们的「20KB 无换行单行」场景，而不是把残片塞给模型
- **`splitLinesForCounting` 末尾要 pop `\n`**，否则多算一行（= Rust `lines()` 的行为）

**两个「业界也踩」的坑**

- **Gemini CLI 的 CRLF bug**：`content.split(/\r?\n/)` 再 `join('\n')` ⇒ **带 offset 的读取会破坏 CRLF 文件**，
  模型拿到的内容与磁盘不一致。它的 `isTruncated` 还把「请求了 offset」当成「被截断了」。⇒ 见下「陷阱二」。
- **Claude Code 丢尾换行**：逆向日志显示 `"foo\n"` 与 `"foo"` 在 Read 输出里等价（编辑器视图不需要尾换行）。
  模型据此取 `old_string` 时可能差一个字节。

**本项目的六个决定**

| 决策 | 取 | 依据 |
|---|---|---|
| offset 基准 | **1-based**，`parameters` 与 `description` 都明写 `1-indexed` | 4/5 家；且与 cargo 的 `src\x.rs:22:31` 同坐标系 |
| 行号前缀 | **不加** | 与 pi/Gemini 同组（`edit_file` 用字符串锚点，无行号引用需求）；实测省 15.6% 字节 ⇒ 8192 预算下多 20% 视野 |
| 尾注 | **必须有**：`第 a-b 行 / 共 N 行 / 用 offset=b+1 继续` | 五家全部有；缺一个模型就不知道要不要继续 |
| `truncate_*` | **返回结构体，且只装两个切点**：`Truncation{head_end, tail_start}`；其余（`is_truncated` / `split` / `omitted_bytes` / `*_lines` / `*_partial`）全做方法，文案上移 | pi 的 `TruncationResult` **只抄结构不抄字段** —— 那 10 个字段是「2000 行 / 50KB 双阈值」的产物；本项目单阈值，逐字段的死因见 ⑥ |
| 越界 | **bail 并报总行数**（`offset=900 超出，该文件共 589 行`） | pi；Gemini 的静默收敛是反例 |
| 单行超限 | **单独分支**：空内容 + 指明「第 N 行超限，请用 `rg -n` 定位」 | pi 的 `firstLineExceedsLimit` |

**三个实现陷阱（均已实测）**

1. **`offset - 1` 在 `offset = 0` 时 panic**（1-based 下 0 非法，但它一定会被传进来）：内部先归一化，0/None 都当 1。
2. 🔴 **禁止 `lines().skip(n).collect::<Vec<_>>().join("\n")` 切片**：
   ```
   原文件 "a\r\nb\r\nc\r\n"
     lines() 重组第 2 行起 = "b\nc"        ← \r 被剥掉、末尾 \n 也丢
     字节切片 &s[3..]     = "b\r\nc\r\n"   ← 逐字节保真
   ```
   后果很具体：模型拿 `b\nc` 当 `old_string` 去 `edit_file` 一个 CRLF 文件 ⇒ **0 匹配**，
   而我们刚在 0 匹配文案里写「可能是行尾符差异」—— 读侧自己吃了 `\r`，那句话就成了甩锅。
   正确做法：**找第 N 个 `\n` 的字节位置再 `&text[pos..]`**，零拷贝、保真、越界自然得 `""`。
3. **单行 20KB**：`truncate_middle` 的 `head/tail` 配额会把它切成两段残片。按 pi 的做法给独立分支。

⚠️ 「行号前缀不加」是本项目自己的判断，**不是行业多数**（2:3）。若将来发现模型定位变差，回来的入口是
`description`（补一句「行号前缀不是内容，`old_string` 不要带」），不是改回 `lines()` 重组。

**⑥ 收敛：`Truncation` 只装两个切点（2026-10-29，四轮追问后定稿）**

⑤ 定下的第一版是 `Truncation{head_end, tail_start, total_bytes, total_lines}` + `kept_lines()`。
用户一句「`total_bytes` 描述的是原文本的属性吧？那让它存感觉毫无意义」推翻了其中一半。判据最后落成一条：

> **core 只装「源上没有、只能从计算过程里得到」的东西。**
> 源是 `text`，`Truncation` 只是源上的两个坐标，不是第二份文件元数据。

**留下来的（2 字段 + 6 方法）**

| 成员 | 形态 | 判据 |
|---|---|---|
| `head_end` / `tail_start` | 字段 | **唯一不可派生项**；`(text, head_end, tail_start)` 就是全部信息 |
| `is_truncated()` | `tail_start > head_end` | 纯比较 |
| `split(text) -> (&str, &str)` | 字面切片 | **借用不复制**；绝不能 `lines().join()`（吃 `\r`、丢尾 `\n`） |
| `omitted_bytes()` | `tail_start - head_end` | O(1)、**精确无歧义**（字节没有「残行归属」问题）；`middle_lines` 的替代品 |
| `head_partial()` / `tail_partial()` | 切点是否紧跟 `\n` | 双重身份：既是行数修正项（`lines().count() - partial`），又是文案分支开关。⚠️ 本行原写「残行时**不能**提示 `offset=b+1`，会死循环」——**2026-10-30 被实测推翻**，见 ⑦ |
| `head_lines()` / `tail_lines()` | `lines().count() - partial as usize` | 只在「行号承重」处用（`read_file` 的 `b`） |

**四个被删掉的成员，各有一个不同的死因**

| 被删 | 死因 |
|---|---|
| `total_bytes` | `text.len()` **O(1)** 可派生；且**会诱导错判**——`total_bytes > tail_start - head_end` 看着像「有截断」，实际是「全文字节 > 省略字节」，几乎恒真 |
| `total_lines` | 调用方 `text.lines().count()` 现算一次即可。pi 存 `totalLines` 是因为它**按行做阈值**、数行本来就顺手；**本项目按字节 snap 到行、全程不数行**，照抄只剩装饰 |
| `kept_lines()` | 只有 head-only 才存在连续 `a..b`；`truncate_head_tail` 保留的是**两段不连续区间**，这个数没有可用语义（「保留 283 行」，哪 283 行？）。它其实是 `read_file` 的局部算术 `b = a + head_lines - 1`，不是 core 属性 |
| `middle_lines` | ① **代数恒等**：实测 `text[h..t].lines().count() ≡ total - head_lines - tail_lines`（由 `total = head.lines + mid.lines + tail.lines - hp - tp` 直接推得）⇒ 零新信息，还白扫一遍；② **残行场景是假话**：`"aaa\nbbb\nccc\n"` 切成 head=`"aaa\nb"` / mid=`"bb\nc"` / tail=`"cc\n"` 时报「省略 2 行」，而**完全未显示的行是 0**（bbb 露了开头、ccc 露了结尾）。`run_command` 改用 `omitted_bytes()` —— 命令输出没有 offset 续读语义，行号对模型毫无用处 |

**行数口径（唯一正确写法，已实测）**

```rust
let head_lines = text[..head_end].lines().count() - head_partial as usize;
```

两个「看起来更简单」的写法都错，而且错的方向相反：

| 写法 | 反例 | 实测 |
|---|---|---|
| 只用 `lines().count()` | `"a\nb"` 切在 3（行中） | lines=**2**，实际只有 1 个完整行 |
| 只用 `matches('\n').count()` | `"a\nb"` **未截断**、文件末尾无换行 | nl=**1**，实际 2 行 |
| `lines() - partial` | 上两者 | 1 / 2 ✅ |

原因：`lines()` 会把「末尾无 `\n` 的最后一段」原样当一行返回，而这段完不完整恰好由 `head_partial` 唯一确定。

**`\r\n` 的四条约束**（不影响计数，但每条都能踩坑）

1. 判据只能是 `\n`，不能是 `\r`：实测 `"a\r\n"` 与 `"a\n"` 都是 lines=1、nl=1、`partial=false`。
2. 🔴 `lines()` **只用于数数，绝不能用于生成返回内容**：`lines().collect().join("\n")` 剥 `\r`、丢尾 `\n`，
   模型拿它当 `old_string` ⇒ 0 匹配，而我们在 0 匹配文案里刚写了「可能是行尾符差异」。
3. 切在 `\r` 与 `\n` 之间：snap 成功时**不可能**（`head_end = pos + 1`，`pos` 是 `\n` 的下标）；只有 snap 失败才可能，
   而那时该行本就是残行。实测 `"a\r\nb\r\n"` 切在 2 ⇒ head=`"a\r"`、partial=true、`head_lines=0`。
4. ⚠️ **CR-only（老 Mac）不支持**：`lines()` 不认 `\r`，全文件算 1 行、nl 全 0、snap 永远失败、`partial` 永远 true。
   写注释声明即可（`my_api` 13 个文件全 LF）。

**当前落地形态（2026-10-29，用户已改）**

`truncate_middle` 是唯一的算法核；`truncate_head` / `truncate_head_tail` **暂时保留为薄壳** = 固定参数 + 拼文案：

```rust
fn truncate_head(text: String, max_bytes: usize) -> String       // get_truncate(&text, max_bytes, 0) + 文案
fn truncate_head_tail(text: String, max_bytes: usize) -> String  // (max - max/4, max/4) + 文案
```

薄壳的判据不是「能不能留」，而是「**留了不能长出策略**」：允许固定参数 + 拼文案，**禁止自己算数字 / 自己判边界**。
`now_len = head.len()` 可以（切片的固有属性）；第二轮再算一遍 `mid.lines().count()` 就不行。
第二刀（`run_command` 真的需要续读时）把文案搬回工具边界、删掉 `Executor::run` 的 match。

**⑥ 的遗留已全部结清（2026-10-29 记、2026-10-30 核）**

- ✅ **`tool.rs:237` 取错方法**已修：`omitted_bytes()` 现在真的被用了（`tail_lines` 反而变成 never used，见 ⑦(e)）。
- ✅ `mut text: String` 已去掉。
- ✅ `is_truncated` 不再 dead_code —— 它成了 `read_file` 尾注的第一个分支判据（`!truncated ⇒ 原样返回无尾注`）。
- ✅ **`Executor::run` 的 `ReadFile` 臂已拆成四分支**（原来的 `_ => truncate_head(...)` 已不再吞掉 `read_file`），
  形态与 `b`/`K` 的公式见 ⑦(d)。

**⑦ 尾注四分支：残行 ≠ 不可续读（2026-10-30，用户拿 `LINE_SNAP_MAX` 一问问倒）**

⑥ 里写过一句「残行时**不能**提示 `offset=b+1`，会死循环」。用户的反问是：**这与 `LINE_SNAP_MAX` 设定不符**。
他是对的，而且错两处 —— 一处措辞，一处结论。

**(a) `LINE_SNAP_MAX` 保证残片「大」，不保证「整」**

回退判据是 `head_end - pos < LINE_SNAP_MAX`（`pos` 是 `\n` 下标），而 `head_end - pos = 1 + 残片长度`。
实测（`"x\n" + 1000B + "\n" + "b"*(k+500)`，切在 `1003+k`）：

| 残片 k | head_end | partial | 残留字节 |
|---|---|---|---|
| 253 / 254 | 1003（回退） | false | 0 |
| **255** | 1258（不回退） | **true** | **255** |
| 2536 / 3192 | — | true | 2536 / 3192 |

⇒ 恒等式：**`head_partial == true` ⇒ 残片 ≥ `LINE_SNAP_MAX - 1` = 255 字节**。所以「残行」不等于「只给了一半」：
40 行 × 5000B 实测残片 **3192B = 该行 63.8%**。⑥ 那句把设定说反了 —— snap 存在的意义正是「小残片丢掉、大残片留着」。

**(b) `offset=b+1` 是「从残行行首重读」，预算全新 ⇒ 一定前进**

实测 `raw = text[5000..]`（195,000B）：`trunc(raw, 8192)` ⇒ `head_lines = 1`、残片 3192；
用 `offset=2` 重读时第 2 行 5000B < 8192 ⇒ **整行返回**，还顺带给第 3 行前 3192B。
代价是那 3192 字节重复出现一次；而「残行就当没给」抹掉的正是 `LINE_SNAP_MAX` 花预算换来的 63.8%。

**真正的死循环只有一种：`head_lines == 0`，即「从 `offset` 起的**第一行本身**就超过 8192」**

| 场景 | 从何处读 | `head_lines` | 结论 |
|---|---|---|---|
| 单行 20000B | 0 | **0** | `b = a + 0 - 1` **非法**；`offset` 原地打转 |
| 5000 + 9000 两行 | 0 | 1 | 可给 `offset=2` |
| 同上 | 第 2 行 | **0** | 到这里才撞死循环，「超长行」文案此刻才该出现 |

⇒ 分支判据是 **`head_lines == 0`**，不是 `head_partial`；且**只能运行时判** —— 模型每次回来的那一读会自己撞上，
不需要提前预测。

**(c) 两个概念的职责分工（「不符」的根因就是把它俩混了）**

| 概念 | 回答的问题 | 消费者 |
|---|---|---|
| `LINE_SNAP_MAX` / `head_partial` | **切在哪儿**、碎片要不要 | 预算分配；要不要披露残片 |
| `b = a + head_lines - 1` | **`offset` 能不能安全跳到 `b+1`** | 续读导航 |

`b` 的语义是「**最后一个能整行拿到的行号**」，不是「内容到此为止」—— 屏幕上越界那一截正是 snap 允许的，
所以用**另一句话**披露，不塞进 `a-b`。

**(d) 落地形态（2026-10-30 用户已改，`Executor::run` 的 `ReadFile` 臂）**

| 条件 | 尾注 |
|---|---|
| `!is_truncated()` | 原样返回，无尾注 |
| `head_lines == 0` | 「第 a 行超过单行上限 8192 字节，本工具无法整行返回；请用 `rg -n` 定位查看」 |
| `!head_partial()` | 「只返回 a-b 行，原文共 N 行；请用 `offset=b+1` 继续」 |
| 残行（≥255B） | 「只返回完整的 a-b 行，第 b+1 行仅前 K 字节，原文共 N 行；请用 `offset=b+1` 继续」 |

`b = skip + show_lines`（≡ `a + head_lines - 1`）；`K = head_end - show_content.rfind('\n').map_or(0, |p| p + 1)`，
一行算术、不需要新字段。**两处 off-by-one 已修**：旧代码 `skip + 1 + show_lines` 多报一行，恰好把 `b` 落在残行行号上
（40×5000B 报「1 - 2 行」而第 2 行只有 3192/5000 字节）—— 那就是用户看到的「把残行算进 b」。

**(e) 剩余（都不挡路）**

- `tail_lines` / `tail_partial` 现在 **never used**（`truncate_head_tail` 改用 `omitted_bytes()`），`cargo check` 报 `201:8`。
  按 ⑥ 的判据它们是**方法**不是字段，留着零成本，等「head+tail 两段各自行号」出现真实消费者再用。
- 超长行文案里的 `8192` 是**硬编码**，与 `Executor.max_bytes` 字段重复 ⇒ 按「字段存在即授权」应插值 `self.max_bytes`（纯文案，待改）。
- `raw.lines().count() + skip` 反推总行数是**第二个真值来源**（真值只有 `read_file` 手里有）。当前算术正确（N=40 实测 `39+1`），
  但若将来 `read_file` 改成自截 + 自拼尾注，这份反推必须一起删 —— 否则两个来源迟早漂。

对比 ⑥ 的结论，⑦ 没有推翻它：`Truncation` 仍然只装两个切点、仍然只提供边界与边界的算术。
被推翻的只是**「边界之后该怎么说话」**这一层判断 —— 而这层本来就不属于 core。

#### 3.6.6 日志层：屏幕与文件的分工（2026-10-30 落地）

**① 谁拥有屏幕**

`Executor::run` 是库函数，它返回 `String` 就够，**它不该写 stdout**。三条理由按严重程度排：
① TUI（ratatui 之类）要独占 stdout（alternate screen），任何零散 `println!` 都会撕裂渲染；
② `cargo test` 抓不到、也关不掉；③ `inv.info()` 是**给日志看的格式**，不是给模型看的格式 ——
让 core 决定显示什么，就是替前端做产品决策。

⇒ 数据仍由调用方那条链产生（`inv.info()`、`tool_res.len()` 本来就都在 `agent.rs` 手里），
只是**显示由 `agent.rs` 决定**。这与 §3.7.2 末 2026-10-21 那句「日志应在 `Executor::run`」不冲突：
那句说的是**信息所有权**（对），没说**往哪打**（错）。判据一路是同一句：**谁拥有屏幕，谁决定显示多少**。

**② 三个类型，三种职责**

| 类型 | 装什么 | 为什么是这个形状 |
|---|---|---|
| `LogKind`（enum，6 变体） | 事件**种类** | **闭合集合** ⇒ `match` 逼你穷尽。标签是日志的 grep 锚点，写成 `&str` 时 `"tool_reslt"` 编译通过、搜索时才失效 |
| `LogSink<'a>`（enum，2 变体 `File`/`Console`） | 落到**哪** | 也是闭合集合，但装的是**选择**不是状态。`Option<&str>` 也够用，代价是 `None` 会被读成「用默认路径」 |
| `Logger`（struct） | 唯一状态 `Option<File>` | 与 §3.6.5⑥ 同一条判据：结构体只装「源上没有、只能从调用过程得到」的东西。`kind` / `body` 都是参数，不进字段 |

**不是 `trait Sink`**：实现只有 1 个（文件）。TUI sink / 内存 sink 都还不存在 ⇒ 按「≥2 个真实实现才抽 trait」，
现在抽就是假壳。**触发条件**：出现第二个 sink，或 V1 流式把「整块写入」改成「增量 + 结束标记」——
那时 `record(kind, body)` 的签名本身就要拆，现在设计 `Event` 等于在没感觉到 delta 的形状之前先焊死
（和 `Executor::run` 返回 `String` 是同一个病）。

**③ `emit(file_text, console_text)`：签名就是不变式**

文件和屏幕**可以不同，但必须来自同一次调用**。`ToolResult` 是唯一的差异点：有文件时屏幕只报
`Tool Result : N 字节(全文见log文件)`，无文件时屏幕拿全文。这条不变式原来靠三处各记一遍维持，
现在靠一个签名 —— 「屏幕撒谎」那个 bug 正是从「三处各记一遍」的缝里长出来的。

**④ 「显式选择」与「自动降级」是两件事（本节的自更正）**

先后说过的两句「打不开就降级」（对）和「用 `Result` 让启动失败」（也对）是互相矛盾的。分界线是**谁决定**：

| | 谁决定 | 后果 |
|---|---|---|
| 自动降级 `open().ok()` | 库替你决定 | 你不知道有没有日志，屏幕还在说「全文见log文件」⇒ **错** |
| 显式选择 `LogSink::Console` | 调用方决定 | 你知道自己没文件，屏幕也不撒谎 ⇒ **对** |

所以 `new()` 用 `?` 上抛（**要了文件却没拿到就该报错，不该假装没事**），`new_no_file()` 只能由
`LogSink::Console` 显式到达。要点是：**没有「要了文件却悄悄没有」的路径**。

⚠️ 另一个更难发现的形态：`new_no_file()` 造出来但**没有选择点**（`agent.rs` 里硬编码 `Logger::new("data/run.log")`）——
接口有降级、运行路径上选不到，等于常量。**一个只有一种取值的选项不是选项**。

**⑤ 父目录：`create(true)` 不建父目录（实测）**

```
parent missing -> open ok? false      ← data/ 不存在时
parent exists  -> open ok? true
append result  -> "##### START TASK #####\nrun1\n##### START TASK #####\nrun2\n"
```

`data/` 在 `.gitignore` 里 ⇒ **新克隆必然没有**。旧写法 `open().ok()` 的实际后果是：第一次跑，屏幕打
`Tool Result : 8247字节(全文见log文件)`，而文件根本不存在 —— **日志在最需要它的时候（首次运行）失声，还谎报**。
现在 `new()` 里先 `create_dir_all(parent)?` 再 `open(path)?`。
（顺带：`writeln!` 用 `append` 不 `truncate` ⇒ 跨 run 累积，靠 `##### START TASK #####` 分隔可用，
但**同一任务串跑两次无法区分** ⇒ `task()` 该补时间戳或 `new()` 该 `truncate(true)`，二选一即可，别两头都默认。）

**⑥ 四条错误路径，现在各有一个 `LogKind::Error` 调用点**

改之前：`未找到符合的工具` / `parse_kind` 失败 / `payload()` 失败 三条都是 `format!("Error: {err:#}")`
直接进 `tool_messages`，**既不落日志也不上屏** —— 模型收到 `Error:` 开始瞎试，而你手上零记录。
而 `parse_kind` 失败通常意味着 `arguments` 形状不对（`#[serde(deny_unknown_fields)]` 一开就常见），
恰是最该复盘的一类；已被记的那条（幻觉工具名）反而是最不重要的。

另一条同类问题：`Executor` 的 `Err` 原来被记成 `ToolResult` ⇒ 屏幕打 `Tool Result : 43字节`，
**「工具炸了」和「工具正常返回」长得一样**。现在 `run()` 的 `match` 直接产出 `(LogKind, String)`，
一处决定「这是什么」，不再让 `{err:#}` 拍平的 error chain 伪装成结果。

**⑦ 不用 `BufWriter`（不是遗漏）**

`File` 直接写，`writeln!` 即落盘 ⇒ 进程被 kill / panic 时已写入的块一定在盘上。套了 `BufWriter`
反而要担心最后一块随进程消失 —— 而「它为什么崩」正是要查的那一次。**另外写文件在 `println!` 之前**：
`cargo run | head` 把 stdout 撑爆导致 `println!` panic 时，日志仍是完整的。

**⑧ 有 START 无 END 是信号，别用 `Drop` 抹掉它**

`task()` 写 `##### START TASK #####`，`##### END #####` 只在 `TaskResult` 分支写。所以
`choices.is_empty()` 的 `bail!`、`非法 assistant message` 的 `bail!`、`client.chat(...)?` 的网络错误
提前退出时，日志**只有 START 没有 END** —— 这本身就是「异常退出」的可读信号，**不要**给 `Logger` 加
`Drop` 兜底去「补全」它。（待补：那两个 `bail!` 前面各记一条 `LogKind::Error`，否则「为什么没 END」还得靠猜。）

**⑨ 待办（都不挡路）**

- 🔴 `main.rs` 现在 `LogSink::Console` ⇒ 回执全文上屏。跑 `offset` 专场要换成 `LogSink::File("data/run.log")`，
  否则三页 `read_file` 会把 `AI 思考` 刷出屏幕。
- `Logger::get_logger(&sink)` 里 `sink` 已是 `&LogSink` ⇒ `&&LogSink`（`match *sink` 靠 default binding
  modes 才编过），应直接传 `sink`。
- 命名：`get_logger` → `from_sink`（Rust 不用 `get_` 前缀做构造）；`new` / `new_no_file` 已私有，入口单一 ✅。
- `record` 把 `Thought` 全文打控制台 —— reasoning 通常比正文长得多，会比 `ToolResult` 更刷屏。
  属「折叠」范畴，V1 或嫌烦了再说。

### 3.7 工程化分档

判据只有一条：**这项工程化，每小时能让你学到多少 Rust，以及是否直接提升可靠性 / 可测性 / 安全性。**

| 档 | 内容 |
|---|---|
| **V0 就做** | 错误分层（领域错误 `thiserror` + 出口 `anyhow`，**结论见 §3.7.1**）· 结构化日志 · 超时与重试 · **把 LLM 调用藏在一个 trait 后面**（分发选择见 §3.2.1）· 路径沙箱 · 命令白名单 · **操作审批（host 在 write/run 前拦 y/n，注入式闭包，见 §3.6.5②）** · 工具输出截断 · token / 成本统计 |
| **V1–V2 再做** | 会话持久化 · 上下文压缩 · 流式输出 · 多后端抽象 · MCP（**等真实需求出现，不提前设计**）· **交互式 REPL + 语义追问**（模型主动提问，单发运行测不了，见 §3.6.5②(b)） |
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

#### ⚠️ anyhow 不是"万能错误类型"（2026-10-21，用户报 E0308 后实测）

**报错**：`expected Result<String, anyhow::Error>, found Result<String, std::io::Error>`
出现在 `pub fn read_file(path: &str) -> anyhow::Result<String> { fs::read_to_string(path) }`。

**根因：`From` 只在 `?` 处触发，函数返回值是精确类型匹配。** 这三处**都不会**转换：

| 位置 | 会不会 `From::from` |
|---|---|
| `expr?` | ✅ 会（`?` 的语义就是「失败时 `From::from` 后 return」）|
| 尾表达式 / `return expr` | ❌ 不会，必须与声明类型**逐字相同** |
| `match` 各分支、闭包体、`unwrap_or_else` 的实参 | ❌ 不会 |

anyhow 的"魔法"只有一条：`impl<E> From<E> for Error where E: StdError + Send + Sync + 'static`
（`anyhow-1.0.104/src/error.rs:674`）。它是**一个 `From` 实现**，不是超类型关系；
Rust 没有隐式转换（coercion 只覆盖 `&T`→`&U`、unsize、deref、fn 指针这几种，
`Result<_, io::Error> → Result<_, anyhow::Error>` 是"改类型参数"，不在其列）。

**三种合法写法，按场景选**：

| 写法 | 什么时候用 |
|---|---|
| `Ok(fs::read_to_string(path)?)` | 编译器 `help` 直接给的方案；最小改动 |
| `fs::read_to_string(path).map_err(anyhow::Error::from)` | 不想多包一层 `Ok`，但也没赚到什么 |
| **`fs::read_to_string(path).with_context(\|\| format!("读取 {path} 失败"))`** | **推荐给工具实现**：尾表达式直接是 `anyhow::Result`（类型精确匹配），且带上"哪个路径失败"—— 这条信息正是回灌给模型让它自纠的依据 |

#### `Path::display()` vs `to_string_lossy()`：输出一样，中间产物不同（2026-10-22 实测）

用户问两者差别。**输出逐字节相同**（含非法路径），差别只在"有没有中间 `String`"：

| | 合法路径 | 非法 UTF-16（孤立代理） | 输出 |
|---|---|---|---|
| `format!("读取 {} 失败", p.display())` | 2 次分配 | **1 次** | `读取 a�b 失败` |
| `format!("读取 {} 失败", p.to_string_lossy())` | 2 次 | **2 次** | `读取 a�b 失败`（字节完全相同） |

- **非法时差的这一次**：`to_string_lossy()` 必须先造一个 `Cow::Owned(String)`（存 `a U+FFFD b`），再被 `format!` 拷进目标缓冲；
  `display()` 返回一个轻量包装，**直接写进 `format!` 的缓冲，从不产生中间 `String`**。
- 两者在非法输入下**都做 `U+FFFD` 替换**（std 文档：`Path` 的 `Display` "may perform lossy conversion"）⇒ **没有正确性差异**。
- 所以判据是**类型需求，不是性能**：要 `Display`（塞进 `format!`/日志）用 `display()`；
  要 `&str`/`String`（`.len()`、`.contains()`、存起来）用 `to_string_lossy()`。
- 附带事实：**`Path` 本身不实现 `Display`**，`format!("{}", path)` 直接编译不过 —— `display()` 就是官方给的那座桥。

**`anyhow::Error` 不实现 `std::error::Error`（实测 E0277）**：
`fn need<T: std::error::Error>(_: &T)` 传 `&anyhow::Error` 直接编译失败。
原因是一致的（coherence）：若它 `impl StdError`，上面那条 blanket `From` 就会与 core 的自反 `impl<T> From<T> for T` 撞车。⇒ 任何 `T: std::error::Error` 的约束**都不接受** `anyhow::Error`。

**三条对循环的直接影响**：

1. 🔴 **`agent.rs` 里对 `ToolError` 用 `?` 是 bug，不是编译错误** —— 它会被 blanket `From` 悄悄收成
   `anyhow::Error` 然后**中止整轮 agent**，而正确行为是把错误**字符串回灌给模型继续**。
   编译器不会拦你，这条只能靠记。
2. **`tool.rs` 用 `anyhow::Result` 可以**（执行失败只需要一段人话），
   但**协议层错误（未知工具 / 参数坏）必须留在 `ToolError`**，否则"模型能据此自纠"的变体信息被抹平。
3. **回灌字符串用 `format!("{e:#}")` 而不是 `e.to_string()`**（实测见上：`Display` 只打最外层
   context，`{:#}` 才是 `外层: 根因` 一条链；`{:?}` 是多行 `Caused by:`）。

#### 「回灌」不是「打印」：三种处理方式的区别（2026-10-22）

用户问「`run` 报错绝不能 `?`，那是需要把错误打印出来吗」—— **不是打印，是回灌。** 三者的去向完全不同：

| 处理 | 谁看得到 | 循环 | 模型能否自纠 |
|---|---|---|---|
| `?`（错误向上冒泡） | 人（`main` 里打印） | **中止** | ❌ 它不知道发生了什么 |
| `println!` / `eprintln!` | 人 | 继续 | ❌ **模型看不到 stdout** |
| **回灌成 `role:"tool"` 的 content** | **模型** | 继续 | ✅ 下一轮换路径/换参数 |

代码形状就一行 `match`：

```rust
let content = match executor.run(&inv) {
    Ok(out) => out,
    Err(e) => format!("Error: {e:#}"),      // ← 不是 `?`，是变成字符串
};
messages.push(tool_message(call_id, content));
```

**判据：模型能不能通过改变自己下一次输出来修复这个错误？**

| 错误来源 | 例子 | 处理 |
|---|---|---|
| 工具执行（模型可修） | 文件不存在、路径越界、未知工具、参数缺字段 | **回灌 + 继续** |
| 传输/协议（模型不可修） | 网络断开、401、`choices` 为空、响应 JSON 解析失败 | `?` **中止** |

把 401 回灌给模型只会让它反复重试同一个请求；把「文件不存在」中止掉，等于让模型为一个拼写错误丢掉整轮。

🔴 **协议缺口：OpenAI 的 `role:"tool"` 消息没有 `is_error` 字段**（Anthropic 的 `tool_result` 有 `is_error: true`）。
所以**只能靠文本前缀区分**，`Error: ` / `[error] ` 这类标记不是风格问题 —— 不加的话，模型会把
「系统找不到指定的文件」当成**文件内容**继续分析（和 §3.4.13「`content` 每个字都在构成模型的判断」同一条）。

⚠️ 回灌的错误文本同样要**截断**：`executor.run()` 返回 `Err` 时**不经过** `truncates`，serde 报错可能带上整个 JSON。

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

#### 🔴 实测踩坑：把 `finish_reason` 当开关（2026-10-22，循环第一版跑通时）

用户第一版循环里写了这段（**已确认是 bug**）：

```rust
match &resp_content.finish_reason.as_deref() {
    Some("finished") => { finished = true; break; }   // ← "finished" 根本不存在
    Some("tool_calls") => println!("调用工具..."),
    Some(reason) => bail!("终止理由 {reason}"),        // ← "stop" 掉进这里
    None => { finished = true; break; }
}
```

实际运行（`run_agent(&client, "看下沙箱根目录'.'有什么文件", 5)`）：

```
开始第 1 步 → AI 调用 list_files(".")      finish_reason = "tool_calls"
开始第 2 步 → AI 给出最终答案（无 tool_calls）finish_reason = "stop"
Error: 终止理由 stop
error: process didn't exit successfully (exit code: 1)
```

**模型答得完全正确，是循环把它判成错误。** 两个独立的错叠在一起：

1. **`"finished"` 是猜的**。OpenAI 的取值是 `stop` / `length` / `tool_calls` / `content_filter`（§3.4.1 表里就有）；
   `finished` 属于 Anthropic 的 `stop_reason` 家族（那边是 `end_turn` / `max_tokens` / `tool_use`），混了。
   ⇒ **猜字符串常量永远猜不对，因为编译器不查、网关也不报**（它只是返回一个你不认识的值）。
2. **`Some(reason) => bail!(..)` 把「未知值」当致命错误**。`finish_reason` 的取值集合是**开放**的
   （§3.4.14 定过：`Option<String>` + 别穷尽，provider 会加新值），所以"兜底分支报错"这条路本身就是错的。

**正确修法不是把 `"finished"` 改成 `"stop"`，而是删掉整个 `match`：**

- **唯一判据是 `tool_calls` 是否为空**（§3.4.4 已定，本次实测再次验证）。`finish_reason` **不进控制流**。
- 理由不只是"provider 可能不一致"：**语义上该问的问题是「模型还给我活干吗」**，而不是「它自己说它停了吗」。
- `finish_reason` 留给**日志/诊断**：`Some("length")` 时提示"输出被长度限制截断"（这时 content 是半截话，
  当成正常结束会让人误以为回答完整）。

顺带这次暴露的另外三处：

| 位置 | 问题 |
|---|---|
| `if tool_messages.is_empty() { push(tool_call_id: "") }` | 造出一条 **`tool_call_id` 为空** 的 tool 消息 ⇒ 下一轮必 400。按「`tool_calls` 为空就 break」的新结构，这段是**死代码**，直接删 |
| `messages.push(resp.choices.pop().unwrap().message)` | `Vec::pop()` 取的是**最后一个**，不是 `choices[0]`。`n=1` 时无害，但语义撒谎；用 `swap_remove(0)` 或 `mem::take` 更准确 |
| `if !finished { println!("未能在{}步内完成") }` 后面紧跟 `println!("任务完成!")` | 超步数时两句都打。CLI 无所谓，但 V1 要区分「完成」和「耗尽」两种返回 |

#### 🔴 实测：工具清单里出现「声明了但没实现」的工具会怎样（2026-10-22）

**背景**：「改名」这个任务没有对应工具，用户想看看模型会怎么应对。实测发现**模型应对得很好** ——
但踩到的不是"没有工具"，而是"清单里有、调用却失败"。

当时的代码状态（`tool.rs:94`）：

```rust
fn raw_invoke(&self, root: &Path) -> anyhow::Result<String> {
    match self {
        Invocation::ReadFile(..) => ...,
        Invocation::ListFiles(..) => ...,
        _ => bail!("工具尚不支持"),        // ← WriteFile 掉进这里
    }
}
```

而 `ToolKind::ALL` 是**三个**（`ReadFile`/`ListFiles`/`WriteFile`），`spec()` 和 `parse_kind` 的
`WriteFile` 分支都写好了 ⇒ **`write_file` 通过 `tools: [...]` 真的发给了模型**。

任务：「把沙箱根目录下的 readme.md 改名为 README.md」，`max_steps = 5`。实际轨迹：

| 步 | 模型动作 | 值得注意的点 |
|---|---|---|
| 1 | `list_files(".")` | 先看有什么 |
| 2 | `read_file("readme.md")` | 思考里自己推出了正确结论：*"There's no rename tool. I need to read the file and write README.md. But then can I delete readme.md? No delete tool."* ⇒ **缺工具时模型自己能规划出替代路径** |
| 3 | `write_file(...)` | `reasoning` 与 `content` 都为空，直接调 |
| 4 | `write_file(...)` 再试一次 | 收到 `Error: 工具尚不支持`，然后连猜三种假设：*"Maybe transient" / "maybe write_file works only for certain paths" / "maybe case-insensitive filesystem: README.md already exists as readme.md?"* |
| 5 | `list_files(".")` 再确认一遍 | 耗尽，`未能在5步内完成!` |

**四条结论：**

1. **「没有工具」不是问题，「有工具但坏了」才是。** 第 2 步证明了模型能优雅处理能力缺失（自己推理出没有
   delete ⇒ 只能 read + write，且明确知道"这不是真正的改名"）。它真正被误导的是**清单里声明了 `write_file`
   却调不通** —— 于是把一次硬失败解释成"临时故障 / 路径限制 / 文件系统大小写"，白烧 2 步。
   ⇒ **工具清单必须等于「现在真能用的能力」，不是「计划中的能力」。** 这是 §3.4.8「`tools` 单一来源」
   的一个新侧面：单一来源保证"清单与派发一致"，**不保证"清单里的都实现了"** —— 而 `raw_invoke` 的
   `_` 臂恰好把这条缝堵死了（加第三个变体编译器**不报 E0004**）。
2. **错误文案要带工具名和可用清单。** `工具尚不支持` 让模型只能猜；写成
   `write_file 尚未实现（当前可用：read_file、list_files）` 它一步就会放弃。
   （模型这次靠"工具尚不支持"四个字猜对了方向，属于运气好。）
3. **步数耗尽时，模型想说的话全丢了。** 第 5 步的 `reasoning` 已经写着 *"the conclusion: cannot perform
   rename; report to user honestly"* —— 但它再没有机会输出这句话，`finished = false` 直接退出。
   V0 打印状态可接受；**V1 的正确收尾是在 `max_steps` 用尽后补一次 `tool_choice: "none"` 的请求**
   （不带工具，强制它把结论说完），这是各家 agent 的通行做法。
4. **日志放在了错误的一层。** `println!("read file : {}")` 写在 `read_file`/`list_files` **内部** ⇒
   `WriteFile` 走 `_` 臂时**一行日志都没有**（第 3、4 步日志完全空白，看不出它调了什么、参数是什么）。
   日志应在 `Executor::run` —— 那里同时握有「哪个工具 / 入参 / 输出多少字节」，一次覆盖所有工具，
   且测试调用内层 IO 时不会被日志噪音污染。
   ⚠️ **2026-10-30 二次更正**：本条只走了一半。**信息所有权**在 `Executor::run` 是对的，但**它在库函数里写
   stdout 是错的**（TUI 独占屏幕、`cargo test` 抓不到也关不掉、`inv.info()` 是日志格式不是模型格式）。
   最终形态：`Executor::run` 保持静默，`agent.rs` 用 `Logger` 决定显示什么 —— 调用方本来就逐字拥有那四要素，
   上移不损失任何信息。见 **§3.6.6**。

**修法（顺序）：**

- 立刻：`ALL` 先缩到两个（`ReadFile`/`ListFiles`），让模型看到的能力 = 实际能做的。
  代价是 `ToolKind::WriteFile` 变体暂时 `never constructed`（一条 `dead_code` 告警），
  `spec()` / `parse_kind` 的 `WriteFile` 分支保留着、等实现好了接上。
- 写 `write_file` 时：`_ => bail!(..)` **必须换成显式 `Invocation::WriteFile(..)` 臂**。
  换掉之后，将来再加变体编译器就会报 E0004 —— 那个 `_` 现在是"漏臂静默"的唯一源头。
- 顺带：`raw_invoke` 里 `WriteFile` 的目标可能**不存在**，`resolve_in(root, &args.path)` 会因
  `canonicalize` ENOENT 直接失败 ⇒ 必须对**父目录** `canonicalize` + `starts_with`，再 `join(file_name)`，
  且 `file_name` 要单独校验（不含分隔符、不是 `.`/`..`）。见 §3.5「分工具的策略不一样」。

#### 循环要不要写成"状态机"（2026-10-21）

**问**：「这个 loop 是不是应该按状态机的写法来？」

**答：它本来就是状态机 —— 但状态是 `Vec<Message>`，不是一个 `enum State`。**
把状态再抽成一个 enum 是**第三次"两个真相"**（前两次是 `tools` 双写、owned `ChatRequest`）。

**判据一句话：一个状态只有一条出边，它就不该是状态，它该是一行代码。**
V0 全程只有 **1 个决策点**：拿到 `message` 后分成「Final」/「Tools」两条出边。
「执行工具」「push 消息」各自只有**唯一**的下一跳 ⇒ 它们是步骤，不是状态。
一个只有单决策点的状态机，展开后就等于一个 `if`。

**为什么 LangGraph / OpenAI Agents SDK 真有 FSM**：它们要 **checkpoint / interrupt / resume** ——
状态得能**序列化落盘**、能从"等人批准"那一刻**重启**。本项目 V2 才落库，且那时要存的是
**消息行 + 游标**（最后一条已处理的下标），不是"阶段名"：`AwaitingApproval` 是可以从
`messages` 派生的（末尾是带 `tool_calls` 的 assistant，且缺对应的 `tool` 回复）⇒ 存同义词 = 双写。

**四种写法，什么时候用哪种**：

| 写法 | 状态住哪 | 适用 |
|---|---|---|
| `for step in 0..max_steps` + `if/else` | `messages` | **V0 选它**：1 个决策点、无中断、不落盘 |
| 转移函数 + 借用 `enum Turn<'a> { Final(&'a str), Tools(&'a [ToolCall]) }` | `messages` | 想给分支**命名**、想给分类逻辑**单测**时 |
| `enum State` + `match` 自持转移 | 一个 enum 字段 | 有**"等外部事件"**或**多条出边**（重试/批准/取消） |
| typestate `Agent<Thinking>` | 类型参数 | **不可能**用于此处：循环次数来自 argv，编译期未知；typestate 只能表达编译期已知的固定序列（builder） |

**实测（两方向都验过）**：借用型 `Turn` 在 `Tools` 分支里"先 `collect` 结果、再 `push(msg)`"**能编译**
（NLL：`calls` 最后一次使用后借用即结束）；一旦 `push` 之后再碰 `calls` →
`error[E0505]: cannot move out of 'msg' because it is borrowed`，且编译器会建议"给 `Msg` 实现 `Clone` 就能过"——
**那条建议正是要避免的浪费**。⇒ 借用 enum 可用，但 V0 里 `if msg.tool_calls.is_empty()` 更短、且不贴着 E0505 的边。

两个容易写错的地方：

- **别拿 `finish_reason` 当转移条件**。它不参与控制流（§3.4.14），判据永远是 `tool_calls` 是否为空 ——
  各家 provider 在"有 tool_calls"时也可能报 `stop`。
- **`max_steps` 不是状态**，它是 fold 的次数上限，对应 `Iterator::take`。

**什么时候再回来改**：V1 出现"等外部事件"（流式半成品 / 人工确认 / Ctrl-C 取消），
或者给网络错误加"退避重试 vs 放弃"（**两条出边**）—— 那时才配一个 `enum State`。

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
- [x] `write_file` 补 `content` 参数（`WriteFileArg{path, content}` 已加）；`run_command` 的 `cmd` 等白名单设计定案再定（**§3.4.10** 末表）
- [ ] `ToolKind::ALL` 防漂移测试：长度 + 名字唯一 + `from_name` 往返（§3.4.8 实测 ③ / §3.4.12 漏洞 ②；V1 换 `trait Tool` 后此测试可删）
- [x] **`from_name` 已改成 `Self::ALL.iter().find(|t| t.name() == n).cloned()`**（2026-10-27 核实：手写 `match` 已删）—— 工具名只剩 `name()` 一处，`from_name` 认识但 `ALL` 不宣告的矛盾态消失（**§3.4.12 漏洞 ①**）
- [x] 每个 `Invocation` 变体装箱 `XxxArgs`（`#[serde(deny_unknown_fields)]`），`parse` 用 `from_args::<T>`（**§3.4.11 / §3.4.12**）
- [x] **`content` 收紧**：`System` / `User` / `Tool` 改 `String`，`Assistant` 留 `Option<String>`（**§3.4.13** 实测：`user` 的 `null` 会让模型幻觉出问题且不报错；`tool` 的 `null` 直接 400）
- [x] `parse_kind` 改一步 `from_str::<T>(args)`，删 `ToolError::BadParse`（**§3.4.13** 末：两步解析丢 `line/column`）
- [x] `ChatResponse` / `Choice`（**`pub`**）+ **私有** `WireRequest<'a>`（**§3.4.14 / §3.7.6**：`Choice.message` 直接用 `Message`，`finish_reason` 不参与判断；请求类型**不 `pub`**、`messages: &'a [Message]` 借用不 clone）
- [ ] `ChatResponse` 补 `usage: Option<Usage>`（只为看 `prompt_tokens_details.cached_tokens`；2026-10-20 实测 `cached_tokens: 384 / prompt_tokens: 420`，V1 上下文管理要用）
- [x] `llm.rs`：`OpencodeGoClient { session_id, chat_url, default_model, api_key, client }` + `new(&Config)` + `async fn chat(&self, msgs: &[Message], tools: &[WireTool]) -> anyhow::Result<ChatResponse>`，内部构造私有 `WireRequest<'a>`（**先具体实现，不写 trait**）—— **2026-10-20 真实 200 跑通**
- [x] **删 `ToolKind::tool_json()`**（零调用点），`chat` 的 `tools` 收 `&[WireTool]`；`let tools = ToolKind::tools()` 放**循环外**构造一次（**§3.7.6** 实测：`&Value` 路径每轮多 105 次分配，且 `to_value` 会把键按字母序重排 ⇒ 与 struct 声明序字节不同）
- [x] `OpencodeGoClient` **手写 `Debug`**：`api_key` 打 `"***"`、`client` 打 `"<reqwest::Client>"`（**§3.7.3 / §3.7.5** 实测：`reqwest::Client` 的 `Debug` 会明文打出 `authorization: Bearer sk-...`）
- [x] api_key **走 `.bearer_auth()` 每请求设置**，**不要**塞进 `default_headers`（**§3.7.3** 实测：`bearer_auth` 自动 `set_sensitive`，其余三种写法有两种明文泄漏）
- [x] **`chat()` 里删掉 `serde_json::to_value(chat_request)?`**（2026-10-23 已改：直接 `.json(&chat_request)`）
- [x] **`chat()` 先判状态码再 `text()`**（2026-10-23 已改：`status` 先存下来，body 拿完再 `if !status.is_success()` 把 `status` + `body` 一起塞进 `anyhow`）
- [x] ✅ **`main.rs` 的协议探针已删**（2026-10-27）：`test_chat` / `get_chat` 等 150 行清掉，现在 `main.rs` = `Config::from_env` → `OpencodeGoClient` → `run_agent(…, 5)` 共 12 行
- [ ] 删 `model.rs` 的 `pub struct ChatRequest`（零调用点，与 `llm.rs::WireRequest` 构成「两个真相」）——**仅剩这一处**
- [x] 循环接上：`tool_calls` 非空 → 执行 → 回灌 → 继续（**§3.7.2** 六分支表；`client` 从参数进来、返回值不出现 `reqwest` 类型）—— **2026-10-22 真实任务跑通**
- [x] 终止条件 + 步骤上限（`for step in 1..=max_steps`，`tool_calls` 为空即 break；`finish_reason` 只用于 `length` 日志）
- [x] **`ToolKind::ALL` 缩到「真能用的工具」**（2026-10-23 由「立刻实现」路径解决：`write_file` 已是真实现，`_` 臂已删）：`WriteFile` 已在 `ALL` 里、`spec()`/`parse_kind` 也全写好了，但 `raw_invoke` 走 `_ => bail!("工具尚不支持")` ⇒ **`write_file` 真的发给了模型**，实测模型白烧 2 步并猜出"临时故障 / 路径限制 / 大小写文件系统"三种错误假设（**§3.7.2 末 2026-10-22 实测**）。**2026-10-23 定：走「立刻实现」这条路，不缩 `ALL`** —— 顺序 = `write_file` → `run_command`
- [x] **`raw_invoke` 的 `_` 臂换成显式 `Invocation::WriteFile(..)`**（2026-10-23 完成：`match` 三个臂穷尽、无 `_`，加变体会报 E0004；失败文案已带 `args.path`）：那个 `_` 是"漏臂静默"的唯一来源（换掉后加变体才会报 E0004）；顺带错误文案要带工具名 + 可用清单
- [x] ✅ **日志层（2026-10-30）**：`read_file`/`list_files` 内部的 `println!` → `Executor::run` → **最终落在 `agent.rs` 的 `Logger`**。判据演进：「信息所有权在 `Executor::run`」**对**（四要素齐全），但「库函数写 stdout」**错**（TUI 独占屏幕 / 测试关不掉 / `inv.info()` 是日志格式不是模型格式）—— 两者不冲突，因为调用方本来就逐字拥有那四要素。`LogKind` / `LogSink` / `Logger` 的分工、四条错误路径、屏幕与文件同源，见 **§3.6.6**
- [ ] **`max_steps` 用尽后补一次 `tool_choice:"none"` 收尾**（V1）：实测耗尽时模型 `reasoning` 里已经写好结论（"report to user honestly"）却再没机会输出（**§3.7.2 末**）
- [ ] `LlmClient` trait 立起来 —— **排在循环跑通之后**，签名从实际代码抄（§3.7.2）
- [x] 4 个工具实现：`list_files` ✔ / `read_file` ✔（2026-10-22 完成：`sort()` 确定序、`join` 替 `fold`、`file_name()` 相对路径、`[DIR]`/`[FILE]` 前缀、非 UTF-8 显式标注） / `write_file` ✔（2026-10-23 v4：整文件覆盖 + 自动建父目录 + 祖先 canon） / `run_command` ✔（2026-10-27：裸名 + 白名单（都在 `raw_invoke`，裸名在前）+ **继承环境 + `env_remove` 自己的 key** + `current_dir` + 三流显式 + **退出码全走 `Ok`**；端到端实测通过，见 §3.5 末 2026-10-27）
- [x] **`write_file` 的沙箱检查要单独写**（2026-10-23 v4 完成：`normalize_path` 消 `..` → 往上 `try_exists()` 找最深已存在祖先 → **canon 祖先** + `starts_with` → `symlink_metadata` 判非普通文件 → 仅 `NotFound` 才建父目录重试）：语义是**整文件覆盖**（`fs::write`，实测 20→2 字节截断无残留），返回里带「已覆盖 N 字节」。**必须自动建父目录**（V0 无 shell，模型没有 `mkdir -p` 兜底，2026-10-23 查证五家后修正）⇒ 写失败 `NotFound` 时：从 `target.parent()` 往上用 `try_exists()` 找**最深的已存在祖先** → canon + `starts_with` → `create_dir_all(parent)` → 重试写。`file_name()` 为 `None` 的输入（`""`/`..`/`a/..`）要拒；`target` 已存在时用 `symlink_metadata().is_file()` 判（`fs::write` 指向目录只报 `拒绝访问 os error 5`，看不出是目录）（**§3.5** 末 2026-10-23）
- [x] **`run_command` 收 `{program, args}`，不收命令字符串**（2026-10-26 v3 落地）：V0 不给 shell（实测管道会被 rg 当正则、`cmd` 单引号不是引号、`sh` 来自 WorkBuddy 自带 PortableGit）；`ALLOW_CMD = [cargo, rg, yq, jq]` 且**排除 `cmd`/`sh`/`powershell`**；显式 `.current_dir(root)`；三流全显式（`stdout`/`stderr` pipe + `stdin` null）后截断（**§3.5** 末 2026-10-23 六组实测 + 2026-10-26 v3）
- [x] **env 策略改判（2026-10-27，推翻 2026-10-26 的 `env_clear` + 最小集）**：原方案 `env_clear` + 6 变量（`PATH`/`HOME`/`USERPROFILE`/`SystemRoot`/`TEMP`/`TMP`）**让 `cargo test`/`cargo run` 链接失败**（rustc 找不到 MSVC 工具链 ⇒ 撞 PATH 上的 GNU coreutils `link`，报 `link: missing operand after '\377\376'`）。补 `SystemDrive`/`ProgramData` 能救，但那是 rustc 黑盒探测的副产物、跨版本跨机器会变 ⇒ **整段删除**，改 `for key in config::LLM_ENV_KEYS { cmd.env_remove(key); }`。理由：白名单是**从零构造正集**（上界不可枚举），负集才闭合；业界主流（Codex `shell_environment_policy` / Gemini CLI `sanitizeEnvironment`）都是「继承 + 按名字减」。实测：继承 254 个变量、`LLM_*` 三个全 absent、链接恢复正常（**§3.5** 末 2026-10-27 三派调研）
- [x] **`run_command` 的退出码不再分档（2026-10-26 复测推翻 0/1 规则；v3 已落地）**：`yq . nosuch.yaml` 真错误也是 **1** 且只写 stderr ⇒ 旧写法回执是零信息量的 `命令退出码 1:`；`cargo test` 失败（101）诊断全在 **stdout**（stderr 仅 314 字节）⇒ 旧写法把"哪个测试失败/断言是什么"整个丢掉。v3 改成：**只有 `spawn()` / `wait_with_output()` 失败才 `Err`，其余退出码全走 `Ok`**，回执带 `命令退出码: N` + `返回结果:` + `错误信息:`（§3.5 末 2026-10-26）
- [x] ✅ **`tool.rs` 编译已通过**（2026-10-27）：`RunCommand` 臂那个落在块内的逗号已删（`…)),}` → `…))`），`cargo check --all-targets` 零 error 零代码告警（§3.5 末 2026-10-26 v3）
- [x] ✅ **`run_command` 的两个安全检查已合并到 `raw_invoke`**（2026-10-27）：**裸名在前、白名单在后**，`run_command` 只剩 IO + 契约注释。这样 V1 换 `Box<dyn Tool>` 时安全边界不会随 `match` 一起消失（§3.5 末 2026-10-26 v3 实测）
- [x] ✅ **`run_command` 已加 `.stdin(Stdio::null())`**（2026-10-27）：三流现在全显式声明（stdout/stderr pipe + stdin null），不再继承父进程终端（§3.5 末 2026-10-26 v3）
- [x] ✅ **`RunCommandArg` 已补 `#[serde(deny_unknown_fields)]`**（2026-10-27）：与另外三个 Arg 一致，模型传 `timeout` 现在会明确报参数有误，不再静默忽略
- [x] ✅ **`run_command` 两条 bail 文案已修**（2026-10-27）：裸名那句不再谎称「未找到可执行文件」（改成「只能写裸名，不能带目录或路径」），白名单那句带 `ALLOW_CMD.join(", ")` 清单；外侧 context 改「无法执行命令 {}」（§3.5 末 2026-10-26 二次更正）
- [x] 🟠 **让模型用 `cargo check --message-format=short`**：`MAX_TOOL_OUTPUT` = 8192，80 个错的 crate 长格式 `cargo check` 是 **17,213 字节 / 643 行**、`--short` 是 6,533。实测 200 个 E0308 时 head=6144 只能装 **27 个（长格式）/ 99 个（short）** ⇒ 不写这句，模型会「修完前 27 个 → 再跑 → 再修」。**2026-10-27 已实现，但落在 prompt 而不是 `spec()`**：`main.rs` 的任务横幅里加了「可以使用“cargo check --message-format=short”命令减少输出」——只对当前任务生效；`spec()` 仍是 4 条（要变成**所有任务永久生效**，把第 5 条加进 `ToolKind::RunCommand` 的 description 即可，属纯文案）
- [x] ✅ **截断拆成 `truncate_head` / `truncate_head_tail`（2026-10-27，`max_bytes` 不用调大）**：原来一份 `truncates` 只砍尾巴，而 **`cargo test` 的诊断恰好在尾部**（1200 行日志把 `panicked` / 断言值 / `test result: FAILED` 全推到 8192 之后 ⇒ 模型只看到 50 行 `test ok`）。现在按 `Invocation` 派发：`RunCommand` → 头 3/4 + 尾 1/4（实测尾部诊断只需 348 字节，2048 有 6 倍余量），其余工具 → 只留头。共用私有 `truncate_middle() -> (head_end, tail_start)`，`LINE_SNAP_MAX = 256`。**没抽「core + 两个壳」**：两份文案不同（`read_file` 要报「剩多少 + 用 rg 定位」，`run_command` 只报「中间省略多少」），core 里带 `if 策略` 就是假壳。实测 6 个用例全过（见 §3.5 末 2026-10-27）
- [x] ✅ **回执超预算：不是问题，是语义（2026-10-30 用户定性）**：`max_bytes` 管的是「**正文预算**」，尾注是切完之后追加的
  元信息（实测 84~120B，有界）。不该为它引入「先算文案长度再切正文」的两遍耦合；真要硬上限就**调小常量**。
  ⚠️ 但 `write_file` 的拒写守卫共用同一个 `max_bytes`，调它会连带压低整文件重写阈值 ⇒ 要动先拆参数。
  代码侧补一句注释即可（旧结论「~50 字节不值得动逻辑」作废——那时的数字是两条薄壳的，四分支后翻倍，见 §3.5 末 2026-10-30）
- [ ] 🟠 **`Executor::run` 对 `Err` 也要截断**：现在 `?` 直接上抛，`agent.rs` 只做 `format!("Error: {e:#}")` ⇒ 一个 80 错的 crate 可达 **17,213 字节 / 643 行**，全部原样进 history（`MAX_TOOL_OUTPUT` = 8192）。⚠️ 退出码全走 `Ok` 之后这条已大幅降级（`Err` 只剩 `spawn`/拒绝执行，文案都很短），属**防御性兜底**，V0 可不做（§3.5 末 2026-10-26 实测）
- [x] **`truncates` 修 panic**：`while !text.is_char_boundary(end) { end -= 1 }` 已补，并加了「原文 N 行 / M 字节」报告（2026-10-22）。**2026-10-27 已重命名并拆分**：`truncates` → `truncate_middle`（私有，返回切点）+ `truncate_head` / `truncate_head_tail`（两个具名策略）
- [x] **`Executor` 横切层 + `raw_invoke` 分派层**：`Executor{root, max_bytes}`（`new` 里 `canonicalize` 一次 = 不变量载体）、`run` = `raw_invoke` → **按 `Invocation` 派发截断策略**（`RunCommand` → 头+尾，其余 → 只留头，2026-10-27）；`raw_invoke` 私有、每臂先 `resolve_in`（**§3.5**：校验函数必须**返回**后续真正被使用的那个值，不是 `bool`/`()`）
- [x] **错误文案里的路径统一成 `args.path`**（2026-10-23 完成）：① ✔ `resolve_in` 的 `root.display()` 已删 ② ✔ `read_file`/`list_files` 的 `with_context` 已退化成裸 `?`，路径由 `raw_invoke` 用 `args.path` 补 ③ ✔ `WriteFile` 臂已加 `.with_context(|| format!("写入 {} 失败", args.path))`，`路径并不是合法文件!` 改成「目标已存在，但并不是普通文件!」（§3.5 末 2026-10-23 实测）
- [x] **`resolve_for_write` 返回「canon 祖先 + 词法尾巴」（方案 B）**（2026-10-26 v6 完成）：`canon_祖先.join(tail)` + 空尾巴单判；**关键坑**是 `tail` 必须 `canon.strip_prefix(canon_tmp)?`（**词法**祖先），用 `canon_ancestor`（canon 后）会因 `VerbatimDisk` vs `Disk` 恒失败、再被 `unwrap_or(&canon)` 静默吞掉 ⇒ `join` 遇绝对路径替换 ⇒ 退回方案 A。v6 实测 **11/11**：沙箱内普通绝对路径（新建/覆盖）、大小写不同祖先（回执 `Sub\...` 证明走了真拼接）、junction 单层/多级、`../` 越界、深层建目录全过（§3.5 末 2026-10-26）
- [ ] **`truncates` / `resolve_in` 两条单测**（`write_file` 侧 2026-10-28 已落地 **`FileHelper` + 4 个独立用例**：
  新建 / 多级父目录（走 `create_dir_all` 重试）/ `../` 越界 / 目标是目录，**断言值与原文、不只 `is_ok`**；写法约定见 **§3.6.5④**）：
  2026-10-27 已用 `Executor::run` **手动端到端**跑过 6 个用例（不超限原样返回 / 5000 行 rg 命中保尾 / 中文多字节 / 20KB 无换行单行 / 真实 `cargo test` 尾部诊断 / `cargo check` 200 错），但这两块**仍是 0 覆盖** —— 手动测不防回归。要补：`truncate_middle("中".repeat(3000), 8192, 0)` 不 panic、`resolve_in(tmp, "../x")` 必须 `Err`、**junction 越界**（`mklink /J` 普通用户可建）、**覆盖已有普通文件**（现在只测了「目标是目录」）
- [ ] 安全边界落地：scratch 目录 + **操作审批** + 命令白名单 + 输出截断（路径沙箱 2026-10-22 ✔ / 白名单 ✔ / 截断 ✔；
  「`run_command` 上线时路径检查即失效，必须换 OS 沙箱/白名单/容器」见 §3.5 调研小节）。
  **审批（2026-10-27 重新定型，见 §3.6.5②）**：`Executor.confirm: Box<dyn Fn(&Invocation) -> bool>`，
  `write_file` / `run_command` 两臂调用前先过它；CLI 传「打印 + 读一行 y/n」，**测试传 `|_| false`** ⇒ 这一层无交互可测。
  「模型自己停下来问」（语义追问）不在本条，属 V1 交互式 REPL
- [ ] 自答 §3.8 的 4 个问题
- [x] ✅ **长文件根治 = 三件，且承重的是写侧（2026-10-30 三件全落地）**（2026-10-27 用户指出「加 offset 也没用，write 又没有插入工具」后改判；详见 §3.6.3 / §3.6.5）：
  现状 `read_file` 头部截断 8192 + `write_file` 整文件覆盖 ⇒ 对 >8192 字节的文件「读一半再写回」= **静默丢内容**
  （实测 `my_api/src/auth.rs` 18,488 字节，中间 10,368 字节不可见，该段里 `SqlitePool` 出现 7 次）。三件按优先级：
  ① ✅ 🔴 **`edit_file(old_string, new_string)`（2026-10-28 已落地）**（5 号工具）—— 让「写」不再要求「手里有全文」，**这才是闭合条件**。
  余 3 条文案 + 1 条语义见 §3.6.3「edit_file 落地后的 review」。
  单测：**`FileHelper`（幂等 guard）2026-10-28 已落地**（`write_file` 4 用例全绿、从无 `tmp/` 的干净状态起跑也绿）；
  **`edit_file` 的 8+2 用例待铺**（清单与断言要求见 §3.6.5④）。
  三态匹配：`0` / `1` / `n>1` 各自 bail 且文案可行动；`new_string` 允许以 `old_string` 为前缀（插入语义）；
  0 匹配的文案要提「可能是行尾符差异」（实测 `my_api/src` 13 个文件全 LF，当前不踩）。
  「插入」不需要独立工具、也**不要**给 `write_file` 加 `mode:` 参数（§3.4 单一策略决定点）；
  ② ✅ 🔴 **`write_file` 大文件拒写守卫（2026-10-27 已落地）**：`meta.len() > max_bytes` ⇒ 拒 + 指路 `edit_file`。**阈值走参数**
  （`write_file(path, content, max_bytes)` + `raw_invoke(&self, root, max_bytes)`）——
  **2026-10-27 用户质疑后改判**：旧说法「直接引常量、不传参，否则改一个忘另一个」是纪律型论证，作废；
  判据是「`max_bytes` 是字段还是 `const`」—— 字段即「它会变」的授权，故读字段（见 §3.6.3）。代价：合法的大文件整体重写被拒 —— V1 用
  Claude Code 的「Edit/Write 前必须 Read 过」覆盖率追踪取代；现在这条是「静默毁 10KB → 明确拒绝」的纯增益；
  ③ ✅ **`read_file` 加 `offset`（行号，不是字节）—— 2026-10-29 定设计、2026-10-30 落地**——唯一导航信号是 cargo 的 `src\x.rs:22:31`，模型没法把行号换算成字节
  （**2026-10-27 自我更正**：早先写「按字节更自洽」是错的；Gemini CLI 用 `start_line`/`end_line`、Claude Code 用行 `offset`/`limit`
  也是行）。字节上限只作最后安全网（防单行 20KB）。
  **2026-10-29 五家调研后设计定稿，见 §3.6.5⑤**：1-based 明写 `1-indexed` / **不加行号前缀**（省 15.6% 字节）/
  尾注必须有「第 a-b 行 · 共 N 行 · 用 offset=b+1 继续」/ 越界 **bail 并报总行数**（不静默收敛）/
  ✅ **`truncate_*` 已改成返回结构体（2026-10-29 落地，形态见 §3.6.5⑥）**：`Truncation{head_end, tail_start}` **只装两个切点**
  （`total_bytes` / `total_lines` / `kept_lines` / `middle_lines` 逐个论证后删除），行数由 `head_lines` / `tail_lines` 方法算；
  `truncate_head` / `truncate_head_tail` 暂留为「固定参数 + 拼文案」薄壳。
  ✅ **尾注四分支已接入（2026-10-30）**：`Executor::run` 的 `ReadFile` 臂按 `!truncated` / `head_lines == 0`（超长单行）/
  `!head_partial` / 残行 分四种文案，`b = skip + show_lines`、`K` = 残片字节数，两处 off-by-one 已修。
  ⚠️ ⑥ 原写的「残行**不能**给 `offset=b+1`，会死循环」**已被实测推翻**（`LINE_SNAP_MAX` 保证残片 ≥255B；
  从残行行首重读预算全新 ⇒ 一定前进），死循环的真正判据是 `head_lines == 0`，详见 §3.6.5⑦。
  自查：`offset=0` 的下溢已由 `saturating_sub(1)` 堵住（schema 里也写了 `"minimum": 1`，但那条替不掉 `saturating_sub`）。
  加第 5 个工具的实际成本：编译器会逼你补 **4 处**穷尽 match（`name()` / `spec()` / `parse_kind` / `raw_invoke`），
  唯一没被逼的是 `ALL` 数组（已有「长度 + 名字唯一」测试钉着）⇒ 又一个 V1 `Box<dyn Tool>` 的论据。
  ⚠️ **别用「给 `uv` 权限让它写 python 脚本读写」代替**，三条理由（2026-10-27 实测/核证）：
  ① `uv run python -c "print(open(f).read())"` 的 stdout **走的是同一个 `truncate_head_tail(8192)`** ——
  实测 18,489 字节照样被砍、中间 10,368 字节仍不可见，只是把「读文件」绕开了、没绕开「进 history 前的截断」；
  ② 业界反证：Gemini CLI（`run_shell_command`）与 Claude Code（`Bash`）**都同时有** shell 和 `read_file` 分页
  ⇒「有 shell 就不用分页」不成立；③ 往 `ALLOW_CMD` 加 `uv` = 加一个万能启动器，回执形状失控、模型会偏好用它绕开小工具。
  ——安全**不是**理由：`cargo` 的 `build.rs` 本来就能执行任意代码，路径沙箱≠权限沙箱
- [x] **验收靶场就绪**（2026-10-27）：`scratch/my_api` 副本 + `tools/scratch_bugs.py`（inject/clean/status）+
  3 个注入错误（`handler.rs` E0308 / `models.rs` E0063 / `response.rs` E0308，全部 <8192 字节）。详见 **§3.6.1~3.6.3**
- [x] **验收通过**（2026-10-27，**15 步**）：3 个注入错误全部修好，`cargo check --all-targets` 零告警、`cargo test`
  **38 passed**（21 单元 + 17 集成），`auth.rs`（18,488 字节雷区）未被动。详见 **§3.6.4**（含 3 个「超出让编译器闭嘴」的行为，
  以及最要紧的一条：**「通过编译」不足以约束语义** —— 它把第 2 个错误按 `tests/http.rs` 的意图删了字段，猜对了，但那是运气）
