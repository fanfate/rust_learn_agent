# plan_tui.md · TUI 调研与选型

> 建立：**2026-11-11**　｜　状态：**调研结论，不是执行计划**。本文只回答「值不值得做、什么时候做、用哪一层」，
> 不含要写的代码，也不预设实现方案（按 `AGENTS.md` §2，设计题给判据不给结论）。
> 上游：`plan.md` §2.1（必做/值得做/砍 三档判据）· §5.1（流式四块）· §七（一条轴规矩）· `plan_v1.md` §7 / §9（审批钩子）·
> `plan_v0.md` §3.6.6（日志层三类型 + 为什么抽 trait 要等第二个实现）
> 标记：**[实测]** = 本机命令或源码可复核（复核命令见 §11）　**[推断]** = 未验证，别当事实引用

---

## 0. 一句话

**TUI 值得做，但「现在把项目移植到 ratatui-kit」不值得。** 三句话拆开：

1. **值得做的理由不在 UI**，在于它是对**已有三条设计**的第一次真实压迫测试：谁拥有 stdout（`plan_v0.md` §3.6.6）、
   `Executor::run -> String` 还成不成立（`plan_v1.md` §10 待答 #5）、流式「边收边决定要不要继续打」（`plan.md` §5.1）。
2. **不值得的是顺序**：V1 阶段 3（流式 + 取消）本来就要动输出的返回/打印路径 ⇒ TUI 先行 = 同一层写两遍。
   ⇒ **TUI 是阶段 3 的验收消费者，不是阶段 0 的前置。**
3. **不值得的是选型**：ratatui-kit 是「把渲染循环抽象掉」的框架（`#[component]` + hooks + router + atoms，20,271 行），
   与本项目 §1「刻意不抽象循环」、§4「框架章跳过」是同一条判据的反面用法。学 TUI 要学**约束**，不是学 **DSL**。

⚠️ **REPL ≠ TUI**（本文最重要的一条防线）：审批 `Y/n`、语义追问、连续多轮，要的是**回路**不是**图形**。
`println!` + `stdin.read_line()` 十几行就能满足；把它偷换成「我要一个界面」会把半天的活放大成三周（§7 方案 C）。

---

## 1. 地基：这件事是四层，不是两个库

> 起因：曾以为「ratatui 和 ratatui-kit 是同一套」。实际是四层，且 **ratatui 0.30 自身已拆成一组 crate**。

| 层 | 是什么 | 本机实测 |
|---|---|---|
| **crossterm 0.29.0** | 终端后端：raw mode · alternate screen · 事件读取 · 键位 · 鼠标 | `src/` 含 `event/ terminal/ cursor/ clipboard.rs`；**已缓存** |
| **ratatui 0.30.2** | 渲染门面（`Buffer` / diff / `Terminal::draw` / `Widget`） | **只是壳**：`src/` 仅 `lib.rs prelude.rs init.rs widgets/`；**已缓存** |
| ├ `ratatui-core 0.1.2` | 核心：`Terminal` `Buffer` `Layout` `Backend` **`backend/test.rs` = `TestBackend`** | **已缓存** |
| ├ `ratatui-widgets 0.3.2` | 现成控件（Table/List/Gauge…） | **38 文件 / 29,723 行**；**可以不用它画的控件，但甩不掉它** —— [实测] 它是 `ratatui` 的**非 optional 依赖**（`ratatui-0.30.2/Cargo.toml:244`，`default-features=false` 但仍编）。真想不编，只能不经门面、直接依赖 `ratatui-core` + `ratatui-crossterm` |
| ├ `ratatui-macros 0.7.2` / `-crossterm` / `-termwiz` / `-termina` | 宏 + 三个可选后端 | `crossterm 0.29` 已缓存 |
| **ratatui-kit 0.12.0** | 在上一层**再加** 组件 / props / hooks / router / atoms | **84 文件 / 20,271 行**；`edition 2024`，作者 yexiyue |
| └ 其依赖 | `ratatui 0.30.1` `crossterm 0.29` `futures 0.3.32` `generational-box` `any_key` `textwrap` `ratatui-kit-macros 0.6.2` | 默认 feature 离线可编；可选 `regex/serde/tui-input/tui-tree-widget/tui-widget-list/unicode-width` |
| └ 缓存缺口 | `tui-input 0.15.3` · `tui-tree-widget 0.24.0` · `tui-widget-list 0.15.2` · `any_key` **未缓存** | ⇒ 开 `input`/`textarea`/`tree`/`table` 要联网拉包（`AGENTS.md` §0 第 5 条） |

