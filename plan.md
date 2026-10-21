# my_agent · 学习计划

> 建立：2026-10-16　｜　目标：**一边深入 Rust，一边真正理解 Agent 是怎么运作的**
> 上游：前一个练习项目 `rust_learn_my_api` 的收工复盘（本项目路线源自其 **§五 / §六**）
> 约定：**代码自己写**，本文件只负责"别漏"和"别想错"——不含可复制的完整实现。
> 拆分（2026-10-20）：**本文件 = 路线图 + V1 之后**；**V0 的全部详细设计与实测记录在 [`plan_v0.md`](./plan_v0.md)**（§3.x 编号原样沿用）。
> ⇒ 下文出现的「§3.x」一律指 **`plan_v0.md`** 内的同名节；**`plan_v0.md` 的 §五** = V0 清单，**本文件的 §五** = V1–V3 清单。

---

## 〇、当前状态

- ✅ 项目已 `cargo init`（`edition = "2024"`）
- ✅ 前置能力已在 `my_api` 练过，**V2 阶段直接复用**：Axum 路由/中间件 · SQLx + 迁移 · JWT 鉴权 · 错误分层 · 集成测试
- ✅ 本机工具链：`rustc 1.99.1` / `cargo 1.99.1`
- ✅ **接入方式已定案（2026-10-16）**：**统一走 OpenAI 兼容接口** —— `POST {LLM_BASE_URL}/chat/completions`。
  这是线上各家与本地推理服务（llama.cpp / Ollama / vLLM…）的**公约数**，所以 **"线上还是本地"不是一道取舍题**：
  换 `.env` 里那两个值即可，**代码一个字都不用改**。选它的理由：省掉部署与权重管理，把精力放在 agent 循环本身；
  本地推理随时能当"换个 base_url"接回来，留作 V3+ 的可选项。API key 已有。
  ⇒ `my_api` doc §5.1「本地模型条件已具备」与 §5.3 第 2 条「先云端验证再切本地」**就此作废** ——
  这件事已退化成一条环境变量，不再需要"迁移"决策。
- ✅ **配置方式已定案（2026-10-16）**：`.env`（不入库）+ `.env.example`（空壳入库），三变量
  `LLM_BASE_URL` / `LLM_MODEL` / `LLM_API_KEY`，用 `dotenvy` 读取 —— 详见 **§3.3.1**。
- ✅ **API 已选定并跑通（2026-10-18）**：OpenCode **Go** 通道 —— `LLM_BASE_URL=https://opencode.ai/zen/go/v1`、
  `LLM_MODEL=deepseek-v4.1-flash`（`GET /models` 实测该 id 存在）。三变量抽象生效：全程只改 `.env`，`config.rs` 与请求代码零改动。
  ⚠️ 该通道有**两条私有要求**（属 provider 私有约定，**未写进通用规范**，换家时可删）：`User-Agent` 必须自定义（否则 Cloudflare `403 / error code 1010`）、
  `x-opencode-session` 必须带（否则 `400 MissingSessionID`）。→ 详见 **§3.9**。
- ✅ **scratch 目录已定位（2026-10-16）**：`my_agent/scratch/`，语义是 **agent 的沙箱根**，不是"测试目录"—— 详见 **§3.5**。
- ✅ **`src/config.rs` 已成型（2026-10-16，**五版迭代**）**：`ConfigError`（thiserror，3 变体 `Missing` / `NotUnicode` / `Empty`）
  + 私有 `get_env` / `format_url` / `get_map` + `Config`（**手写 `Debug` 把 key 打 `"***"`**）+ `from_env()` / `from_map()`。
  `cargo check --all-targets` **零代码告警**（只剩 `target/` 增量锁文件噪音）。
  ✅ **两条取值链已交汇（方案 C 已落地）**：`from_env` 直接 `Self::from_map(&get_env)`，**生产路径经过被测内核** → 详见 **§3.3.2**。
  ⏳ **唯一欠账**：`#[cfg(test)] mod tests` 还没写（§3.3.2 末 4 问）—— 有意留到 V0 跑通后再补。
