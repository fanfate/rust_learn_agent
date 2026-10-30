# my_agent · 学习计划

> 建立：2026-10-16　｜　目标：**一边深入 Rust，一边真正理解 Agent 是怎么运作的**
> 上游：前一个练习项目 `rust_learn_my_api` 的收工复盘（本项目路线源自其 **§五 / §六**）
> 约定：**代码自己写**，本文件只负责"别漏"和"别想错"——不含可复制的完整实现。
> 拆分（2026-10-20）：**本文件 = 路线图 + V1 之后**；**V0 的全部详细设计与实测记录在 [`plan_v0.md`](./plan_v0.md)**（§3.x 编号原样沿用）。
> ⇒ 下文出现的「§3.x」一律指 **`plan_v0.md`** 内的同名节；**`plan_v0.md` 的 §五** = V0 清单，**本文件的 §五** = V1–V3 清单。

---

## 〇、当前状态

- 🎉 **V0 验收通过（2026-10-27）**：在 `scratch/my_api` 副本注入 3 个编译错误，`max_steps = 50`，**实际 15 步自主完成**。
  我方独立复核：`cargo check --all-targets` **EXIT=0 零告警**、`cargo test` **38 passed**（21 单元 + 17 集成）、
  `diff` 显示 `handler.rs`/`response.rs` 与本体**逐字节相同**、`models.rs` 只多一行注释、
  **`auth.rs`（18,488 字节，长文件雷区）一个字没动**。
  三个值得记的行为：① 第 2 个错误它**没走最短路径**——读了 `tests/http.rs` 判断「公开接口不该返回邮箱」，选择删字段而非补字段，
  并主动给出反向改法 ⇒ **「通过编译」不足以约束语义**，这是 V1 的「改契约前先确认意图」的来由；
  ② 自己决定跑 `cargo test` 而不止于 `cargo check`（顺带验证「退出码全走 `Ok`」+「头尾截断」两个决定）；
  ③ 意识到「没有删除工具」（`can't delete files with tools (no rm)`）就用覆盖写绕过 ⇒ 工具集缺口的直接证据。
  详见 `plan_v0.md` **§3.6.4**。
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
  ~~下一个动作 = 循环~~（**已完成，见下条**；当时列的 3 处小债**已清两条**，只剩 `model.rs::ChatRequest` 与 `main.rs` dead code，见 `plan_v0.md` §五）。
