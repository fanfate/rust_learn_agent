use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    Thought,
    Answer,
    ToolCall,
    ToolResult,
    Error,
    TaskResult,
}

pub enum LogSink<'a> {
    File(&'a str),
    Console,
}

pub struct Logger {
    file: Option<File>,
}

impl Logger {
    pub fn get_logger(sink: &LogSink) -> anyhow::Result<Self> {
        match *sink {
            LogSink::File(p) => Self::new(p),
            LogSink::Console => Ok(Self::new_no_file()),
        }
    }

    fn new(path: &str) -> anyhow::Result<Self> {
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self { file: Some(file) })
    }

    fn new_no_file() -> Self {
        Self { file: None }
    }

    fn emit(&mut self, file_text: &str, console_text: &str) {
        if let Some(file) = self.file.as_mut() {
            let _ = writeln!(file, "{file_text}");
        }
        println!("{console_text}");
    }

    pub fn task(&mut self, task: &str) {
        let task_info = format!("##### START TASK #####\n任务开始 : {task}\n");
        self.emit(&task_info, &task_info);
    }

    pub fn step(&mut self, n: usize) {
        let step_info = format!("----- 开始第 {n} 步 -----");
        self.emit(&step_info, &step_info);
    }

    pub fn record(&mut self, kind: LogKind, body: &str) {
        let full_record: String = match kind {
            LogKind::Thought => {
                format!("AI 思考 : {body}")
            }
            LogKind::Answer => {
                format!("AI 回答 : {body}")
            }
            LogKind::ToolCall => {
                format!("开始调用工具...\nTool Info : {body}")
            }
            LogKind::ToolResult => {
                format!("Tool Result : {body}")
            }
            LogKind::Error => {
                format!("Error : {body}")
            }
            LogKind::TaskResult => {
                format!("任务结束 : {body}\n##### END #####\n")
            }
        };

        if let LogKind::ToolResult = kind
            && self.file.is_some()
        {
            let terminal_record = format!("Tool Result : {} 字节(全文见log文件)", body.len());
            self.emit(&full_record, &terminal_record);
        } else {
            self.emit(&full_record, &full_record);
        }
    }
}