- ✅ **`main.rs` 已从"裸探针"走到"闭环验证"（2026-10-18 → 10-19）**：先 200 打通、再带 dummy `tools` 拿到真实
  `tool_calls`、再回灌一轮拿到 `finish_reason:"stop"` —— **"发送 → 收调用 → 回灌 → 收尾"整条链路成立**。
  三份原始报文见 `plan_v0.md` **§3.9**（`model.rs` 的唯一事实来源）。
  ⚠️ 它现在是**协议探针，不是 agent**：工具有假结果、只有硬编码两轮、没有循环与 `max_steps`。
- ✅ **`model.rs` + `llm.rs` 已成型（2026-10-20）**：`Message`（`content` 已按 §3.4.13 收紧）· `ToolCall::payload()`
  · `FunctionDecl`/`ToolKind`/`ALL`/`spec()`/`tools()` · `Invocation` 装箱 `XxxArgs` + `parse_kind` · `ChatResponse`/`Choice`
  · `OpencodeGoClient`（手写 `Debug`、`bearer_auth`、私有 `WireRequest<'a>`）。
  **真实 200 跑通，`finish_reason:"tool_calls"`、`read_file{"path":"scratch/readme.md"}` 解析进类型**。
  **下一个动作 = 循环**（先补 3 处小债：删 `to_value` 二段序列化 / 先判状态码 / 删 `ChatRequest` 与 `main.rs` 旧探针），
  然后 4 个真工具 + `max_steps`。

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
| **V0** | 无框架 · 非流式 · CLI · 单文件约 300 行 | 一个能用的命令行 agent | 见 `plan_v0.md` §3.6（含 **dogfooding 一次真实修复**） |
| **V1** | 手写 SSE 流式 + 取消/超时 + 上下文管理 | 可 Ctrl-C 中断、不撑爆上下文的版本 | 流式分片能正确跨 chunk 累积 `tool_calls` |
| **V2** | 接回 `my_api`：会话落 sqlite + Axum 包一层 | `POST /chat` + SSE / WebSocket | 复用 `CurrentUser` 做到会话按用户隔离 |
| **V3** | MCP · 多后端抽象 | — | 到这一步才真正知道抽象该划在哪 |

**当前唯一在做的：V0。** V1 之后的事不提前设计。

---

## 三、V0 详细 → 已归档到 [`plan_v0.md`](./plan_v0.md)

> 📦 V0 的设计过程、协议实测报文、类型决策日志、安全边界与验收标准，全部搬到了 **`plan_v0.md`**（约 1600 行）。
> 章节号**原样保留**（§3.1 … §3.9），所以别处笔记里的「详见 §3.5」「§3.4.4」仍然有效，只是指向的是 `plan_v0.md`。
> 本文件从此只负责：**路线图 + V1 之后的设计**。V0 的新发现写 `plan_v0.md`；只有**影响 V1+ 设计**的结论才在这里留摘要。

### V0·0 现状速览

| | |
|---|---|
| **已完成** | `.env` 三变量 · `config.rs` 五版定稿（方案 C 已落地） · 真实通道 **200** · **实验 2 / 2.5 闭环**（工具调用 → 回灌 → 收尾，prompt cache 命中 256） · **`model.rs` 类型齐了**（`content` 已收紧、`Invocation` 装箱 Args、`parse_kind` 一步解析） · **`llm.rs` 真实 200**（手写 `Debug`、`bearer_auth`、私有 `WireRequest<'a>`） |
| **正在写** | **循环**（`for step in 0..max_steps`，六分支见 `plan_v0.md` §3.7.2）→ 然后 4 个真工具 |
| **下一个动作** | ① 三处小债：`chat()` 删 `to_value` 二段序列化 / 先判状态码再读 body / 删 `model.rs::ChatRequest` 与 `main.rs` 旧探针 ② 循环接上（`client` 从参数进来、返回不出现 `reqwest` 类型）③ 4 工具实现 + 沙箱 + 截断 ④ **最后**抽 `LlmClient` —— 顺序依据见 `plan_v0.md` §3.7.2 |
| **验收** | 在 `scratch/my_api` 副本上，说一句"让它通过编译"，agent 自主「读 → 改 → `cargo check` → 读报错 → 再改」，**中途不需介入** |
| **欠账** | `config.rs` 的 `#[cfg(test)] mod tests`（有意留到 V0 跑通后） |