- ✅ **循环已跑通（2026-10-22）**：`agent.rs::run_agent` —— `for step in 1..=max_steps` + 六分支 + 工具结果/错误
  **回灌**（`Error: {e:#}`）+ `tool_calls` 为空即收尾。`tool.rs` 已有 `Executor{root,max_bytes}` 不变量载体、
  `resolve_in`（canon + `starts_with`，**返回 PathBuf 供 IO 直接使用**）、`truncate_head` / `truncate_head_tail`
  （字符边界回退 + 总大小报告；**2026-10-27 由单个 `truncates` 拆成两个策略**，见 §〇 末）。
  真实任务跑通两轮：`list_files(".")` → `read_file("readme.md")` → 收尾。
  **实测「缺工具」任务（改名）暴露的四条**见 `plan_v0.md` §3.7.2 末 —— 核心是
  **`write_file` 已在 `ALL` 里但 `raw_invoke` 走 `_ => bail!`** ⇒ 模型看得见却调不通，白烧 2 步；
  以及**步数用尽时模型想说的结论会丢**（V1 需补一次 `tool_choice:"none"` 收尾）。
  **`write_file` 已收工（2026-10-26 v6，11/11 实测通过）**：整文件覆盖 + 自动建父目录（**仅**
  `NotFound` 才 `create_dir_all` 重试）+ 先 `symlink_metadata` 判非普通文件 + `resolve_for_write`
  （`normalize` 消 `..` → 往上 `try_exists()` 找最深已存在祖先 → **canon 祖先** + `starts_with`
  → **返回 `canon_祖先.join(词法尾巴)`**，空尾巴单判）。
  关键坑：`tail` 必须 `canon.strip_prefix(canon_tmp)?`（**词法**祖先）；用 canon 后的祖先会因
  `VerbatimDisk` vs `Disk` 恒失败 → `unwrap_or` 静默吞掉 → `join` 遇绝对路径替换 → 退回"写成功却报
  `prefix not found`"。
  **`run_command` 已收工（2026-10-27 v4）**：无 shell 的 `{program, args}` + **裸名检查与白名单合并到 `raw_invoke`（裸名在前）**
  + `ALLOW_CMD=[cargo,rg,yq,jq]` + **继承环境 + `env_remove` 自己的 key** + `.current_dir(root)` + 三流全显式；
  **退出码全走 `Ok`**（只有 `spawn`/`wait_with_output` 失败才 `Err`），回执三段带标签、空段不打。
  两组实测推翻了「0/1 算成功」：`yq` 真错误也是 1 且只写 stderr、`cargo test` 失败诊断全在 stdout。
  🔴 **env 策略在 2026-10-27 被推翻重做**：`env_clear` + 最小变量集让 `cargo test`/`cargo run` **链接失败**
  （rustc 找不到 MSVC 工具链 ⇒ 撞 PATH 上的 GNU coreutils `link`，报 `link: missing operand after '\377\376'`）。
  往白名单补 `SystemDrive`/`ProgramData` 能救，但那是 rustc 黑盒探测的副产物、跨版本跨机器会变 ⇒ **整段删除**。
  理由是**集合封闭性**，不是严格程度：白名单是从零构造正集（上界不可枚举），负集才闭合。业界主流
  （Codex `shell_environment_policy` 默认 `inherit="all"` 但仍剔除名字含 `KEY/SECRET/TOKEN` 的变量；
  Gemini CLI `sanitizeEnvironment`）都是「继承 + 按名字减」。端到端实测：继承 254 个变量、`LLM_*` 三个全 absent、链接恢复。
  **截断也在这轮重做**：`truncates` 只砍尾巴，而 `cargo test` 的诊断恰好在尾部（1200 行日志把 `panicked`/断言值
  推到 8192 之外 ⇒ 模型只看到 50 行 `test ok`）。现拆成 `truncate_middle`（私有，算两个切点）+ `truncate_head` /
  `truncate_head_tail`，由 `Executor::run` 按 `Invocation` 派发（命令 → 头 3/4 + 尾 1/4，文件/目录 → 只留头）；
  **`MAX_TOOL_OUTPUT` 一个字没改**——同样的字节预算换双倍信息。
  ~~下一个动作 = 跑 V0 验收~~（**已完成 2026-10-27，15 步**，见 §〇 首条与 `plan_v0.md` §3.6.4）。
  **新的下一个动作 = V0 收尾（2026-10-27 重新定序，见 `plan_v0.md` §3.6.5）**：
 ✅ **①`edit_file` + `write_file` 大文件拒写守卫（均已落地）** —— 后者单独加没用、前者单独加会被绕过；
 这一对才是「`read_file` 头部截断 + `write_file` 整文件覆盖 ⇒ 读一半写回静默丢内容」的**结构性**闭合
 （`auth.rs` 18,488 字节，中间 10,368 不可见）。② `read_file` 的 **行号** `offset`（**不是字节** ——
 唯一导航信号是 cargo 的 `file:line:col`，模型没法换算字节；早先「按字节更自洽」是错的）③ 日志改
 `→ 工具名(参数)` / `← N 字节` + `truncate_middle` / `resolve_in` 单测（**`write_file` 侧 2026-10-28 已落地
 `FileHelper` 幂等 guard + 4 用例全绿**；`edit_file` 的 8+2 用例待铺，写法约定见 §3.6.5④）④ 操作审批钩子
  （`Executor.confirm: Box<dyn Fn(&Invocation) -> bool>`，**测试传 `|_| false` ⇒ 无交互可测**）⑤ 之后才进 V1。
  ⚠️ **「意图确认」已拆成两件**：**(a) 操作审批** = V0 收尾（host 强制，行业一致：Codex `approval_policy`、
  Claude Code permission prompt）；**(b) 语义追问**（模型自己停下来问）= **V1 交互式 REPL**，
  单发运行里它问完进程就结束、无法追问 ⇒ 测不了。靠 prompt 让模型自觉提问**不是可测的工程属性**，只能缓解。
  `cargo check --message-format=short` 已写进 `main.rs` 的 prompt（只对当前任务生效），`spec()` 的常驻描述仍未改。
  设计与实测见 `plan_v0.md` **§3.5 末（共 22 节，2026-10-27 最新两节**：`env_clear` 白名单被推翻的三派调研 + 端到端实测、
  截断拆头尾；更早的含 v3 评审**）**；更早的含 rg/jq 走 prompt 还是专用工具、run_command 六组实测、
  `write_file` 覆盖语义、错误文案用 `args.path`、junction 写穿 + 复测 v3/v4。

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
| **V0.5** | **不写新功能**：拿它去修你别的项目里的一个真 bug | 一份「它在哪里断掉」的记录 | 记录出 **≥3 个真实断点** ⇒ **V1 的范围由断点决定，不由本表决定** |
| **V1** | 上下文管理 + 流式/取消 + 最小 eval（+ Skill） | 能连续跑多步真实任务、可中断、能被评测 | 流式分片能正确跨 chunk 累积 `tool_calls`；eval 能回答「改了 prompt 是变好还是变坏」 |
| **V2** | 接回 `my_api`：会话落 sqlite + Axum 包一层 | `POST /chat` + SSE / WebSocket | 复用 `CurrentUser` 做到会话按用户隔离 |
| **V3** | MCP client / server · 多后端抽象 | — | **触发条件**：手上真有一个 MCP server 要接，或要把 agent 暴露出去。不是为了「功能表更齐」 |

