use crate::model::{Invocation, ToolKind};
use anyhow::{Context, bail};
use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

const MAX_TOOL_OUTPUT: usize = 8 * 1024;

//工具函数

fn read_file(path: &Path) -> anyhow::Result<String> {
    Ok(fs::read_to_string(path)?)
}

fn list_files(path: &Path) -> anyhow::Result<String> {
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

// write_file 会创建缺失的父目录，无条件信任 path 已经过沙箱检查
fn write_file(path: &Path, content: &str) -> anyhow::Result<usize> {
    let meta_res = fs::symlink_metadata(path);

    match meta_res {
        Ok(meta) => {
            if !meta.is_file() {
                bail!("目标已存在，但并不是普通文件!");
            }

            fs::write(path, content).with_context(|| "写入路径失败")?;
            Ok(content.len())
        }

        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(err).with_context(|| "无法读取文件状态");
            }

            let path_p = path.parent().with_context(|| "父路径提取失败")?;

            fs::create_dir_all(path_p).with_context(|| "创建父路径失败")?;

            fs::write(path, content).with_context(|| "创建父路径后写入失败")?;
            Ok(content.len())
        }
    }
}

fn run_command(program: &str, args: &[String], root: &Path) -> anyhow::Result<String> {
    match Path::new(program).file_name() {
        Some(p) => {
            if p != program {
                bail!("program 必须是裸名（不带路径、不带 .exe），命令不合法");
            }
        }
        None => {
            bail!("未找到可执行文件，命令不合法");
        }
    }

    let mut cmd = Command::new(program);

    let allow_path: &[&str] = &["PATH", "HOME", "USERPROFILE", "SystemRoot", "TEMP", "TMP"];
    cmd.env_clear();
    for key in allow_path {
        if let Ok(val) = env::var(key) {
            cmd.env(key, val);
        }
    }
    cmd.current_dir(root);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.stdin(Stdio::null());

    let output = cmd.args(args).spawn()?.wait_with_output()?;

    let res_code = output.status.code().unwrap_or(-1);
    let res_out = String::from_utf8_lossy_owned(output.stdout);
    let res_error = String::from_utf8_lossy_owned(output.stderr);

    Ok(format!(
        "命令退出码: {res_code}\n返回结果: {res_out}\n错误信息: {res_error}\n"
    ))
}

// 参数处理和输出后处理函数

fn truncates(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }

    let (line_count, length) = (text.lines().count(), text.len());

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
        line_count, length
    )
}

fn resolve_in(root: &Path, p: &str) -> anyhow::Result<PathBuf> {
    let target = if Path::new(p).is_absolute() {
        p.into()
    } else {
        root.join(p)
    };

    let canon = fs::canonicalize(&target)
        .with_context(|| format!("无法解析路径{p}, 可用 list_files '.' 查看根目录"))?;
    if !canon.starts_with(root) {
        bail!("路径越界: {p}");
    }

    Ok(canon)
}

fn resolve_for_write(root: &Path, p: &str) -> anyhow::Result<PathBuf> {
    let target = if Path::new(p).is_absolute() {
        p.into()
    } else {
        root.join(p)
    };

    let canon = normalize_path(&target)?;
    let mut canon_tmp = canon.as_path();
    loop {
        if canon_tmp.try_exists()? {
            break;
        }

        let Some(parent) = canon_tmp.parent() else {
            break;
        };

        canon_tmp = parent;
    }

    let canon_ancestor = fs::canonicalize(canon_tmp)?;

    if !canon_ancestor.starts_with(root) {
        bail!("路径越界: {p}");
    }

    let tail = canon.strip_prefix(canon_tmp)?;
    if tail.as_os_str().is_empty() {
        Ok(canon_ancestor)
    } else {
        Ok(canon_ancestor.join(tail))
    }
}

fn normalize_path(path: &Path) -> anyhow::Result<PathBuf> {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => (),
            Component::ParentDir => {
                if !out.pop() {
                    bail!("路径越界")
                }
            }
            Component::Normal(name) => {
                out.push(name);
            }
            c => out.push(c.as_os_str()),
        }
    }

    Ok(out)
}

// tool call

impl Invocation {
    fn raw_invoke(&self, root: &Path) -> anyhow::Result<String> {
        match self {
            Invocation::ReadFile(args) => {
                let p = resolve_in(root, &args.path)?;
                read_file(&p).with_context(|| format!("读取 {} 失败", args.path))
            }
            Invocation::ListFiles(args) => {
                let p = resolve_in(root, &args.path)?;
                list_files(&p).with_context(|| format!("列取 {} 失败", args.path))
            }
            Invocation::WriteFile(args) => {
                let p = resolve_for_write(root, &args.path)?;
                let s = write_file(&p, &args.content)
                    .with_context(|| format!("写入 {} 失败", args.path))?;
                Ok(format!(
                    "成功写入文件{} : {s} 字节",
                    p.strip_prefix(root)?.display()
                ))
            }
            Invocation::RunCommand(args) => {
                if !ToolKind::ALLOW_CMD.contains(&args.program.as_str()) {
                    bail!(
                        "{} 不是允许的合法命令, 合法命令为: [{}]",
                        args.program,
                        ToolKind::ALLOW_CMD.join(", ")
                    );
                }
                run_command(&args.program, &args.args, root)
                    .with_context(|| format!("执行命令 {} 失败", args.program))
            }
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
        println!("Tool Info : {}", inv.info());
        let raw = inv.raw_invoke(&self.root)?;
        Ok(truncates(raw, self.max_bytes))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_resolve_write() {
        let root = fs::canonicalize("scratch/tmp").unwrap();
        {
            let p = resolve_for_write(&root, "new.txt").unwrap();
            let s = write_file(&p, "x");
            assert!(s.is_ok());
            assert_eq!(fs::read_to_string(root.join("new.txt")).unwrap(), "x");
        }

        {
            let p = resolve_for_write(&root, "a/b/c.txt").unwrap();
            let s = write_file(&p, "x");
            assert!(s.is_ok());
            assert_eq!(fs::read_to_string(root.join("a/b/c.txt")).unwrap(), "x");
        }
        {
            let p = resolve_for_write(&root, "../c.txt");
            assert!(p.is_err());
        }
        {
            let p = resolve_for_write(&root, "a/b").unwrap();
            let s = write_file(&p, "x");
            assert!(s.is_err());
            if let Err(err) = s {
                assert!(format!("{err:#}").contains("目标已存在"));
            };
        }
    }
}
