use std::path::Path;

use anyhow::bail;

use crate::{
    llm::LlmClient,
    log::{LogKind, LogSink, Logger},
    model::{Invocation, Message, RunReport, ToolKind},
    tool::Executor,
};

const SYSTEM_PROMPT: &str = "你是一个有用的AI助手,遵循以下指令:\n\
    1.**环境** : 你在一个沙箱目录内工作,所有路径相对该目录,'.'是根.\n\
    2.**边界** : 只能读写沙箱内文件；越界会报错，**不要重试**.\n\
    3.**行为** : 直接完成任务，不要反复确认、不要问'需要我读吗'\n\
    4.**输出** : 最终答案直接给，不要复述工具原始输出";

pub async fn run_agent<C: LlmClient>(
    client: &mut C,
    task: &str,
    max_steps: usize,
    sink: &LogSink<'_>,
    sandbox_root: &Path,
) -> anyhow::Result<RunReport> {
    let system_prompt = Message::System {
        content: SYSTEM_PROMPT.to_string(),
    };
    let user_task = Message::User {
        content: task.to_string(),
    };
    let mut messages = vec![system_prompt, user_task];
    let tools = ToolKind::tools();
    let tool_exec = Executor::new(sandbox_root)?;

    let mut finished = false;

    let mut logger = Logger::get_logger(sink)?;

    logger.task(task);

    let mut run_report = RunReport::default();

    for step in 1..=max_steps {
        logger.step(step);
        run_report.steps = step;

        let mut resp = client.chat(&messages, &tools).await?;
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

        if tool_calls.is_empty() {
            if resp_content.finish_reason.as_deref() == Some("length") {
                logger.record(LogKind::Error, "输出被长度限制截断");
            }
            finished = true;
            run_report.finish_reason = resp_content.finish_reason.take();
            break;
        }

        let mut tool_messages = vec![];

        for tool_call in tool_calls {
            let tool_id = &tool_call.id;
            run_report.tool_calls_executed += 1;
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

                            let (log_kind, tool_res) = match tool_exec.run(&inv) {
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

        messages.push(resp.choices.swap_remove(0).message);
        messages.append(&mut tool_messages);
    }

    run_report.finished = finished;

    logger.record(LogKind::TaskResult, &run_report.total_info());

    Ok(run_report)
}

impl RunReport {
    fn total_info(&self) -> String {
        let task_res = if self.finished {
            format!("任务完成！步数 {} 步。", self.steps)
        } else {
            format!("未能在 {} 步内完成任务！", self.steps)
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
}