**当前唯一在做的：V0 收尾**（`offset` → 日志/单测 → 审批钩子）。V1 之后的事不提前设计。

### 2.1 判据：什么时候该停、该砍（2026-10-28）

**先纠一个错前提**：「手工写的比不过 Claude Code / LangGraph」成立于**通用性**（provider 兼容、边缘 case、UI、eval 体系、
一万小时打磨），**不成立于针对性** —— harness 的质量来自「**为特定任务裁剪**」，不是功能多少。在自己的项目上可以给专属
prompt、砍掉 80% 的通用负担、假定「用户只有一个人且就在旁边」，大厂做不到，因为产品得服务所有人。
所以真问题不是「值不值得写完」，而是 —— **下一步能否改变你将来某个决策**。

| 档 | 判据 | 当前归属 |
|---|---|---|
| **必做** | 不做会污染后续（结构债） | V0 收尾四项 |
| **值得做** | 写完后你对「将来某个决策」更有把握 | 上下文管理 · 流式+取消 · 最小 eval · Skill |
| **砍 / 等触发** | 读别人的比写自己的快；或**没有真实约束** | MCP（等真实 server）· 多 provider · 子 agent · 压缩算法 · 断线重连 |

**两个可执行的停止/继续判据**：

> **该停**：连续两次出现「我想给它加的功能，Claude Code 已经做得更好，且用它比继续写这个项目更省时间」。
>
> **该继续**：你能说出**一件需要你的项目上下文 / 专用工具 / 特定格式的真实任务**，这个 agent 能做而通用产品做不好。
> 第二条极易满足 —— 通用产品永远不会有你项目的知识，这就是「比不过大厂」在这件事上根本不成立的根本原因。

⚠️ **「让项目更接近现代 agent」是个陷阱**：现代 agent 的功能表是**约束的产物**，不是原因的清单 —— 它们加 MCP 因为要接生态，
加 Skill 因为 prompt 撑爆了，加流式因为用户等不了 30 秒。你现在没有这三个约束，加了就是装饰。

