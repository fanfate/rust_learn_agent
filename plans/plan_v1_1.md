# plan_v1_1.md · V1 阶段 1 结账（可观测 + 可重复）

> 建立：2026-11-12　｜　范围：`plan_v1.md` §3 阶段 1
> 基线（实测 2026-11-20）：`cargo test` **54 passed**（tool 43 / agent 8 / model 3）· `cargo clippy --all-targets` **0 warning** · `cargo fmt --check` **clean**
> HEAD `63eb05e`；未提交：`M src/log.rs`、`M src/main.rs`、`?? examples/`、本文件。
> 本文件只记「阶段 1 已发生的事实」+「下一阶段的设计定稿」。路线与判据仍在 `plan_v1.md`，不重复。

---

## 0. 阶段 1 交付了什么

| # | 件 | 状态 | 落点 |
| --- | --- | --- | --- |
| 1 | `Usage` 进 `ChatResponse` | ✅ | `model.rs:363` / `:367` |
| 2 | `RunReport` + `total_info()` | ✅ | `model.rs:393` / `agent.rs:160` |
| 3 | 假 client（离线测循环控制流） | ✅ 8 条用例 | `agent.rs:215` 起 |
| 4 | 截断（`finish_reason == "length"`）处理 | ✅ 顺带 | `agent.rs:73` / `:90` / `:162` |
| 5 | 真跑验收（16 步 / 21 次调用跑通） | ✅ | `data/session-20261120-174022.430.log`（§1.5） |
| 6 | 验收入口移出库（库内不再有 `println!`） | ✅ | `examples/check_new.rs`（35 行） |
| 7 | 日志会话化：一会话一文件 + 时钟统一 UTC | ✅ | `log::session_log_path()`（`log.rs:158`） |
| 8 | REPL 回路 | ⬜ 未做 | 见 §4 #1 |

**验收判据对照**（`plan_v1.md` §3 的四条机械判据）：本阶段达成「可观测」（`Usage`/`RunReport`）与「可重复」（离线测试），REPL 属于「可交互」，仍未开始。

---

## 1. 已完成项的定稿理由

### 1.1 `LlmClient` trait（替换实现的 seam）

```rust
pub trait LlmClient {
    fn chat(&mut self, messages: &[Message], tools: &[WireTool])
        -> impl Future<Output = anyhow::Result<ChatResponse>> + Send;
}
```

- **位置**：`llm.rs:47`，`impl LlmClient for OpencodeGoClient` 在 `:55`（impl 里照常写 `async fn`，这是编译器建议的脱糖）。
- **实测**：`pub trait` + `async fn` 会触发 `async_fn_in_trait`（warn-by-default）⇒ 直接破坏"clippy 0 warning"基线。脱糖成 `impl Future + Send` 可避开，且把 `Send` 写进契约。
- **`&mut self` 的理由**（按强度排序）：
  1. `&mut self` 让 `+ Send` 与普通字段同时成立。**实测反例**：`&self` + `RefCell` + `+ Send` 不编译 —— `error: future cannot be sent between threads safely ... & references cannot be sent unless their referent is Sync`。
  2. LLM 客户端现实中是有状态的（重试计数、token 刷新、会话 id 轮换），`&mut self` 不撒谎。
  3. 假 client 不需要 `RefCell`/`Mutex` 任何一层。
- **代价**：禁止并发调用。但并发只在「并行工具执行 / 子 agent」时有意义，两者均在 `plan_v1.md` §9 的「不做」里；真需要时改 `&self` + `Mutex`，只有两个 impl 要动。
- **不能 `dyn`**：`impl Future` 返回 ⇒ 非 dyn-compatible。V1 不需要运行期换实现，静态分发即可（也避开此前撞过的 E0038）。

### 1.2 假 client（`Scripted`）

- **位置**：`agent.rs:221`，与 8 条用例同在 `#[cfg(test)] mod test`（`agent.rs:215`）。
- **形态**：`VecDeque<ChatResponse>` + `seen: Vec<Vec<Message>>`（每次调用收到的历史快照）。选 `VecDeque` 是因为 `pop_front()` 一个词说清 FIFO；将来回放长录制时也不会踩 `Vec::remove(0)` 的 O(n²)。
- **脚本耗尽**：返回 `Err("脚本耗尽于第 N 次调用")`，不用 `panic!` —— `.unwrap()` 一样会红，但文案能被断言。
- **构造 helper**：`resp(finish, message)` → `say(text)` / `call(id, name, args)` / `calls(&[(id, name, args)])` / `bad_call(id)`，测试体因此塌成几行。
- **`seen` 是核心**：它是唯一能钉住「对话结构」的断言载体，且阶段 2 裁剪的不变量测试（`tool_call_id` 不得悬空）直接复用它。