⇒ 所以「裸 ratatui」这个说法也不准：真正要的那层是 **crossterm + `ratatui-core`**，`ratatui-widgets` 那 29.7k 行可全跳。

---

## 2. 生态实测：四个 CLI agent 的 TUI 栈

| 产品 | TUI 栈 | 证据（[实测] 抓取/源码） |
|---|---|---|
| **Claude Code** | **TS + 自 fork 的 Ink**（React reconciler 指向终端节点，非引用 npm 包） | `src/ink/` 含完整引擎：`reconciler.ts` `dom.ts` `renderer.ts`（Yoga flexbox + chalk） |
| **Gemini CLI** | **TS + 直接用 npm `ink`** | `packages/cli/src/interactiveCli.tsx`: `import { render } from 'ink'`；`ui/App.tsx`·`components/Composer.tsx` |
| **Codex CLI** | **Rust + ratatui** | `codex-rs/tui/Cargo.toml`；`custom_terminal.rs` 注释 "derived from `ratatui::Terminal`"；TS 只剩 npm 安装壳 |
| **OpenCode** | **Go + bubbletea**（Charm 生态） | `internal/tui/tui.go`；Elm 风味 `Update/View`；另有 issue 讨论 "Evaluate migrating Go-based TUI to Ink" |
| **pi（本 harness）** | **TS + 自研 `@earendil-works/pi-tui`**（无 ink / 无 react） | 见 §2.2 |

**结论修正**：不是「基本都是 TS 的 React」。真格局是 —— **应用/桌面层**（VS Code 插件、Electron）是 TS 天下；
**纯终端 TUI** 是 Go(bubbletea) 与 Rust(ratatui) 的战场；TS(Ink) 占了**声量最大的两个**。四家 2:2。

### 2.1 opencode 的「1,333 行」是一个文件，不是整个 TUI

> 起因：把 `internal/ui/model.go`（~1,333 LOC）当成了整个 TUI 包的规模。

[实测] GitHub tree API（`main`，sha `73ee4932`）：

```
internal/tui/  →  48 个 .go 文件 · 299,734 字节
最大文件：tui.go 25,545B · components/chat/message.go 18,774B
          components/dialog/permission.go 15,211B · dialog/filepicker.go 12,621B
对照：internal/ 全体 137 个 .go，平均 9,173 字节
```

⇒ **约 7–9 千行**（按 30–40 字节/行 [推断]，未 cloc）。占该项目 Go 代码 **48/137 ≈ 35% 文件、≈24% 字节**。

### 2.2 pi 的 TUI（本机源码是最硬的一份证据）

路径：`C:\Users\jiahaof\.pi\agent\install\releases\1.5.2\node_modules\@earendil-works\pi-tui`