**vibe coding 的分界线**（按「是否需要判断」划，不按文件划）：

| 交给 AI | 自己写 |
|---|---|
| serde 类型定义 · CLI 参数解析 · 日志格式化 · 测试用例铺量 · 文档 · 探针脚本 | 任何「这个抽象该不该存在」的决定：trait/enum 划界、错误边界切在哪、状态放哪、何时 panic |

判据很硬：**前者写错编译器会告警，后者写错编译器一声不响**（`Vec<Box<dyn>>` 101 次分配 vs `Vec<enum>` 1 次、
`Step` 32B vs 96B —— 全都编得过）。切到纯 vibe coding 的实际损失不是洁癖：遇到 bug 会分不清是**设计问题**还是**实现问题**。

**若最终决定停**：最有价值的收尾不是补完 `offset`，是一份**从零重述的设计笔记**（为什么 enum 不 trait、为什么继承 env
而不是白名单、为什么 edit 用锚点不用补丁、为什么截断按输出性质分派）。`plan_v0.md` 记的是**过程**，笔记要的是**结论**；
半年后会忘掉 80%，能写下来的那 20% 才是真正内化的部分。

---

## 三、V0 详细 → 已归档到 [`plan_v0.md`](./plan_v0.md)

> 📦 V0 的设计过程、协议实测报文、类型决策日志、安全边界与验收标准，全部搬到了 **`plan_v0.md`**（约 3700 行）。
> 章节号**原样保留**（§3.1 … §3.9），所以别处笔记里的「详见 §3.5」「§3.4.4」仍然有效，只是指向的是 `plan_v0.md`。
> 本文件从此只负责：**路线图 + V1 之后的设计**。V0 的新发现写 `plan_v0.md`；只有**影响 V1+ 设计**的结论才在这里留摘要。

### V0·0 现状速览

| | |
|---|---|
| **已完成** | `.env` 三变量 · `config.rs` 五版定稿（方案 C 已落地） · 真实通道 **200** · **实验 2 / 2.5 闭环**（工具调用 → 回灌 → 收尾，prompt cache 命中 256） · **`model.rs` 类型齐了** · **`llm.rs` 真实 200** · **循环跑通**（`for step in 1..=max_steps` + 错误回灌 `Error: {e:#}`） · **四个工具全收工** `read_file`/`list_files`/`write_file`/`run_command`（路径沙箱：读侧 canon、写侧祖先 canon；命令：裸名+白名单+继承环境+三流显式） · **截断按输出性质分派**（命令头+尾、文件只留头） · **`write_file` 大文件拒写守卫**（2026-10-27，阈值传 `max_bytes` 参数） · **`edit_file` 落地**（2026-10-28，三态匹配 + 起始行号取自旧文本，见 §3.6.3 review） · **单测基座 `FileHelper`**（2026-10-28，幂等 guard，`write_file` 4 用例全绿，见 §3.6.5④） |
| **正在写** | 无。**工具层与循环层都通了**，`cargo check` / `cargo test` 零 error 零告警，端到端实测 6 组通过 |
| **下一个动作** | 🔴 **① `read_file` 行号 `offset`**（设计已定稿、待实现：**1-based 明写 `1-indexed` · 不加行号前缀 · 尾注报「第 a-b 行 / 共 N 行 / 用 offset=b+1 继续」· 越界 bail 报总行数 · `truncate_head` 改返回结构体**，五家调研见 `plan_v0.md` §3.6.5⑤）→ **② 日志 `→/←` + 补单测**（`write_file` 4 + `edit_file` 13 用例已绿；`truncate_middle`/`resolve_in` 仍 0 覆盖）→ **③ 操作审批钩子** → **④ V0.5 真实任务 dogfooding** → **⑤ 进 V1**（验收已于 2026-10-27 通过，15 步）见 `plan_v0.md` §3.6.5 |
| **验收** | 在 `scratch/my_api` 副本上，说一句"让它通过编译"，agent 自主「读 → 改 → `cargo check` → 读报错 → 再改」，**中途不需介入** |
| **欠账** | `config.rs` / `truncate_middle` / `resolve_in` / `edit_file` 的单测（`FileHelper` 已就位，剩下的只是铺用例）；`model.rs::ChatRequest` 零调用点待删；**语义追问**（模型主动提问）要等多轮 REPL，属 V1 |

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

