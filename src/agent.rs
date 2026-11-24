use std::path::Path;

use anyhow::bail;

use crate::{
    llm::LlmClient,
    log::{LogKind, LogSink, Logger},
    model::{Invocation, Message, RunReport, ToolKind, WireTool},
    tool::Executor,
};

const SYSTEM_PROMPT: &str = "你是一个有用的AI助手,遵循以下指令:\n\
    1.**环境** : 你在一个沙箱目录内工作,所有路径相对该目录,'.'是根.\n\
    2.**边界** : 只能读写沙箱内文件；越界会报错，**不要重试**.\n\
    3.**行为** : 直接完成任务，不要反复确认、不要问'需要我读吗'\n\
    4.**输出** : 最终答案直接给，不要复述工具原始输出";

pub struct Agent<'a, C: LlmClient> {
    client: &'a mut C,
    messages: Vec<Message>,
    tools: Vec<WireTool>,
    tool_exec: Executor,
}

impl<'a, C: LlmClient> Agent<'a, C> {
    pub fn new(client: &'a mut C, root: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            client,
            messages: vec![Message::System {
                content: SYSTEM_PROMPT.into(),
            }],
            tools: ToolKind::tools(),
            tool_exec: Executor::new(root)?,
        })
    }

    pub async fn run(
        &mut self,
        task: &str,
        max_steps: usize,
        sink: &LogSink<'_>,
    ) -> anyhow::Result<RunReport> {
        let user_task = Message::User {
            content: task.to_string(),
        };
        self.messages.push(user_task);

        let mut finished = false;
        let mut logger = Logger::get_logger(sink)?;
        logger.task(task);

        let mut run_report = RunReport::default();

        for step in 1..=max_steps {
            logger.step(step);
            run_report.steps = step;

            let mut resp = self.client.chat(&self.messages, &self.tools).await?;
            if resp.choices.is_empty() {
                bail!("返回结果信息为空!");
            }

            run_report.usage.push(resp.usage.take());

            let resp_content = &mut resp.choices[0];

            let Message::Assistant {
                content,
                reasoning_content,
                tool_calls,
            } = &resp_content.message
            else {
                bail!("返回非法的 assistant message! ");
            };

            if let Some(reasoning) = reasoning_content {
                logger.record(LogKind::Thought, reasoning);
            }

            if let Some(ai_content) = content {
                logger.record(LogKind::Answer, ai_content);
            }

            let is_truncated = resp_content.finish_reason.as_deref() == Some("length");

            if tool_calls.is_empty() {
                if is_truncated {
                    logger.record(LogKind::Error, "输出被长度限制截断");
                }
                finished = true;
                run_report.finish_reason = resp_content.finish_reason.take();
                self.messages.push(resp.choices.swap_remove(0).message);
                break;
            }

            let mut tool_messages = vec![];

            for tool_call in tool_calls {
                let tool_id = &tool_call.id;
                run_report.tool_calls_requested += 1;
                if is_truncated {
                    logger.record(LogKind::Error, "截断：跳过工具执行");
                    tool_messages.push(Message::Tool {
                        tool_call_id: tool_id.clone(),
                        content: "Error: 输出被长度限制截断，调用未执行，请用完整参数重新发起"
                            .into(),
                    });
                    continue;
                }
                match tool_call.payload() {
                    Ok(func_call) => {
                        let Some(tool_kind) = ToolKind::from_name(&func_call.name) else {
                            tool_messages.push(Message::Tool {
                                tool_call_id: tool_id.clone(),
                                content: "Error: 未找到符合的工具".to_string(),
                            });
                            logger.record(LogKind::Error, "未找到符合的工具");
                            continue;
                        };

                        match Invocation::parse_kind(&tool_kind, &func_call.arguments) {
                            Ok(inv) => {
                                logger.record(LogKind::ToolCall, &inv.info());

                                run_report.tool_calls_executed += 1;

                                let (log_kind, tool_res) = match self.tool_exec.run(&inv) {
                                    Ok(out) => (LogKind::ToolResult, out),
                                    Err(err) => (LogKind::Error, format!("Error: {err:#}")),
                                };

                                logger.record(log_kind, &tool_res);

                                tool_messages.push(Message::Tool {
                                    tool_call_id: tool_id.clone(),
                                    content: tool_res,
                                });
                            }

                            Err(err) => {
                                tool_messages.push(Message::Tool {
                                    tool_call_id: tool_id.clone(),
                                    content: format!("Error: {err:#}"),
                                });
                                logger.record(LogKind::Error, &format!("{err:#}"));
                            }
                        }
                    }
                    Err(err) => {
                        tool_messages.push(Message::Tool {
                            tool_call_id: tool_id.clone(),
                            content: format!("Error: {err:#}"),
                        });
                        logger.record(LogKind::Error, &format!("{err:#}"));
                    }
                }
            }

            run_report.finish_reason = resp_content.finish_reason.take();

            self.messages.push(resp.choices.swap_remove(0).message);
            self.messages.append(&mut tool_messages);
        }

        run_report.finished = finished;
        logger.record(LogKind::TaskResult, &run_report.total_info());
        Ok(run_report)
    }
}

