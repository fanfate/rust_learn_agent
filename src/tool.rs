use crate::{
    config,
    model::{Invocation, ToolKind},
};
use anyhow::{Context, bail};
use std::{
    ffi::OsStr,
    fs,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

const MAX_TOOL_OUTPUT: usize = 8 * 1024;
const LINE_SNAP_MAX: usize = 256;

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
fn write_file(path: &Path, content: &str, max_bytes: usize) -> anyhow::Result<usize> {
    let meta_res = fs::symlink_metadata(path);

    match meta_res {
        Ok(meta) => {
            if !meta.is_file() {
                bail!("目标已存在，但并不是普通文件!");
            }

            if meta.len() > max_bytes as u64 {
                bail!("文件过大，超过 {max_bytes} 字节，请使用edit_file工具");
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

fn edit_file(
    path: &Path,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> anyhow::Result<String> {
    todo!()
}

fn run_command(program: &str, args: &[String], root: &Path) -> anyhow::Result<String> {
    let mut cmd = Command::new(program);

    let disallow_path = config::LLM_ENV_KEYS;
    for key in disallow_path {
        cmd.env_remove(key);
    }
    cmd.current_dir(root);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.stdin(Stdio::null());

    let output = cmd.args(args).spawn()?.wait_with_output()?;

    let res_code = output.status.code().unwrap_or(-1);
    let res_out = String::from_utf8_lossy_owned(output.stdout);
    let res_error = String::from_utf8_lossy_owned(output.stderr);

    let mut res_str = format!("命令退出码: {res_code}\n");
    if !res_out.is_empty() {
        res_str = format!("{res_str}返回结果: {res_out}\n");
    }
    if !res_error.is_empty() {
        res_str = format!("{res_str}错误信息: {res_error}\n");
    }

    Ok(res_str)
}

// 参数处理和输出后处理函数

fn truncate_head(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }

    let (total_len, total_lines) = (text.len(), text.lines().count());

    let (head, _) = truncate_middle(&text, max_bytes, 0);
    text.truncate(head);
    let now_lines = text.lines().count();
    let now_len = text.len();
    format!(
        "{text}...内容过长，只返回了前{now_lines}行/{now_len}字节，原文共{total_lines}行/{total_len}字节\n"
    )
}

fn truncate_head_tail(text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let (total_len, total_lines) = (text.len(), text.lines().count());

    let (head, tail) = truncate_middle(&text, max_bytes - max_bytes / 4, max_bytes / 4);

    let middle_lines = &text[head..tail].lines().count();
    let middle_len = &text[head..tail].len();

    format!(
        "{}\n...[中间省略{middle_lines}行/{middle_len}字节，原文{total_lines}行/{total_len}字节]\n{}",
        &text[..head],
        &text[tail..]
    )
}

fn truncate_middle(text: &str, head_bytes: usize, tail_bytes: usize) -> (usize, usize) {
    let total_bytes = text.len();
    if total_bytes <= head_bytes + tail_bytes {
        return (total_bytes, total_bytes);
    }

    let mut head_end = std::cmp::min(head_bytes, total_bytes);
    if let Some(pos) = text.as_bytes()[..head_end]
        .iter()
        .rposition(|&b| b == b'\n')
        && head_end - pos < LINE_SNAP_MAX
    {
        head_end = pos + 1;
    };
    while !text.is_char_boundary(head_end) {
        head_end -= 1;
    }

    let mut tail_start = total_bytes.saturating_sub(tail_bytes);
    if let Some(pos) = text.as_bytes()[tail_start..]
        .iter()
        .position(|&b| b == b'\n')
        && pos < LINE_SNAP_MAX
    {
        tail_start = tail_start + pos + 1;
    }
    while !text.is_char_boundary(tail_start) {
        tail_start += 1;
    }

    debug_assert!(
        head_end <= tail_start,
        "head_end={head_end} tail_start={tail_start} total={total_bytes}"
    );

    (head_end, tail_start)
}

fn resolve_in(root: &Path, p: &str) -> anyhow::Result<PathBuf> {
    let target = if Path::new(p).is_absolute() {
        p.into()
    } else {
        root.join(p)
    };

    let canon = fs::canonicalize(&target)
        .with_context(|| format!("无法解析路径 {p} , 可用 list_files '.' 查看根目录"))?;
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
        bail!("路径越界(不在沙箱内，可能经过符号连接): {p}");
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
                    bail!("路径中含过多'..'，越界沙箱根")
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
    fn raw_invoke(&self, root: &Path, max_bytes: usize) -> anyhow::Result<String> {
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
                let s = write_file(&p, &args.content, max_bytes)
                    .with_context(|| format!("写入 {} 失败", args.path))?;
                Ok(format!(
                    "成功写入文件{} : {s} 字节",
                    p.strip_prefix(root)
                        .expect("canon 必然以词法祖先为前缀")
                        .display()
                ))
            }
            Invocation::EditFile(args) => {
                let p = resolve_in(root, &args.path)?;
                let s = edit_file(&p, &args.old_string, &args.new_string, args.replace_all)
                    .with_context(|| format!("编辑 {} 失败", args.path))?;
                Ok(format!(
                    "成功编辑文件{}, diff 为 :\n{s}\n",
                    p.strip_prefix(root)
                        .expect("canon 必然以词法祖先为前缀")
                        .display()
                ))
            }
            Invocation::RunCommand(args) => {
                if Path::new(&args.program).file_name() != Some(OsStr::new(&args.program)) {
                    bail!("program 必须是裸名（不带路径），命令不合法");
                }
                if !ToolKind::ALLOW_CMD.contains(&args.program.as_str()) {
                    bail!(
                        "{} 不是允许的合法命令, 合法命令为: [{}]",
                        args.program,
                        ToolKind::ALLOW_CMD.join(", ")
                    );
                }
                run_command(&args.program, &args.args, root)
                    .with_context(|| format!("无法执行命令 {}", args.program))
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
        let raw = inv.raw_invoke(&self.root, self.max_bytes)?;
        match inv {
            Invocation::RunCommand(_) => Ok(truncate_head_tail(raw, self.max_bytes)),
            _ => Ok(truncate_head(raw, self.max_bytes)),
        }
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
            let s = write_file(&p, "x", MAX_TOOL_OUTPUT);
            assert!(s.is_ok());
            assert_eq!(fs::read_to_string(root.join("new.txt")).unwrap(), "x");
        }

        {
            let p = resolve_for_write(&root, "a/b/c.txt").unwrap();
            let s = write_file(&p, "x", MAX_TOOL_OUTPUT);
            assert!(s.is_ok());
            assert_eq!(fs::read_to_string(root.join("a/b/c.txt")).unwrap(), "x");
        }
        {
            let p = resolve_for_write(&root, "../c.txt");
            assert!(p.is_err());
        }
        {
            let p = resolve_for_write(&root, "a/b").unwrap();
            let s = write_file(&p, "x", MAX_TOOL_OUTPUT);
            assert!(s.is_err());
            if let Err(err) = s {
                assert!(format!("{err:#}").contains("目标已存在"));
            };
        }
    }
}