| 项 | [实测] |
|---|---|
| 包定位 | `pi-coding-agent` 的第 5 个自研工作区包（另有 `pi-ai` `pi-agent-core` `pi-codemode` `pi-mcp` `pi-schema` `pi-telemetry`） |
| 依赖 | **无** `ink` / `react` / `opentui`（`package.json` grep 零命中） |
| 规模 | dist 顶层 **28 个 `.js` / 11,421 行** + `components/` **18 个 `.js` / 4,840 行** ⇒ ≈16.2k 行 JS |
| native 层 | `native/{darwin,linux,win32}/` 各带 C/ObjC 源码 + napi prebuilds（clipboard 等） |
| 自述 | "Minimal TUI framework with **differential rendering** and **synchronized output**" |
| 它自己列的难点 | `CSI 2026` 原子更新防撕裂 · Bracketed Paste（>10 行粘贴）· application-owned scrolling · Kitty/iTerm2 inline images · **raw mode 下 Ctrl+C 不发 SIGINT，要自己拦截** |
| 抽象形状 | `TUI` **接口 + 两个实现**：`TuiMainScreen`（保留终端 scrollback）/ `TuiAltScreen`（自有视口滚动） |

**§3.6.6 判据的正面印证**：`plan_v0.md` §3.6.6 定「实现 ≥2 个才抽 trait」。pi 抽 `TUI` 接口，是因为它真有**两个**可互换的 renderer；
你现在不抽 `trait Sink`，是因为只有 File 一个。**同一条判据，两边都成立** —— 这也说明那条判据不是抠字眼。

### 2.3 共同规律：三家最大的文件都不是渲染器

| 项目 | 最大的是 |
|---|---|
| opencode | `chat/message.go`（消息渲染）· `dialog/permission.go` · `dialog/filepicker.go` |
| pi | `components/` 18 个（Editor / Markdown / SelectList / SettingsList / ScrollView） |
| Codex | `chatwidget.rs` + `TextArea` 编辑引擎 + `PasteBurst` 粘贴状态机 |

⇒ **渲染循环从来不是最大的那块**：消息列表、编辑器、弹窗才是。这条直接决定 §4 的「够不够」。

---

## 3. 「用 React 做 TUI 到底省了什么」

Claude Code 的文档自己写：终端没有 DOM、没有 CSS 引擎、没有合成器、没有 retained-mode 图形管线；
**layout / styling / diffing / hit-testing / scrolling / selection 全部要从零发明** —— 所以它用的是 **fork 过的 Ink**，
自己写了 reconciler、Yoga 布局、diff、ANSI 输出。

⇒ 结论：**React 给的是心智模型（状态 → 声明 → 重绘），不省实现。**（与本项目 §1「不用 `async-openai`，因为价值在不抽象」同一条判据。）

### 3.1 三套栈的知识是同构的 ⇒ 语言/生态比例是**无关变量**

| 硬知识 | Rust / ratatui | TS / Ink | Go / bubbletea |
|---|---|---|---|
| 状态 → 画面 | `App` struct + `Terminal::draw` | React reconciler | `Update(msg) -> Model` + `View()` |
| 只重绘变化 | `Buffer` diff | 自写 diff（Claude Code） | 字符串 diff（无 cell grid，反而便宜） |
| 布局 | `Layout` 约束 | Yoga flexbox | lipgloss / 手算 |
| **LLM 流式增量塞进渲染循环** | mpsc + `select!` + tick | React state 批处理 | `chan Msg` |
| 取消 / 退出干净 | raw mode 的 RAII + `select!` | `useEffect` cleanup | `tea.Quit` |

**同一个真问题三种栈都必须回答**：async 任务产生的增量怎么喂进单线程渲染循环，且可取消。
这属于「编译器一声不响」型（丢帧、撕裂、退出后终端留乱码，全编得过）⇒ 按 `plan.md` §2.1 分界线，**属自己写那一列**。

⇒ 「TS 是前端王者」对本题是无关变量：前端强在**组件生态与人力存量**，不强在终端的物理约束；而这里要学的恰好是后者。
（若目标是复用前端技能/找活，那是 `plan.md` §七 的另一条轴，需单独按「≤3 周 + 回流主线吗」记账。）

---

## 4. ratatui 给你 / 不给你

