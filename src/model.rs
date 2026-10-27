use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,

        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: Option<FunctionCall>,
    pub custom: Option<CustomCall>,
}

impl ToolCall {
    pub fn payload(&self) -> Result<&FunctionCall, ToolError> {
        self.function
            .as_ref()
            .ok_or(ToolError::UnsupportedCallType {
                kind: self.kind.clone(),
            })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CustomCall {
    pub name: String,
    pub input: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadFileArg {
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListFilesArg {
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteFileArg {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditFileArg {
    pub path: String,
    pub old_string: String,
    pub new_string: String,

    #[serde(default = "default_replace")]
    pub replace_all: bool,
}

fn default_replace() -> bool {
    false
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunCommandArg {
    pub program: String,

    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug)]
pub enum Invocation {
    ReadFile(ReadFileArg),
    ListFiles(ListFilesArg),
    WriteFile(WriteFileArg),
    EditFile(EditFileArg),
    RunCommand(RunCommandArg),
}

impl Invocation {
    pub fn parse_kind(kind: &ToolKind, args: &str) -> Result<Self, ToolError> {
        match kind {
            ToolKind::ReadFile => {
                let read_args: ReadFileArg =
                    serde_json::from_str(args).map_err(|err| ToolError::WrongParams {
                        name: kind.name().to_string(),
                        details: err.to_string(),
                    })?;
                Ok(Self::ReadFile(read_args))
            }
            ToolKind::ListFiles => {
                let list_args: ListFilesArg =
                    serde_json::from_str(args).map_err(|err| ToolError::WrongParams {
                        name: kind.name().to_string(),
                        details: err.to_string(),
                    })?;
                Ok(Self::ListFiles(list_args))
            }
            ToolKind::WriteFile => {
                let write_args: WriteFileArg =
                    serde_json::from_str(args).map_err(|err| ToolError::WrongParams {
                        name: kind.name().to_string(),
                        details: err.to_string(),
                    })?;
                Ok(Self::WriteFile(write_args))
            }
            ToolKind::EditFile => {
                let edit_args: EditFileArg =
                    serde_json::from_str(args).map_err(|err| ToolError::WrongParams {
                        name: kind.name().to_string(),
                        details: err.to_string(),
                    })?;
                Ok(Self::EditFile(edit_args))
            }
            ToolKind::RunCommand => {
                let runcmd_args: RunCommandArg =
                    serde_json::from_str(args).map_err(|err| ToolError::WrongParams {
                        name: kind.name().to_string(),
                        details: err.to_string(),
                    })?;
                Ok(Self::RunCommand(runcmd_args))
            }
        }
    }

    pub fn info(&self) -> String {
        match self {
            Self::ReadFile(args) => format!("read_file, path : {}", args.path),
            Self::ListFiles(args) => format!("list_files, path : {}", args.path),
            Self::WriteFile(args) => format!("write_file, path : {}", args.path),
            Self::EditFile(args) => format!(
                "edit_file, path : {}, replace_all : {}\nold :\n{},\nnew :\n{}\n",
                args.path, args.replace_all, args.old_string, args.new_string,
            ),
            Self::RunCommand(args) => format!(
                "run_command, program : {}, args : [{}]",
                args.program,
                args.args.join(", ")
            ),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct WireTool {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionDecl,
}

#[derive(Debug, Serialize)]
pub struct FunctionDecl {
    pub name: &'static str,
    pub description: String,
    pub parameters: serde_json::Value,
    pub strict: bool,
}

#[derive(Debug, Clone)]
pub enum ToolKind {
    ReadFile,
    ListFiles,
    WriteFile,
    EditFile,
    RunCommand,
}

impl ToolKind {
    const ALL: &'static [ToolKind] = &[
        ToolKind::ReadFile,
        ToolKind::ListFiles,
        ToolKind::WriteFile,
        ToolKind::EditFile,
        ToolKind::RunCommand,
    ];

    pub const ALLOW_CMD: &'static [&str] = &["cargo", "rg", "yq", "jq"];

    pub fn name(&self) -> &'static str {
        match self {
            ToolKind::ReadFile => "read_file",
            ToolKind::ListFiles => "list_files",
            ToolKind::WriteFile => "write_file",
            ToolKind::RunCommand => "run_command",
            ToolKind::EditFile => "edit_file",
        }
    }

    pub fn spec(&self) -> FunctionDecl {
        match self {
            ToolKind::ReadFile => FunctionDecl {
                name: "read_file",
                description: "读取文件内容，路径相对沙箱根目录".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"],
                    "additionalProperties": false
                }),
                strict: true,
            },
            ToolKind::ListFiles => FunctionDecl {
                name: "list_files",
                description:
                    "列出路径下的所有文件，路径相对沙箱根目录，前缀DIR代表路径，FILE代表文件"
                        .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"],
                    "additionalProperties": false
                }),
                strict: true,
            },
            ToolKind::WriteFile => FunctionDecl {
                name: "write_file",
                description: "写入文件内容，路径相对沙箱根目录, 覆盖整个文件（不是追加/插入）,\
                    会自动创建缺失的父目录,返回写入文件的字节数"
                    .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" }, "content" : { "type" : "string" } },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }),
                strict: true,
            },
            ToolKind::EditFile => FunctionDecl {
                name: "edit_file",
                description: "更新文件内容，路径相对沙箱根目录, 主要作用是追加/插入/替换, 返回替换信息。要求如下\n \
                    1. 默认（replace_all = false）时 old_string 必须在文件里恰好出现一次；出现多次就报错，要补足上下文把它变成唯一。\
                    2. replace_all = true 时允许出现多处，并全部替换；这时不要把 old_string 选得太通用（只由空格、}、单个关键字组成）。\
                    3. 找不到（0 处）在两种情况下都算错。\
                    4. 插入：old_string 挑一个锚点（通常是一整行），new_string 里把锚点原样保留再插入新行；不要试图用空串定位。"
                    .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" }, "old_string":{"type" : "string"}, "new_string" : {"type" : "string"}, "replace_all" : { "type" : "boolean" } },
                    "required": ["path", "old_string", "new_string", "replace_all"],
                    "additionalProperties": false
                }),
                strict: true,
            },
            ToolKind::RunCommand => FunctionDecl {
                name: "run_command",
                description: format!(
                    "运行命令行命令。\n1.没有 shell：不支持管道、重定向、通配符、变量展开; \n\
                        2.program 只接受裸名，且必须在白名单里，白名单为: [{}];\n\
                        3.工作目录固定在沙箱根，路径都相对它;\n\
                        4.返回：退出码 + stdout + stderr；输出过长会被截断",
                    Self::ALLOW_CMD.join(", ")
                ),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "program": { "type": "string" }, "args" : { "type" : "array", "items" : {"type" : "string"} } },
                    "required": ["program"],
                    "additionalProperties": false
                }),
                strict: false,
            },
        }
    }

    pub fn from_name(n: &str) -> Option<ToolKind> {
        Self::ALL.iter().find(|t| t.name() == n).cloned()
    }

    pub fn tools() -> Vec<WireTool> {
        Self::ALL
            .iter()
            .map(|tool| WireTool {
                kind: "function",
                function: tool.spec(),
            })
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("type {kind} 不是function")]
    UnsupportedCallType { kind: String },

    #[error("Tool {name} 不支持")]
    UnknownTool { name: String },

    #[error("Tool {name} 参数有误, {details}")]
    WrongParams { name: String, details: String },

    #[error("Tool 为空")]
    Empty,
}

#[derive(Debug, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<WireTool>,
    pub tool_choice: String,
    pub parallel_tool_calls: bool,
}

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    pub id: String,
    pub object: String,
    pub model: String,
    pub choices: Vec<Choice>,
    // usage和其他项对当前阶段没什么用，先不加
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub index: i64,
    pub finish_reason: Option<String>,
    pub message: Message,
}
