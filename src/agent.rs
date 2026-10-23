use anyhow::bail;

use crate::{
    llm::OpencodeGoClient,
    model::{Invocation, Message, ToolKind},
    tool::Executor,
};

const SYSTEM_PROMPT: &str = "你是一个有用的AI助手,遵循以下指令:\n1.**环境** : 你在一个沙箱目录内工作,所有路径相对该目录,'.'是根.\n2.**边界** : 只能读写沙箱内文件；越界会报错，**不要重试**.\n3.**行为** : 直接完成任务，不要反复确认、不要问'需要我读吗'\n4.**输出** : 最终答案直接给，不要复述工具原始输出";

pub async fn run_agent(
    client: &OpencodeGoClient,
    task: &str,
    max_steps: usize,
) -> anyhow::Result<()> {
    let system_prompt = Message::System {
        content: SYSTEM_PROMPT.to_string(),
    };
    let user_task = Message::User {
        content: task.to_string(),
    };
    let mut messages = vec![system_prompt, user_task];
    let tools = ToolKind::tools();
    let tool_exec = Executor::new("scratch")?;

    let mut finished = false;

    for step in 1..=max_steps {
        println!("----------");
        println!("开始第 {} 步", step);

        let mut resp = client.chat(&messages, &tools).await?;
        if resp.choices.is_empty() {
            bail!("返回结果信息为空!");
        }

        let resp_content = &resp.choices[0];

        let Message::Assistant {
            content,
            reasoning_content,
            tool_calls,
        } = &resp_content.message
        else {
            bail!("返回非法的 assistant message! ");
        };

        if let Some(reasoning) = reasoning_content {
            println!("AI 思考 : {}", reasoning);
        }

        if let Some(ai_content) = content {
            println!("AI 回答 : {}", ai_content);
        }

        if tool_calls.is_empty() {
            if resp_content.finish_reason.as_deref() == Some("length") {
                println!("输出被长度限制截断");
            }
            finished = true;
            break;
        }

        let mut tool_messages = vec![];

        for tool_call in tool_calls {
            let tool_id = &tool_call.id;
            match tool_call.payload() {
                Ok(func_call) => {
                    let Some(tool_kind) = ToolKind::from_name(&func_call.name) else {
                        tool_messages.push(Message::Tool {
                            tool_call_id: tool_id.clone(),
                            content: "Error: 未找到符合的工具".to_string(),
                        });
                        continue;
                    };

                    match Invocation::parse_kind(&tool_kind, &func_call.arguments) {
                        Ok(inv) => {
                            let tool_res = match tool_exec.run(&inv) {
                                Ok(out) => out,
                                Err(err) => format!("Error: {err:#}"),
                            };

                            println!("Tool Result :\n{}", tool_res);

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
                        }
                    }
                }
                Err(err) => {
                    tool_messages.push(Message::Tool {
                        tool_call_id: tool_id.clone(),
                        content: format!("Error: {err:#}"),
                    });
                }
            }
        }

        messages.push(resp.choices.swap_remove(0).message);
        messages.append(&mut tool_messages);
    }

    if !finished {
        println!("未能在{max_steps}步内完成任务!");
    } else {
        println!("任务完成!");
    }

    Ok(())
}