**给**：`Buffer` cell 网格 · **diff + flush** · `Layout` 约束 · `Widget` trait · **`TestBackend`（离线渲染到 Buffer，可断言）**。
最后一条最关键：它决定 **TUI 层能不能被测试覆盖**（`TestBackend` 是唯一的离线断言手段）。

**不给**（你必须自己写）：

| # | 欠的东西 | 代价 | 备注 |
|---|---|---|---|
| 1 | 事件循环（`event::poll/read` + tick） | 200–300 行的主体 | — |
| 2 | **raw mode 的 RAII**（panic 时也要 restore） | 小，漏了很脏 | 崩溃后终端留乱码 |
| 3 | **async 增量 → 渲染循环**（mpsc + `select!`） | 中 | 与 V1 阶段 3 是**同一件事** |
| 4 | 滚动/历史语义（`Scrollbar` 是控件，状态是你的） | 小–中 | — |
| 5 | 多行编辑器（光标/IME/粘贴/undo） | **大** | 生态要借 `tui-textarea`/`edtui` |
| 6 | **Markdown 渲染**（模型输出就是 markdown，躲不掉） | **大** | Rust 侧只有社区 crate，成熟度不及 Ink / Go `glamour` [推断，未核版本] |
| 7 | 同步输出防撕裂（pi 用 `CSI 2026`） | 小 | `Terminal::draw` 整帧 diff 通常够；高频流式重绘才撞 |
| 8 | inline image | 可选 | 不做 |

### 4.1 本项目真实需求只用到 3 件

真实需求 = **REPL 回路 + 审批 `Y/n` + 模型流式增量上屏 + Ctrl-C 干净退出**（不是「做一个 TUI」）。

⇒ 只需 **`Buffer`/`Terminal::draw` + `Layout` + `TestBackend`** + crossterm `event::poll/read`。
⇒ **不需要**编辑器、Markdown、主题、弹窗。合计 **300–500 行**，两个依赖本机均已缓存。
⇒ 「不够」的分界线：想要**权限弹窗 + 文件选择器 + 可滚动历史 + 多行输入**（opencode `dialog/` 那套）时 —— 那是 7–16k 行的来历。
⇒ 第一版建议：**纯文本 + 少量 ANSI 着色**，不追 markdown 效果（第 6 格是唯一值得提前知道的债）。

---

## 5. 为什么它算「回流主线」而不是岔路（三条缝合点）

> **2026-11-11 校正**：把 §8.1 的判据老实应用到三条签名上，结论是 **TUI 只逼改其中一条**。
> 下表第 1 条成立、**第 2 条不成立**（错误已于本次改正）、第 3 条成立但方案 D 就能满足。

| # | 缝合点 | 现文档里的话 | TUI 会不会逼它改 |
|---|---|---|---|
| 1 | **谁拥有 stdout** | `plan_v0.md` §3.6.6：① TUI 要独占 stdout，任何零散 `println!` 会撕裂渲染（**列为第一条理由**）；「抽 `trait Sink`」的触发条件是「出现**第二个 sink**」 | **会** ✔ 这是 TUI 唯一独有的贡献 |
| 2 | **`Executor::run` 的返回形态** | `plan_v1.md` §10 待答 #5「`anyhow::Result<String>` 在流式 + 取消后还成不成立」；`plan.md` §5.1「事后 `truncate_head` 失效」 | **不会** ✘ —— TUI 完全可以消费**成品字符串**（Codex / opencode 都把“完整事件”画上去；工具结果本来就是“完整对象”才回给模型）。原文「TUI 不答就写不下去」是**错的**：逼它改的是**阶段 3**（流式 + 取消），与 TUI 无关 |
| 3 | **交互回路** | `plan_v1.md` §9：审批钩子挂起的三前提 = REPL 循环 + `stdin.is_terminal()` + 无人值守默认 + 不信任上下文；`plan.md` V2「唯一真新的是有状态会话 + 双向往返」 | 会，但 **方案 D（纯 REPL，半天）就够了** |