**V0** → 完整清单见 [`plan_v0.md` §五](./plan_v0.md)（**51 项已勾 / 12 项未勾**，截至 2026-10-28，**验收已通过**）。
**工具层与循环层已全部打通**：四个工具（`read_file`/`list_files`/`write_file`/`run_command`）都在，
路径沙箱读侧 canon + 写侧祖先 canon，命令侧裸名+白名单+继承环境+三流显式，截断按输出性质分派；
`cargo check --all-targets` 零 error 零代码告警，`cargo test` **4 passed**，`Executor::run` 端到端 6 组实测通过
（含真实 `cargo test` 的尾部诊断、200 个编译错误的头部截断、`LLM_*` 不泄漏且链接正常）。

剩下的 12 项里，**没有一项挡路 V0 验收**：单测 3 处（`truncate_middle`/`resolve_in`/`edit_file` 的用例，`FileHelper` 基座已就位）、
日志 `→/←` 1 处、`ChatRequest` 零调用点 1 处、`spec()` 加一句 flag 1 处，其余是 V1 才要的（`LlmClient`、`usage`、
`tool_choice:"none"` 收尾、安全边界升级）和「自答 §3.8 的 4 问」。
**V0 验收已通过（2026-10-27，15 步）。下一个动作 = 行号 `offset` → 日志/补单测 → 操作审批 → V0.5 → 进 V1**
（`write_file` 拒写守卫 + **`edit_file`** + **单测基座 `FileHelper`** 均已落地，2026-10-28：三态匹配、`replace_all` 默认 false、
起始行号取自**旧文本**——「在 `new_file` 上用 `new_string` 反查」实测会差 303 行；`edit_file` 的代价是峰值内存 2.00×、
耗时全在 I/O；`FileHelper` 的幂等实证见 `plan_v0.md` §3.6.5④）。
**`offset` 的设计已定稿（2026-10-29，五家调研见 §3.6.5⑤）**：1-based 明写 `1-indexed` / **不加行号前缀**（省 15.6% 字节）/
尾注必须有「第 a-b 行 · 共 N 行 · 用 offset=b+1 继续」/ 越界 **bail 报总行数**（Gemini 的静默收敛是反例）。
✅ **`truncate_*` 返回结构体已落地并收敛到两个切点**（`Truncation{head_end, tail_start}`，2026-10-29 四轮追问后定稿）：
`total_bytes` / `total_lines` / `kept_lines` / `middle_lines` 逐个论证后**删除** —— 判据是「core 只装源上没有、只能从计算过程
得到的东西」，其余一律做方法（`head_lines` = `lines().count() - head_partial`）；`truncate_head` / `truncate_head_tail`
暂留为「固定参数 + 拼文案」薄壳（判据：薄壳不许长出策略）。**尚未接入**：`read_file` 自截 + offset 尾注 ⇒ 届时
`Executor::run` 的 `_ =>` 分支必须拆，否则 `read_file` 拿到的是不含 offset 指引的命令式文案。实测与遗留见 §3.6.5⑥。

**V0.5（下一步的现实形态，2026-10-28 新增）**

- [ ] 拿 agent 去修一个**真实 bug**（`my_api` 之外的、你确实要修的那种），全程记录它断在哪：
  没有 REPL ⇒ 无法纠正它的错误方向；上下文会撑爆；卡住了只能 Ctrl-C 杀进程
- [ ] **断点清单 ≥3 条** ⇒ 据此决定 V1 做哪几件（这才是「功能由真实需求驱动」的落点）

**V1**（2026-10-28 按 §2.1 判据重排 —— **不再是功能清单**）