pub async fn run_agent<C: LlmClient>(
    client: &mut C,
    task: &str,
    max_steps: usize,
    sink: &LogSink<'_>,
    sandbox_root: &Path,
) -> anyhow::Result<RunReport> {
    Agent::new(client, sandbox_root)?
        .run(task, max_steps, sink)
        .await
}

impl RunReport {
    fn total_info(&self) -> String {
        let is_truncated = self.finish_reason.as_deref() == Some("length");
        let task_res = if self.finished && !is_truncated {
            format!("输出结束。步数 {} 步。", self.steps)
        } else if is_truncated {
            format!("输出结束，最终结果生成被截断。步数 {} 步。", self.steps)
        } else {
            format!("未能在 {} 步内结束输出！", self.steps)
        };

        let tool_calls = if self.tool_calls_executed == self.tool_calls_requested {
            format!("工具调用 {} 次", self.tool_calls_executed)
        } else {
            format!(
                "工具调用 {} 次（请求 {} 次）",
                self.tool_calls_executed, self.tool_calls_requested
            )
        };

        let reported = self.usage.iter().flatten().count();
        if reported == 0 {
            return format!("{task_res} {tool_calls}。 usage 未上报！");
        }

        let (prompt, completion, cached, reasoning) =
            self.usage.iter().fold((0, 0, 0, 0), |acc, x| match x {
                None => acc,
                Some(usage) => (
                    acc.0 + usage.prompt_tokens,
                    acc.1 + usage.completion_tokens,
                    acc.2
                        + usage
                            .prompt_tokens_details
                            .as_ref()
                            .map_or(0, |y| y.cached_tokens),
                    acc.3
                        + usage
                            .completion_tokens_details
                            .as_ref()
                            .map_or(0, |y| y.reasoning_tokens),
                ),
            });

        format!(
            "{task_res} {tool_calls}，总消耗 {} tok, prompt {} tok (cached {}), completion {} tok (reasoning {}).",
            prompt + completion,
            prompt,
            cached,
            completion,
            reasoning
        )
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::model::{ChatResponse, Choice, FunctionCall};
    use std::{collections::VecDeque, fs, path::PathBuf};

    struct Scripted {
        replies: VecDeque<ChatResponse>,
        seen: Vec<Vec<Message>>,
    }

    impl LlmClient for Scripted {
        async fn chat(
            &mut self,
            messages: &[Message],
            _: &[crate::model::WireTool],
        ) -> anyhow::Result<ChatResponse> {
            self.seen.push(messages.to_vec());
            self.replies
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("脚本耗尽于第 {} 次调用", self.seen.len()))
        }
    }

    impl Scripted {
        fn new(replies: VecDeque<ChatResponse>) -> Self {
            Self {
                replies,
                seen: vec![],
            }
        }
    }

    fn resp(finish: &str, message: Message) -> ChatResponse {
        ChatResponse {
            id: "t".into(),
            object: "chat.completion".into(),
            model: "m".into(),
            choices: vec![Choice {
                index: 0,
                finish_reason: Some(finish.into()),
                message,
            }],
            usage: None,
        }
    }

    fn say(text: &str) -> ChatResponse {
        resp(
            "stop",
            Message::Assistant {
                content: Some(text.into()),
                reasoning_content: None,
                tool_calls: vec![],
            },
        )
    }

    fn call(id: &str, name: &str, args: &str) -> ChatResponse {
        resp(
            "tool_calls",
            Message::Assistant {
                content: None,
                reasoning_content: None,
                tool_calls: vec![crate::model::ToolCall {
                    id: id.into(),
                    kind: "function".into(),
                    function: Some(FunctionCall {
                        name: name.into(),
                        arguments: args.into(),
                    }),
                    custom: None,
                }],
            },
        )
    }