覆盖矩阵（实测，`cargo test -- --list`）：

| 用例 | 分支 | 关键断言 |
| --- | --- | --- |
| `test_loop_answer_only` | 无 tool_calls ⇒ 收尾 | `steps==1`、`finished`、`seen[0].len()==2`（system+user） |
| `test_loop_call_success` | 工具执行成功 | `executed==1`、`seen[1]` 末条非 `Error` |
| `test_loop_multi_calls` | 一轮两个调用 | `seen[1].len()==5`、id `"1"` 成功 / `"2"` 报错、`requested=2 executed=1` |
| `test_loop_tool_unknown` | `from_name` 失败 | 末条 `Tool` 含 `"未找到符合的工具"`、`executed==0` |
| `test_loop_tool_arg_err` | `parse_kind` 失败 | 参数用非法 JSON（`payload()` 不解析 JSON，实测 `model.rs:37`） |
| `test_loop_tool_run_err` | `Executor::run` 失败 | `executed==1` —— 钉住「executed 含执行报错」的口径 |
| `test_loop_unsupported_call` | `payload()` 失败 | `function: None` ⇒ 不支持的类型 |
| `test_loop_step_over` | 步数耗尽 | `!finished`、`finish_reason==Some("tool_calls")`、`seen.len()==2` |

**未覆盖**：`bail!` 两条输入契约（`choices` 空 / 非 assistant message），见 §4 #3。

### 1.3 截断处理

- `is_truncated` 只提取一次（`agent.rs:73`），两个分支都要，但**含义不同**：
  - 无 `tool_calls` ⇒ 回答被切了 ⇒ 不能宣称"任务完成"；
  - 有 `tool_calls` ⇒ 参数可能是半截 JSON ⇒ **不执行**，回一条 `Error` tool 消息让模型重发（`agent.rs:90`）。
- `total_info()` 三终态（`agent.rs:162`）：`finished && !truncated` / `truncated` / `!finished`。
- 副产品：截断时 `executed` 保持 0 而 `requested` 照常累加 ⇒ **`requested > executed` 成为「本次被截断跳过」的诊断信号**，不必新增字段。
- 参考（实测 pi `bundle/chunks/chunk-VEJDBY3O.js`）：`message.stopReason === "length"` 时走 `failToolCallsFromTruncatedMessage`，一个都不执行，文案 `"...arguments may be truncated. Re-issue the tool call with complete arguments."`。
- 参考（实测 pi `chunk-MMDXLZVS.js`）：`length` 有**两种成因**，用 `isRecoverableLength = stopReason==="length" && maxOutput>0 && usage.output < maxOutput` 区分 —— 输出没撞上限 ⇒ 是**上下文溢出** ⇒ 压缩后重试一次。**我们当前不具备该判据**：`ChatRequest` 一个采样参数都不发，`maxOutput` 对我们未知。

### 1.4 修掉的真 bug：最后一条 assistant 消息没进历史

原代码在 `tool_calls.is_empty()` 分支里 `break` 早于 `messages.push(...)`，导致模型最后那句回答**从不进入对话历史**。REPL 一做就会撞上「第二轮历史断头」。现在 `messages.push(resp.choices.swap_remove(0).message)` 在 `break` 之前（`agent.rs:81`）。

### 1.5 真跑验收（2026-11-20，`cargo run --example check_new`）

```
[+  44.048s] 任务结束 : 输出结束。步数 16 步。 工具调用 21 次，总消耗 82430 tok,
prompt 76679 tok (cached 72960), completion 5751 tok (reasoning 2243).
```

实测确认（这三条**假 client 证不了**）：

| 观察 | 结论 |
| --- | --- |
| `cached` / `reasoning` 真实上报 | `Usage` 里那两个 `Option` 不是猜的字段 |
| `82430 == 76679 + 5751` | `total_info` 现算 `prompt + completion`、不累加 `total_tokens` 的决定成立 |
| `工具调用 21 次`（无括号） | `requested == executed` ⇒ 「只在异常时加括号」在正常路径上静默 |

工具分布：`run_command 7` / `write_file 5` / `read_file 4` / `list_files 4` / `edit_file 1` = 21（44 秒 / 16 步，均值 1.3 次调用每步）。