**第一档 · 值得做**（三件同档，没有一件是「加功能」）：

- [ ] **上下文管理**：历史裁剪 / assistant·tool 消息配对协议
  —— **唯一还没拿到的真知识**。它是**策略不是实现**，读一万行别人的代码也学不会「什么时候丢历史、丢了怎么保证 tool 配对不坏」
- [ ] **手写 SSE 流式 + 取消/超时**（`CancellationToken` + `tokio::select!`，Ctrl-C 可中断）
  —— 与「超时取消」是同一件事的两半（非流式只有一次 `await`，`select!` 没有可选的）。细节见 §5.1
- [ ] **最小 eval**：把 V0 验收那次「注入 3 个 bug 看能不能修」固定成可重复的（固定任务 + 固定判定）
  —— **没有 eval，后面所有 prompt / 工具改动都是猜**

**第二档 · 低成本可选**：

- [ ] **Skill 支持**（≈1~2 小时，复用现有工具）：`list_skills` = `list_files` 加一层过滤、`read_skill` = `read_file`。
  价值三件：① system prompt 从**硬编码字符串**变成外部可编辑文档 ② **description 本身是 token 预算项**（100 个 skill 的
  name+desc 可能吃掉 5000 token，而内容一个字没读 —— 这是 `spec()` 那件事放大到系统级）③ 理解「为什么是模型**自己决定读哪个文件**，
  而不是检索式 RAG」—— 因为 agent 有工具调用，**它可以主动取**，不需要你猜它要什么。
  它同时是「上下文管理」的**最小实验载体**（比历史裁剪算法更容易上手、立刻可观察）

**第三档 · 等触发条件**：

- [ ] MCP **client**（官方 SDK `rmcp` v3.6，rustc 1.88 + edition 2024；**排在 `Vec<Box<dyn Tool>>` 注册表之后** ——
  MCP 就是「一种 Tool 实现」，注册 = push 一个 adapter：`tools/list` → `FunctionDecl`、`tools/call` → 回灌字符串。
  V0 的 `enum Invocation` 派发用不了，因为工具名/schema 是运行期才知道的）
  **触发条件：手上真有一个 MCP server 要接**（filesystem / github），不是为了「接近现代 agent」。
  它的价值不在报文（那部分两小时能抄完），在**证伪 5 条 V0 假设**：
  ① `ToolKind::ALL` 是编译期常量数组 ② `tools()` 在循环外算一次 ⇒ 清单运行期不可变（`list_changed` 直接冲突）
  ③ `enum Invocation` 的穷尽 match ④ `ALLOW_CMD` 写死 ⑤ `run_command` 继承环境（判定「同 UID 本地工具链可信」）
  —— 第 ⑤ 条最值钱：你论证过「外部/不可信才该用 env 白名单」，但那个分支**你永远没真写过一次**，MCP 是它唯一的使用场景。
  合理姿势：**让 AI 铺 JSON-RPC 报文层，注册表与安全边界自己改**

**明确不做**（写下来比记在脑子里有用）：

- 断线重连 / `id` 去重 / `retry` 退避 / 自定义 `event:` 类型
- 多 provider 抽象 / 子 agent / 上下文压缩算法（抄不过别人，且没有真实约束）

**V2**

- [ ] 会话与消息落 sqlite（复用 `my_api` 的 sqlx 层 + 迁移工作流）
- [ ] Axum 包一层：`POST /chat` + SSE / WebSocket
- [ ] 鉴权复用 `CurrentUser` → 会话按用户隔离

**V3**

- [ ] MCP **server 化**（把 agent 自己暴露成 MCP server，与 V1 的 client 方向相反）
- [ ] 多后端抽象

### 5.1 流式到底新在哪（2026-10-28 用户指出「之前说不砍」后改判）

此前把它归成「当玩具」是错的，错在**只想了协议、没想过消费**。四块拆开，只有第一块是 Python 里已经懂的：

