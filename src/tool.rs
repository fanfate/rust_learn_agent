use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};

use crate::model::Invocation;

const MAX_TOOL_OUTPUT: usize = 8 * 1024;

pub fn read_file(path: &Path) -> anyhow::Result<String> {
    fs::read_to_string(path).with_context(|| format!("读取 {} 失败", path.display()))
}

pub fn list_files(path: &Path) -> anyhow::Result<String> {
    _list_files(path).with_context(|| format!("列取 {} 失败", path.display()))
}

fn _list_files(path: &Path) -> anyhow::Result<String> {
    let mut files = vec![];
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.file_name();
        let path = path.into_string().unwrap_or_else(|p| {
            p.to_string_lossy().to_string() + " (原名含有非UTF-8字节，无法使用本工具打开)"
        });
        let path_type = if entry.file_type()?.is_dir() {
            "DIR"
        } else {
            "FILE"
        };
        files.push(format!("[{path_type}] {path}"));
    }

    if files.is_empty() {
        return Ok("空目录".to_string());
    }

    files.sort();
    Ok(files.join("\n"))
}

fn truncates(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }

    let mut end = max_bytes;
    if let Some(pos) = text.as_bytes()[..end].iter().rposition(|&b| b == b'\n') {
        end = pos + 1;
    }
    while !text.is_char_boundary(end) {
        end -= 1;
    }

    text.truncate(end);
    format!(
        "{text}...[过长截断, 最大{max_bytes}字节, 原文{}行/{}字节]\n",
        text.lines().count(),
        text.len()
    )
}

fn resolve_in(root: &Path, p: &str) -> anyhow::Result<PathBuf> {
    let target = if Path::new(p).is_absolute() {
        p.into()
    } else {
        root.join(p)
    };

    let canon = std::fs::canonicalize(&target)
        .with_context(|| format!("无法解析路径{p}，沙箱根{}", root.display()))?;
    if !canon.starts_with(root) {
        bail!("路径越界: {p}");
    }

    Ok(canon)
}

impl Invocation {
    fn raw_invoke(&self, root: &Path) -> anyhow::Result<String> {
        match self {
            Invocation::ReadFile(args) => {
                let p = resolve_in(root, &args.path)?;
                read_file(&p)
            }
            Invocation::ListFiles(args) => {
                let p = resolve_in(root, &args.path)?;
                list_files(&p)
            }
            _ => bail!("工具尚不支持"),
        }
    }
}

#[derive(Debug)]
pub struct Executor {
    root: PathBuf,
    max_bytes: usize,
}

impl Executor {
    pub fn new(root: &str) -> anyhow::Result<Self> {
        let mut root = PathBuf::from(root);
        root = root.canonicalize()?;
        Ok(Self {
            root,
            max_bytes: MAX_TOOL_OUTPUT,
        })
    }

    pub fn run(&self, inv: &Invocation) -> anyhow::Result<String> {
        println!("开始调用工具...");
        println!("ToolInfo : {:?}", inv);
        let raw = inv.raw_invoke(&self.root)?;
        Ok(truncates(raw, self.max_bytes))
    }
}