### V0·1 归档索引（内容都在 `plan_v0.md`）

| 节 | 主题 | 什么时候回去查 |
|---|---|---|
| §3.1 | 范围：4 工具 · 非流式 · CLI · 安全边界 | 想给 V0 加功能时，先看这里划的线 |
| §3.2 / 3.2.1–3.2.3 | `impl`/`dyn`/`Box` **是两维**（判据：需要拥有它吗）· enum vs trait = 闭合 vs 开放 · **建模词汇缺口**（为什么总先想到 struct + trait） | 每次要定"这该是 trait 还是 enum"时；配实测代价（101 次分配 vs 1 次，`Step` 32B vs 96B） |
| §3.3 / 3.3.1–3.3.2 | 实测依赖版本 · `.env` 方案 · **`config.rs` 五版演化**（方案 C / 遮 key 遮 `Debug` / `ValueError` 该不该 thiserror / HRTB 闭包） | 改 `config.rs` 或再遇到"两条平行实现链"时 |
| §3.4 / 3.4.1–3.4.14 | **协议备忘**：`tool_calls` 解剖 · serde 四种 tag 策略 · `arguments` 为何是字符串 · **两层建模** · 请求侧 ≠ 响应侧 · `type` 是判别标签（还有 `custom`）· `#[serde(default)]` 精确语义 · **`tools` 单一来源 / 注册表三级** · **注册表与执行层两个 enum** · **字段用 `&'static str` 还是 `String`** · **`args` 交给 Args struct 解析（别 untagged）** · **Args struct 成本核算（7 处改动 / 2 处静默）** · **`content` 不是处处可选（`user:null` 会幻觉）** · **请求/响应体缺什么** | 写 `model.rs` / `llm.rs` 时的主要依据 |
| §3.5 | 安全边界：scratch 是**沙箱根**（执行点 + 靶场 + 一次性） | 落地工具实现前必读 |
| §3.6 | 验收标准：比的不是功能数，是可靠性 | 定义"V0 做完了" |
| §3.7 / 3.7.1–3.7.6 | 工程化分档 · 错误策略分层（**可以 panic / 绝不可 panic**）· **`LlmClient` 什么时候抽（跑通后抽，先留缝）** · **`Config` 字段归谁 + api_key 必须走 `bearer_auth`（自动 `Sensitive`）** · **「用接口」vs「用 JSON 拼」是两件事** · **`Debug` 写不写（手写 / newtype / 不实现；`unwrap` 不受影响）** · **`chat()` 收参数列表还是 `ChatRequest`** · **循环要不要写成状态机（判据：一条出边就不算状态）** | 决定某项要不要在 V0 做；写错误类型时；写循环/client 时 |
| §3.8 | 要自答的 4 个问题（空 `tool_calls` vs 纯文本、`Err` 怎么回灌、白名单怎么不靠字符串判断、上下文何时撑爆） | 动手前过一遍，仍然有效 |
| §3.9 | 第一动作 → **实验 1 / 2 / 2.5 的完整报文** · provider 两条私有 header · 请求侧 `FunctionDecl` | **写类型前先回来核对真实报文**，别凭记忆 |

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

**V0** → 完整清单见 [`plan_v0.md` §五](./plan_v0.md)（**23 项已勾 / 15 项未勾**，截至 2026-10-20）。
当前卡在：**循环**。`model.rs` / `llm.rs` 已能发请求并把 `tool_calls` 解析成类型（真实 200），
但工具是假的、没有 `for step in 0..max_steps`。下一步先把 3 处小债清掉，再接循环与 4 个真工具。

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