⚠️ 第 3 条要收窄：**回路 ≠ 图形**（§0 末）。审批与语义追问用 `read_line` 就够，TUI 只解决「好看 + 历史可滚动」。

### 5.1 真正被逼改的是**第三个**签名（实测）

TUI 的着力点不在 §8.1 点名的两个签名上，而在**入参形态与生命周期**：

```rust
main.rs:23    logger_config: &LogSink<'_>                          // 传的是“规格”
log.rs:30     pub fn get_logger(sink: &LogSink) -> Result<Self>    // 用完即丢，Logger 不存它
agent.rs:37   let mut logger = Logger::get_logger(sink)?;          // Logger 活在单次 run 内
```

⇒ 渲染循环要**跨 run 存活**，`&mut Logger` 无法共享 ⇒ TUI 要的是 `Box<dyn Sink + Send>` 或 **channel 事件流**。
这是**所有权 / 生命周期**改动，不是“往 `emit` 里加个分支”——`emit` 本身大概率一字不改。

⇒ 所以 §3.6.6「出现第二个 sink 才抽 trait」的触发条件**并没有被满足**（防循环论证条款见 §8.1）。

---

## 6. 顺序：为什么不是现在

1. **会写两遍输出层**。`plan_v1.md` §3/阶段 3 明确要动 `Executor::run` 的返回/打印路径。TUI 先行 ⇒ 先写一套事件投递，
   流式落地再写一套「边收边打」。反过来（阶段 3 之后）只做一次：那时**已经**必须把输出从 `String` 拆成事件流，TUI 只是第二个消费者。
2. ~~**它会顺手遮住未还的债**。V0.5 的 **B2（日志无时间戳，append 模式多次运行首尾相接）尚未做**~~
   → **该条已失效（2026-11-11 校正）**：B2（`v0.5-breakpoints.md` #3，`plan.md:306` 的原编号）**已做完**。
   `log.rs:86` 有 `[+{:>8.3}s]` 耗时前缀（commit `2fcc7fd`），`task()` 另写 `##### START TASK, TIMESTAMP : … #####` 分隔 run。
   替换论点（同一条判据的另一个实例，仍是“先别做 TUI”）：**#6 的取证契约是给文本终端写的**——
   “`(见log文件)`” + “完整输出在文件：`tmp/agent_output/<hash>.log`” 这两条线索住在**滚动的 stdout** 里；
   TUI 独占屏幕后，这两句要么被裁掉，要么得重新设计一个“打开落盘文件”的入口 ⇒ **它是真缝合点，也是真工作量**。
3. **`plan.md` §七 的规矩要遵守**：同一时间只允许一条轴主动推进；岔路 ≤3 周且必须回答「回流主线了吗」。
   TUI（UI 状态管理）与「表达力/正确性」**不是同一条轴** ⇒ 若要做，得单独明写一次记账，不能算「顺便」。

---

## 7. 候选方案与代价

| 方案 | 形态 | 代价 / 风险 |
|---|---|---|
| **A · crossterm + ratatui-core**（最薄形态，与 §8 判据 2 相容） | 阶段 3 之后：`Event` 流 → TUI sink + 一行输入 + Ctrl-C | 依赖只加 `ratatui`+`crossterm`（已缓存）；300–500 行；学到的是约束；可测（`TestBackend`） |
| **B · ratatui-kit 全量移植** | 组件化重构 UI 层，router/atom/hooks | 20,271 行框架 API + feature 需联网 ≥4 包 + Windows 终端差异未测；3 周岔路极易超；学到的是 DSL |
| **C · 只抽 sink 不做 UI** | 把 `Logger` 屏幕侧改成 `Vec<Event>`，仍打 stdout | 严格说是阶段 3 的一部分，不算岔路；成本几小时；**顺序上最省** |
| **D · 纯 REPL 不做 TUI** | `println!` + `read_line` + 现有 `Logger` | 半天；解锁审批钩子/语义追问；不含渲染、不含流式上屏 |