**没覆盖到的三条**（正好说明假 client 的价值）：截断 0 次、`Error :` 0 条、落盘 0 次（8000 字节上限未撞）。**真跑跑的是 happy path，异常路径完全依赖 §1.2 那 8 条离线用例。**

**由真数据得出的判据修正**：`cached / prompt = 95%` ⇒ 阶段 2 裁剪的主收益是「防溢出」而非「省钱」，主判据不能用 token 降幅（见 §4 #10）。

---

## 2. 现状事实（供后续引用）

| 项 | 值 |
| --- | --- |
| `src/*.rs` 行数 | 2724（`agent 592` / `tool 1310` / `model 452` / `log 163` / `llm 91` / `config 96` / `main 14` / `lib 6`）＋ `examples/check_new.rs` 35 |
| `run_agent` 签名 | `agent.rs:18`：`<C: LlmClient>(client: &mut C, task, max_steps, sink, sandbox_root) -> Result<RunReport>` |
| `RunReport` | `model.rs:393`，6 字段：`steps` / `tool_calls_requested` / `tool_calls_executed` / `finished` / `finish_reason` / `usage: Vec<Option<Usage>>` |
| `is_truncated` | `agent.rs:73`（run 内）/ `:162`（`total_info` 内重新判断，因为 `finish_reason` 已被 `take()` 走） |
| `#[cfg(test)]` | `agent.rs:215` / `tool.rs:685` / `model.rs:402` |
| `main.rs` | 14 行，**空壳**（只剩 `Config::from_env` + `println!`，其余注释掉）—— 等 REPL 填 |
| 验收入口 | `cargo run --example check_new`（`examples/check_new.rs`，含两条硬编码任务） |
| 日志路径 | `log::session_log_path()`（`log.rs:158`）⇒ `data/session-<UTC时间戳>.log`，一会话一文件 |
| `Logger.start_wall` | `Option<DateTime<Utc>>`（`log.rs:26`）。原为 `SystemTime`；实测 chrono 0.4.45 `datetime/mod.rs:1931` 的 `From<SystemTime>` 内部是 `timestamp_opt(..).unwrap()` ⇒ 换掉后少两次转换、少一个 panic 点 |

---

## 3. 设计定稿（未落地）：`Agent<'a, C>`

### 3.1 为什么是「抽象一层」而不是「给 `run_agent` 加参数」

`run_agent` 里**每 run 重建**的四样东西，本来就不是 run 级状态：

```rust
let mut messages = vec![system_prompt, user_task];   // 会话级
let tools = ToolKind::tools();                       // 会话级（恒定）
let tool_exec = Executor::new(sandbox_root)?;        // 会话级 ← 里面还带 clear_tmp
let mut logger = Logger::get_logger(sink)?;          // 会话级？见 §3.5
```

判据：**跨 run 必须活着的进字段，每 run 重算的留局部变量**。按此只有 `RunReport` 应留在 `run()` 内。

### 3.2 形状

```rust
pub struct Agent<'a, C: LlmClient> {
    client: &'a mut C,
    messages: Vec<Message>,
    tools: Vec<WireTool>,
    tool_exec: Executor,
}

impl<'a, C: LlmClient> Agent<'a, C> {
    pub fn new(client: &'a mut C, sandbox_root: &Path) -> anyhow::Result<Self>   // clear_tmp 在这里
    pub async fn run(&mut self, task: &str, max_steps: usize, sink: &LogSink)
        -> anyhow::Result<RunReport>
}
```

`run_agent` 退化成一行 wrapper（8 条用例先只改调用点，断言体不动）。

顺手消灭的三件事：① `clear_tmp` 的 bug 变成**不可能**（构造 = 一次，不是"挪到进程启动"）；② `messages` 天然多轮；③ `tools` 只构造一次。

**`&mut C` 而不持有 `C`** 是刻意的：测试里 `client.seen` 跑完仍能访问。

### 3.3 参考 pi / opencode 得出的结论

pi 分四层（**逐行实测**）：

| 层 | 职责 | 自有状态 |
| --- | --- | --- |
| `runAgentLoop(context, config, emit, signal, streamFn)` | 纯循环 | **无**（原地 `currentContext.messages.push`） |
| `Agent` | 会话状态机 | `_state.{messages,isStreaming,streamingMessage,pendingToolCalls,errorMessage}` + `steeringQueue`/`followUpQueue` + `activeRun` |
| `AgentSession` | 宿主关注点 | extension / compaction / retry / steering / settings |
| `SessionManager` | transcript 持久化 | `fileEntries` / `byId` / `leafId` / branch |