| 块 | 有新知吗 |
|---|---|
| ① SSE 线路协议（`data:` / `event:` / `id:` / `retry:` / 双换行分隔） | **零**。懂 field 机制就够了，任何语言一样 |
| ② **chunk 边界 ≠ 行边界** | ✅ 有，且 Python 库（`iter_lines` / `httpx-sse`）帮你藏掉了 |
| ③ **UTF-8 边界** | ✅ 有，与 `truncate_middle` 里的 `is_char_boundary` **同源** |
| ④ **流式 `tool_calls` 的增量语义** | ✅ 有，且**最有价值**（与语言无关，各家 provider 还不一致） |

**② 实测：两种失败模式都真实存在。** `bytes_stream()` 给的是**任意字节切分**的 `Bytes`：

| 切点 | 朴素按 chunk 各自 `lines()` | 缓冲到 `'\n'` 再切行 |
|---|---|---|
| 切在 10（`data: {"a` 内部） | `["{\"a\"", "{\"b\":2}"]` ← **残片**，JSON 解析时才炸 | `["{\"a\":1}", "{\"b\":2}"]` |
| 切在 17（`da\|ta` 中间） | `["{\"a\":1}"]` ← **静默丢掉整个事件** | 同上，2 条完好 |

教训：**残片会在解析时报错，丢事件完全静默** —— 后者才危险。

**③ 实测：分别 `from_utf8_lossy` 再拼接 = 数据不可恢复。** 「中」被切成 1+2 字节时：

```
分别 lossy 后拼接 = "data: {\"content\":\"\u{fffd}\u{fffd}\u{fffd}文测试\"}"   ← 3 个替换字符
缓冲后解码        = "data: {\"content\":\"中文测试\"}"
```

正确姿势：`Vec<u8>` 缓冲 + `str::from_utf8` 失败时只消费 `valid_up_to()`。这是「**收的时候别提前解码**」，
和 `truncate_middle` 的「**切的时候别切坏**」是同一知识的两面 —— 同一件事已经踩过一次。

**④ 才是真正的价值。** V0 只见过**完整形态**（`function.arguments` 是一个完整 JSON 字符串）；流式下变成
`delta.tool_calls[0].index` + `function.name`（可能只在第一片给）+ `arguments` 是**字符串片段**要按 index 拼完才 `from_str`。
**这部分推不出来，只能实测报文。**

两个连带影响：

- **Rust 侧**：`bytes_stream()` 是 `impl Stream`（不是 `Iterator`），要 `futures_util::StreamExt`；若想写
  `trait LlmClient { async fn stream(..) }` 会撞 **E0038 dyn 不兼容**（已验证过的坑），流式把这个 trait 边界的代价放大了
- **架构侧**：现在 `Executor::run` 返回 `String`，`truncate_head` 是**事后**截断。流式之后字已经打到终端了，
  「事后截断」模型失效 ⇒ 必须**边收边决定要不要继续打**。这是 `truncate_head` 在新场景下的真实失效点，不是装饰

**边界**：做 content 增量打印 · `tool_calls` 按 index 累积 · 行/UTF-8 双缓冲 · Ctrl-C 取消；
**不做**断线重连 / `id` 去重 / `retry` 退避 / 自定义 `event:` 类型。

**复利**：`chunk 边界不可控 + 增量状态` 这个模式，在 MCP 的 stdio 流、日志 tail、WebSocket 里会再出现一遍。
另外它恰好是「**编译器一声不响**」的典型（切坏 UTF-8、丢事件，全编得过）⇒ 按 §2.1 的分界线，**属于自己写那一列**。

---

## 六、通用提醒

- **对"新版库 API"的断言不可靠时**：直接 grep 本机 registry 源码（`D:\Rust\.cargo\registry\src\`），
  编译器和源码不会说谎。这条上个项目已经验证过多次。
- **时间预期**：doc §5.3 那句仍然成立 —— "核心代码也就这么一个循环"是极度乐观的说法，
  真正的活在流式、可靠性、上下文那一圈。**按两周以上算，不是"一个周末"。**
