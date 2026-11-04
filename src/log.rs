use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::time::{Instant, SystemTime};

use chrono::{DateTime, Utc};

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
    start_at: Option<Instant>,
    start_wall: Option<SystemTime>,
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
        Ok(Self {
            file: Some(file),
            start_at: None,
            start_wall: None,
        })
    }

    fn new_no_file() -> Self {
        Self {
            file: None,
            start_at: None,
            start_wall: None,
        }
    }

    fn emit(&mut self, file_text: &str, console_text: &str) {
        if let Some(file) = self.file.as_mut() {
            let _ = writeln!(file, "{file_text}");
        }

        let _ = writeln!(io::stdout().lock(), "{console_text}");
    }

    fn start_timer(&mut self) {
        self.start_at = Some(Instant::now());
        self.start_wall = Some(SystemTime::now());
    }

    fn end_timer(&mut self) {
        self.start_at = None;
        self.start_wall = None;
    }

    fn start_time(&self) -> String {
        if let Some(sys_time) = self.start_wall {
            DateTime::<Utc>::from(sys_time).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        } else {
            "".into()
        }
    }

    fn instant_time(&self) -> String {
        if let Some(start) = self.start_at {
            let add_on = start.elapsed().as_secs_f64();
            format!("[+{add_on:>8.3}s] ")
        } else {
            "".into()
        }
    }

    pub fn task(&mut self, task: &str) {
        self.start_timer();
        let task_info = format!(
            "\n##### START TASK, TIMESTAMP : {} #####\n任务开始 : {task}\n",
            self.start_time()
        );
        self.emit(&task_info, &task_info);
    }

    pub fn step(&mut self, n: usize) {
        let step_info = format!("{}----- 开始第 {n} 步 -----", self.instant_time());
        self.emit(&step_info, &step_info);
    }

    pub fn record(&mut self, kind: LogKind, body: &str) {
        let instant_time = self.instant_time();
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
                format!("Tool Result :\n{body}")
            }
            LogKind::Error => {
                format!("Error : {body}")
            }
            LogKind::TaskResult => {
                self.end_timer();
                format!("任务结束 : {body}\n##### END #####\n")
            }
        };

        let full_record = format!("{instant_time}{full_record}");

        if let LogKind::ToolResult = kind
            && self.file.is_some()
        {
            let terminal_record = format!(
                "{instant_time}Tool Result : {} 字节(全文见log文件)",
                body.len()
            );
            self.emit(&full_record, &terminal_record);
        } else {
            self.emit(&full_record, &full_record);
        }
    }
}

impl Drop for Logger {
    fn drop(&mut self) {
        if let Some(instant) = self.start_at {
            let line = format!(
                "[+{:>8.3}s] 未正常结束，启动于 {}",
                instant.elapsed().as_secs_f64(),
                self.start_wall
                    .map(|w| DateTime::<Utc>::from(w)
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                    .unwrap_or_default()
            );

            self.emit(&line, &line);
        }
    }
}