opencode 分三层（**deepwiki 口径，二手**）：`Session`（API + inbox 投递，投递与执行分离）→ `SessionRunnerLLM.drain` / `runStep` → `SessionStep.attempt`（一次 `llm.stream`）；历史**不在 runner 内存**，存 DB 每步重载。

**结论：层数 = 功能数。** 他们多出来的层，正是我们「触发条件式不做」的那些功能（extension、compaction、durable store、steering）。我们需要的正好是两家**中间那层**，且合并成一个 struct。

### 3.4 三条直接照抄的决定

1. **循环保持函数，状态才进 struct**。`finished` / `run_report` / `step` 这些 run-local 变量不许进字段，否则 struct 会成杂物间。
2. **System message 是 `messages[0]` 的一部分，重置时只保留它**。实测 pi `reset()`：`let baseline = getCurrentSystemMessage(this._state.messages)` ⇒ 此前的「隐式空则塞 System」与「单独 `initial_messages()`」两个候选都不选。
3. **`continue()` 是独立入口**，不是「问模型你完成了吗」。实测 pi 的 `continue()` 要求最后一条是 assistant，否则抛错。我们的「外部判据决定再跑一轮」将来照此形状做。

### 3.5 待定：`Logger` 归属

`log.rs:42` 用 `.append(true)` ⇒ 文件可跨 run 累加；但 `start_at` / `start_wall` 是 **per-Logger** 字段，`end_timer()` 在 `TaskResult` 时清空 ⇒ 每轮时间戳从 `[+0.000s]` 重新开始。

- Logger 留在 `run()` ⇒ 读日志看不出「这一轮在会话里的位置」
- Logger 进 struct ⇒ 必须决定语义：每轮归零 / 会话累计 + 本轮 offset

**这是「读日志的人想知道什么」的问题，不是代码问题。** 暂定：Logger 留在 `run()`，等日志出现第二个消费者再定。

### 3.6 现在不为流式 / TUI 预先设计数据结构

结论：**只定接缝位置，不定数据结构形态**。把改动分成两类：

**便宜（到那时再加）**

| 项 | 为什么便宜 |
| --- | --- |
| `enum Event` / delta 类型 | 新增类型 + 在**一个**调用点（`Logger::record`）展开 |
| delta 组装（tool_calls 分片、usage 收尾） | **run-local**，活在 `run()` 内部 |
| `is_streaming` / 半截消息缓冲 | 同上（除非 UI 要中途读） |
| `Arc<Mutex<State>>` | 只有「UI 中途读任意状态」才需要 |
| 取消（abort） | 阶段 3，`select!` + `AbortHandle`，与字段无关 |

**昂贵（现在必须定对）**

1. run-local 变量不许进 `Agent`（§3.4 第 1 条）。
2. **输出的唯一收敛点**：循环里只许 `logger.record(...)`，不许 `println!`。事件化时改这一个函数 + 6 个 `LogKind` 分支，全部输出自动跟着走。
3. **`LogSink` 不许长成「UI 的变体」**。`LogSink::Tui(handle)` 会把 `log.rs` 焊到 ratatui 上。pi 的做法是**回调**（`runAgentLoop(..., emit, ...)`），日志层只说「往哪儿写」。

**`Agent` 四字段在两个未来场景下都成立**：流式改的是 `LlmClient`（加 `stream()`）与循环体（边收边组装边 emit）；TUI 逼改的是输出形态 + 取消，不是状态结构。实测 pi 的 `Agent._state` 里我们没有的字段（`isStreaming` / `streamingMessage` / `steeringQueue`）**都是可加字段，不是结构改动**——不抄的前提是「我们没有采用 UI 从 agent 对象渲染」。

**到时的判据（只留一条）**：

> TUI 只消费事件 ⇒ 保留 `&mut self` 的单任务形态。
> TUI 需要中途读任意状态 ⇒ 才把 `Agent` 拆成 `state` + `runner` 并引入共享所有权。

### 3.7 字段清单与明确不做的

```rust
client / messages / tools / tool_exec        // 只这四个
```

| pi 的字段 | 为什么不抄 |
| --- | --- |
| `isStreaming` / `streamingMessage` | 阶段 3 流式 |
| `steeringQueue` / `followUpQueue` | 中途插话，阶段 3+ |
| `activeRun` | 我们的 `&mut self` 在编译期就禁了并发（pi 只能在运行期 `throw`） |
| `errorMessage` | 我们返回 `Result`，状态里不留错误 |
| `pendingToolCalls` | 我们同步逐个执行，没有「挂起中的调用」 |

---

## 4. 待办（带触发条件）