**判据**：先 D（拿回路）还是先 C（拿事件化）→ 都可以在阶段 3 附近做；**A 必须等阶段 3 之后**；**B 需要单独记账**。

---

## 8. 可执行判据（不预设答案）

1. **「回流主线」的硬判据**：移植/接入后，**这三条签名**里有没有被逼改的？

   | 签名 | 谁逼它改 |
   |---|---|
   | `Executor::run -> String` | **只有**阶段 3（流式 + 取消）—— TUI **不逼** |
   | `Logger::emit` | 未必；TUI 换的是它**背后**的 sink |
   | `run_agent(&LogSink)` / `Logger::get_logger` | **TUI 会**（规格 → 活对象/通道 + 跨 run 生命周期） |

   三条全不改 ⇒ 只换皮，按 `plan.md` §2.1 属「装饰」该砍。
   **防循环论证条款**：不许拿“想象中的第二个实现会需要什么”当触发条件 —— 判据原文是「**实现** ≥2 个才抽 trait」。
   要验就写个 **20–30 行的假 sink**（`Vec<String>` + 结束 dump）替掉 `Console`，看 `run_agent` 签名被迫变成什么。
   那是“实现第二个”，**成本 30 分钟**，不是半天。
2. **时间盒**：设 **3 天**为第一 checkpoint。若在试 B（或任何框架形态），做不到「循环跑在 TUI 里 + 日志走 sink + Ctrl-C 干净退出」⇒
   瓶颈在**框架 API** 而非 TUI 知识 ⇒ 退到方案 A。
3. **先问目的**（三选一，选型完全不同）：① 学终端渲染/事件循环/与 async 对齐 → **A**；② 学声明式组件组织界面 → B（但要记账）；
   ③ 让 agent 看起来像个产品 → 现在都不做。
4. **测试账**：若 UI 层不可断言（`TestBackend` 用不上），它就只剩“人眼看”⇒ 视为硬伤。
5. **Windows 终端差异**（纯实测项）：crossterm 在 Windows Terminal 与老 console/ConEmu 下行为不同，而本仓库路径层已在跟
   `VerbatimDisk` / junction 打架。动手第一步先跑 `cargo run --example todo_app`（ratatui-kit）或自写最小 alternate-screen 探针，
   确认 alternate screen + raw mode + 中文宽度正常。

---

## 9. 待答问题（open）

1. **要不要 TUI 这件事，判据 1 能不能先验？** ~~成本 ≈ 半天~~ → **已答（2026-11-11）**：不用 experiment，答案已知（§5 校正）。
   TUI **不会**逼改 `Executor::run`；被逼改的是 `run_agent` / `get_logger` 的入参形态与生命周期。
   想真验就写 §8.1 那 30 行假 sink。
2. **Markdown 的最小方案**：`tui-markdown` 之类社区 crate 的成熟度/版本 + 是否先退化为纯文本。
   **触发**：方案 A 动手当天；默认答案是**先纯文本**，不预研（第 6 格债务已知，届时再还）。
3. **~~`truncate_head` 在 TUI 下的替代~~**：原提法「TUI 是事件流 ⇒ 截断决策挪到消费端」**混了两条流**，已修正：

   | 流 | 形态 | 截断/策略在哪一端 |
   |---|---|---|
   | (a) LLM token 流 | 边到边消费 | 才可能挪到消费端（阶段 3 的事） |
   | (b) 工具输出 | 天生“完整对象”后才回给模型（`run -> String`） | **恒在生产端**，流式不碰 |

   ⇒ **显示策略已经在消费端**：`log.rs:132` `record()` 按 `LogKind` 分策略（`ToolResult` 只打字节数）。事件化时它自然迁移，不是新问题。
   ⇒ **数据截断恒在生产端**：#6 定稿的「`shown` 8000B 有界」是 (b) 的决策，**阶段 3 不需要重做**。