    fn calls(tool_calls: &[(&str, &str, &str)]) -> ChatResponse {
        resp(
            "tool_calls",
            Message::Assistant {
                content: None,
                reasoning_content: None,
                tool_calls: tool_calls
                    .iter()
                    .map(|tool_call| crate::model::ToolCall {
                        id: tool_call.0.into(),
                        kind: "function".into(),
                        function: Some(FunctionCall {
                            name: tool_call.1.into(),
                            arguments: tool_call.2.into(),
                        }),
                        custom: None,
                    })
                    .collect(),
            },
        )
    }

    fn bad_call(id: &str) -> ChatResponse {
        resp(
            "tool_calls",
            Message::Assistant {
                content: None,
                reasoning_content: None,
                tool_calls: vec![crate::model::ToolCall {
                    id: id.into(),
                    kind: "function".into(),
                    function: None,
                    custom: None,
                }],
            },
        )
    }

    fn tmp_root(root: &str) -> PathBuf {
        let p = PathBuf::from("tmp").join(root);
        clear_tmp(&p);
        fs::create_dir_all(&p).expect("创建测试目录失败");
        p
    }

    fn clear_tmp(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn test_loop_answer_only() {
        let root = tmp_root("test_loop_answer_only");

        let mut client = Scripted::new(VecDeque::from([say("done")]));
        let report = run_agent(&mut client, "task", 5, &LogSink::Console, &root)
            .await
            .unwrap();

        assert_eq!(report.steps, 1);
        assert!(report.finished);
        assert_eq!(report.finish_reason.as_deref(), Some("stop"));
        assert_eq!(report.tool_calls_requested, 0);
        assert_eq!(report.tool_calls_executed, 0);
        assert_eq!(client.seen.len(), 1);
        assert_eq!(client.seen[0].len(), 2);

        clear_tmp(&root);
    }

    #[tokio::test]
    async fn test_loop_tool_unknown() {
        let root = tmp_root("test_loop_tool_unknown");

        let mut client = Scripted::new(VecDeque::from([
            call("1", "read", r#"{"path":"a.txt", "offset" : 0}"#),
            say("done"),
        ]));
        let report = run_agent(&mut client, "task", 5, &LogSink::Console, &root)
            .await
            .unwrap();

        assert_eq!(report.steps, 2);
        assert!(report.finished);
        assert_eq!(report.finish_reason.as_deref(), Some("stop"));
        assert_eq!(report.tool_calls_requested, 1);
        assert_eq!(report.tool_calls_executed, 0);
        assert_eq!(client.seen.len(), 2);
        assert_eq!(client.seen[1].len(), 4);
        match &client.seen[1][3] {
            Message::Tool {
                tool_call_id,
                content,
            } => {
                assert_eq!(tool_call_id, "1");
                assert!(content.starts_with("Error"));
                assert!(content.contains("未找到符合的工具"))
            }
            other => panic!("末条应该是 Tool 消息，实际上是 {other:?}"),
        }

        clear_tmp(&root);
    }

    #[tokio::test]
    async fn test_loop_tool_arg_err() {
        let root = tmp_root("test_loop_tool_arg_err");

        let mut client = Scripted::new(VecDeque::from([
            call("1", "read_file", "a.txt"),
            say("done"),
        ]));
        let report = run_agent(&mut client, "task", 5, &LogSink::Console, &root)
            .await
            .unwrap();

        assert_eq!(report.steps, 2);
        assert!(report.finished);
        assert_eq!(report.finish_reason.as_deref(), Some("stop"));
        assert_eq!(report.tool_calls_requested, 1);
        assert_eq!(report.tool_calls_executed, 0);
        assert_eq!(client.seen.len(), 2);
        assert_eq!(client.seen[1].len(), 4);
        match &client.seen[1][3] {
            Message::Tool {
                tool_call_id,
                content,
            } => {
                assert_eq!(tool_call_id, "1");
                assert!(content.starts_with("Error"));
            }
            other => panic!("末条应该是 Tool 消息，实际上是 {other:?}"),
        }

        clear_tmp(&root);
    }

    #[tokio::test]
    async fn test_loop_tool_run_err() {
        let root = tmp_root("test_loop_tool_run_err");

        let mut client = Scripted::new(VecDeque::from([
            call("1", "read_file", r#"{"path" : "a.txt", "offset" : 0}"#),
            say("done"),
        ]));
        let report = run_agent(&mut client, "task", 5, &LogSink::Console, &root)
            .await
            .unwrap();

        assert_eq!(report.steps, 2);
        assert!(report.finished);
        assert_eq!(report.finish_reason.as_deref(), Some("stop"));
        assert_eq!(report.tool_calls_requested, 1);
        assert_eq!(report.tool_calls_executed, 1);
        assert_eq!(client.seen.len(), 2);
        assert_eq!(client.seen[1].len(), 4);
        match &client.seen[1][3] {
            Message::Tool {
                tool_call_id,
                content,
            } => {
                assert_eq!(tool_call_id, "1");
                assert!(content.starts_with("Error"));
            }
            other => panic!("末条应该是 Tool 消息，实际上是 {other:?}"),
        }

        clear_tmp(&root);
    }

    #[tokio::test]
    async fn test_loop_unsupported_call() {
        let root = tmp_root("test_loop_unsupported_call");

        let mut client = Scripted::new(VecDeque::from([bad_call("1"), say("done")]));
        let report = run_agent(&mut client, "task", 5, &LogSink::Console, &root)
            .await
            .unwrap();

        assert_eq!(report.steps, 2);
        assert!(report.finished);
        assert_eq!(report.finish_reason.as_deref(), Some("stop"));
        assert_eq!(report.tool_calls_requested, 1);
        assert_eq!(report.tool_calls_executed, 0);
        assert_eq!(client.seen.len(), 2);
        assert_eq!(client.seen[1].len(), 4);
        match &client.seen[1][3] {
            Message::Tool {
                tool_call_id,
                content,
            } => {
                assert_eq!(tool_call_id, "1");
                assert!(content.starts_with("Error"));
            }
            other => panic!("末条应该是 Tool 消息，实际上是 {other:?}"),
        }

        clear_tmp(&root);
    }

    #[tokio::test]
    async fn test_loop_call_success() {
        let root = tmp_root("test_loop_call_success");

        let mut client = Scripted::new(VecDeque::from([
            call("1", "list_files", r#"{"path" : "."}"#),
            say("done"),
        ]));
        let report = run_agent(&mut client, "task", 5, &LogSink::Console, &root)
            .await
            .unwrap();

        assert_eq!(report.steps, 2);
        assert!(report.finished);
        assert_eq!(report.finish_reason.as_deref(), Some("stop"));
        assert_eq!(report.tool_calls_requested, 1);
        assert_eq!(report.tool_calls_executed, 1);
        assert_eq!(client.seen.len(), 2);
        assert_eq!(client.seen[1].len(), 4);
        match &client.seen[1][3] {
            Message::Tool {
                tool_call_id,
                content,
            } => {
                assert_eq!(tool_call_id, "1");
                assert!(!content.starts_with("Error"));
            }
            other => panic!("末条应该是 Tool 消息，实际上是 {other:?}"),
        }

        clear_tmp(&root);
    }

    #[tokio::test]
    async fn test_loop_multi_calls() {
        let root = tmp_root("test_loop_multi_calls");

        let mut client = Scripted::new(VecDeque::from([
            calls(&[
                ("1", "list_files", r#"{"path" : "."}"#),
                ("2", "read", r#"{"path" : "a.txt", "offset" : 0 }"#),
            ]),
            say("done"),
        ]));
        let report = run_agent(&mut client, "task", 5, &LogSink::Console, &root)
            .await
            .unwrap();

        assert_eq!(report.steps, 2);
        assert!(report.finished);
        assert_eq!(report.finish_reason.as_deref(), Some("stop"));
        assert_eq!(report.tool_calls_requested, 2);
        assert_eq!(report.tool_calls_executed, 1);
        assert_eq!(client.seen.len(), 2);
        assert_eq!(client.seen[1].len(), 5);
        match &client.seen[1][3] {
            Message::Tool {
                tool_call_id,
                content,
            } => {
                assert_eq!(tool_call_id, "1");
                assert!(!content.starts_with("Error"));
            }
            other => panic!("倒数第二条应该是 Tool 消息，实际上是 {other:?}"),
        }

        match &client.seen[1][4] {
            Message::Tool {
                tool_call_id,
                content,
            } => {
                assert_eq!(tool_call_id, "2");
                assert!(content.starts_with("Error"));
            }
            other => panic!("末条应该是 Tool 消息，实际上是 {other:?}"),
        }

        clear_tmp(&root);
    }

    #[tokio::test]
    async fn test_loop_step_over() {
        let root = tmp_root("test_loop_step_over");

        let mut client = Scripted::new(VecDeque::from([
            call("1", "list_files", r#"{"path" : "."}"#),
            call("2", "list_files", r#"{"path" : "."}"#),
        ]));
        let report = run_agent(&mut client, "task", 2, &LogSink::Console, &root)
            .await
            .unwrap();

        assert_eq!(report.steps, 2);
        assert!(!report.finished);
        assert_eq!(report.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(report.tool_calls_requested, 2);
        assert_eq!(report.tool_calls_executed, 2);
        assert_eq!(client.seen.len(), 2);

        clear_tmp(&root);
    }
}