| # | 内容 | 触发条件 | 状态 |
| --- | --- | --- | --- |
| 1 | **REPL 回路**（`plan_tui.md` 方案 D，≈15 行）+ `Agent` 抽象 | 无（阶段 1 剩余） | ⬜ |
| 2 | `Agent<'a, C>` + `clear_tmp` 进 `new()` | 随 #1 | ⬜ 设计已定稿（§3） |
| 3 | 分支 D 两条 `bail` 用例（`choices` 空 / 非 assistant） | 无（便宜，真零覆盖） | ⬜ |
| 4 | `RunReport.final_answer: Option<String>` | 出现第一个「不打印全流程、只要最终答案」的消费者（`src/bin/eval.rs` / 管道） | ⬜ 不做 |
| 5 | `length` 的两种成因（输出上限 vs 上下文溢出） | 先能发出 `max_tokens`，否则判据不成立 | ⬜ 不做 |
| 6 | 自动压缩重试 | 阶段 2 的 turn 分组 + 边界选择落地后 | ⬜ 不做 |
| 7 | 「外部判据决定继续」 | 出现「模型自称完成但机械判据没过」的实际案例 | ⬜ 不做 |
| 8 | `Logger` 归属与时间戳语义 | 日志出现第二个消费者 | ⬜ 待定（§3.5） |
| 9 | REPL 多轮下，某轮 `?` 提前返回时 `TaskResult` 不打 ⇒ `Logger::Drop`（`log.rs:144`）要到会话结束才报「未正常结束」 | 随 #1 | ⬜ |
| 10 | **阶段 2 主判据修正**：真跑 `cached 72960 / prompt 76679 = 95%` ⇒ 裁剪收益主要是「防溢出」不是「省钱」；主判据用「是否逼近上限 / 是否 400」，token 降幅降为副判据 | 阶段 2 开始前 | ⬜ |

---

## 5. 本阶段明确否决的做法

| 否决项 | 理由 |
| --- | --- |
| 「回传最后一条消息让 LLM 确认完成」 | 自我确认偏差，白烧一轮；pi/opencode 都不做。真实的对应物是 `continue()` 由**外部判据**触发 |
| 为假想的 provider 差异预建归一化层 | 差异吃在 impl 里，不吃在 trait 边界上（`plan_v1.md` §9） |
| `mockall` 之类 mock 库 | 需要的是 Fake + Spy（脚本 + `seen` 快照），30 行手写即可；mock 的期望式断言语义不匹配 |
| `tests/` 集成测试目录 | `tests/` 是独立 crate，看不见 `#[cfg(test)]` 的 `Scripted` ⇒ 假 client 要抄一份；而且它测的是循环内部，不是公开契约 |
| 抽公共的临时目录 helper | `tool.rs::FileHelper` 与 `agent.rs::tmp_root` 需求不同（造文件树 + Drop vs 只要目录存在），未达「同一份逻辑第二处」 |
| opencode 式「每步重载历史」 | 那是 durable store 的代价（可恢复 / 多客户端 / 崩溃续跑），我们没有那个需求 |

---

## 6. 验证命令（本阶段确认过的）

```bash
cargo test 2>&1 | tail -2                                    # 54 passed
cargo clippy --all-targets --message-format=short             # 0 warning（必须带 --all-targets）
cargo fmt --check                                             # clean
cargo test -- --list                                          # 用例清单：tool 43 / agent 8 / model 3
```

---

## 7. 版本记录

| 日期 | 变更 |
| --- | --- |
| 2026-11-20 | `check_new` 移入 `examples/`（库不再有 `println!`）；日志改一会话一文件 `log::session_log_path()`；`Logger.start_wall` 由 `SystemTime` 换 `DateTime<Utc>`（少两次转换 + 去掉 `From<SystemTime>` 内部 `unwrap`） |
| 2026-11-20 | §1.5 补入真跑验收实测（16 步 / 21 次 / 82430 tok / cached 95%）与阶段 2 主判据修正；`check_new` 移入 `examples/`（库不再有 `println!`）；日志改一会话一文件；`Logger.start_wall` 由 `SystemTime` 换 `DateTime<Utc>` |
| 2026-11-12 | §3.6 补入「不为流式 / TUI 预先设计数据结构」的判据与两类改动清单 |
| 2026-11-12 | 建立。阶段 1 结账（`Usage` / `RunReport` / 假 client 8 条 / 截断处理 / `messages.push` 修 bug），`Agent<'a, C>` 设计定稿 + pi／opencode 对照结论入档 |