4. **测试形态**：`TestBackend` 断言到「屏幕缓冲区某行某列」这个粒度，会不会让 UI 测试变成脆测试（改布局就红）？
   判据：只断言**语义行**（如「最后一行是 prompt」），不断言像素。
   **触发**：写第一个 UI 测试时；在那之前，`TestBackend` 能不能用要先由 §11 的探针回答（门）。

---

## 10. 明确不做

- **把项目「移植」到 ratatui-kit**：与 `plan.md` §1「不抽象循环」冲突；除非目的变成「读一个 2 万行 TUI 框架怎么组织」——那是**读源码**练习，不是**引依赖**。
- **主题系统 / 鼠标悬停 / inline image / 动画**：`plan.md` §2.1「现代 agent 功能表是约束的产物」。
- **在阶段 3 之前写渲染代码**：见 §6 第 1 条。
- **把 REPL 需求包装成 TUI 需求**：见 §0 末。

---

## 11. 附录：复核命令（本文所有 [实测] 都可重跑）

```bash
# 1) 四层版本与缓存状态
ls "D:/Rust/.cargo/registry/src/rsproxy.cn-e3de039b2554c837/" | grep -iE "^(ratatui|crossterm|tui-input|tui-tree-widget|futures-)"
# 2) ratatui 0.30 的门面形状 + 拆包依赖
grep -nE "^\[dependencies\.|^version = " "…/ratatui-0.30.2/Cargo.toml" | head -30
ls "…/ratatui-0.30.2/src"
# 3) 规模
cd "…/ratatui-widgets-0.3.2/src" && find . -name '*.rs' -exec cat {} + | wc -l   # 29,723
cd "…/ratatui-kit-0.12.0/src"   && find . -name '*.rs' | wc -l                   # 84
# 4) pi 的 TUI（本机源码）
cd "C:/Users/jiahaof/.pi/agent/install/releases/1.5.2/node_modules/@earendil-works/pi-tui/dist" && ls *.js | wc -l && cat *.js | wc -l
cd ../components && ls *.js | wc -l && cat *.js | wc -l
grep -oE '"(ink|react|opentui)"' ../package.json    # 零命中 = 自研
# 5) opencode TUI 规模（tree API 数文件与字节；行数是推断）
curl -s "https://api.github.com/repos/opencode-ai/opencode/git/trees/main?recursive=1" -o /tmp/oc.json
jq -r '[.tree[]|select(.path|startswith("internal/tui/"))|select(.path|endswith(".go"))]|"files: \(length)  bytes: \(map(.size)|add)"' /tmp/oc.json
# 6) Claude Code / Gemini CLI 的 Ink 证据：grep 各自仓库 src/ink/、packages/cli/src/interactiveCli.tsx
```

---

## 12. 版本记录

| 日期 | 变更 |
|---|---|
| 2026-11-11 | 建立。来源：一次关于「能不能顺便学 TUI / 是否移植到 ratatui-kit」的讨论；四家 TUI 栈实测、两层纠正（ratatui 0.30 已拆包；opencode 的 1,333 行是单文件）、三条缝合点、四条判据 |
| 2026-11-11 | 校正两处事实：①§6-2 的 B2“尚未做”→**已做完**（另换论据）；②`ratatui-widgets` 是**非 optional** 依赖，“可整层不用”要改成“不调用但照样编” |
| 2026-11-11 | 应用 §8.1 判据后改结论：§5 第 2 条“TUI 逼改 `Executor::run`”**不成立**（改由阶段 3 逼）；新增 §5.1（真正着力点是 `run_agent`/`get_logger` 的入参 + 生命周期，不是 `emit`）；§8.1 拆三条签名 + 新增**防循环论证条款**；§9 #1/#3 已答/已修正 |
