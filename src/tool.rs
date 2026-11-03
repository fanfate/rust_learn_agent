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

const MAX_TOOL_OUTPUT: usize = 8000;
const LINE_SNAP_MAX: usize = 256;

//工具函数

fn read_file(path: &Path, offset: usize) -> anyhow::Result<String> {
    let base_content = fs::read_to_string(path)?;
    let skip = offset.saturating_sub(1);

    if skip == 0 {
        Ok(base_content)
    } else {
        let full_lines = base_content.lines().count();
        if offset > full_lines {
            bail!("起始行号 {offset} 超过最大行号 {full_lines}");
        }

        if let Some((start_pos, _)) = base_content.match_indices("\n").nth(skip - 1) {
            Ok(base_content[start_pos + 1..].to_string())
        } else {
            // cannot happen
            bail!("未能找到该行 {offset}")
        }
    }
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
    if old_string.is_empty() {
        bail!("旧文本不能为空, 插入请挑一整行做锚点");
    }

    let base_file = fs::read_to_string(path).with_context(|| "读取文件失败")?;
    let base_lines = base_file.lines().count();

    let match_count = base_file.matches(old_string).count();

    if match_count == 0 {
        bail!("未找到旧文本匹配, 请逐字节匹配(含缩进),可能是 CRLF vs LF 差异");
    } else if match_count > 1 && !replace_all {
        bail!(
            "未开启全部替换但找到多处旧文本匹配，请扩大旧文本范围以确保匹配唯一。若确实要全部替换，请置 replace_all 为 true"
        );
    }

    let new_file = if replace_all {
        base_file.replace(old_string, new_string)
    } else {
        base_file.replacen(old_string, new_string, 1)
    };
    let new_lines = new_file.lines().count();

    fs::write(path, &new_file).with_context(|| "写入文件失败")?;

    let re_line_str: String = if replace_all {
        "".to_string()
    } else {
        let pos = base_file.find(old_string).unwrap();
        let re_line = base_file[..pos].bytes().filter(|&b| b == b'\n').count() + 1;
        format!(", 起始第 {re_line} 行")
    };

    Ok(format!(
        "替换 {match_count} 处{re_line_str}; 总行数 {base_lines} -> {new_lines}"
    ))
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

struct Truncation {
    head_end: usize,
    tail_start: usize,
}

impl Truncation {
    fn get_truncate(text: &str, head_bytes: usize, tail_bytes: usize) -> Self {
        let (head_end, tail_start) = truncate_middle(text, head_bytes, tail_bytes);
        Self {
            head_end,
            tail_start,
        }
    }

    fn is_truncated(&self) -> bool {
        self.tail_start > self.head_end
    }

    fn split<'a>(&self, text: &'a str) -> (&'a str, &'a str) {
        (&text[..self.head_end], &text[self.tail_start..])
    }

    fn omitted_bytes(&self) -> usize {
        self.tail_start - self.head_end
    }

    fn head_lines(&self, text: &str) -> usize {
        text[..self.head_end].lines().count() - self.head_partial(text) as usize
    }

    fn head_partial(&self, text: &str) -> bool {
        self.head_end > 0 && self.head_end < text.len() && !text[..self.head_end].ends_with("\n")
    }
}

fn truncate_head(text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }

    let (total_len, total_lines) = (text.len(), text.lines().count());

    let truncation = Truncation::get_truncate(&text, max_bytes, 0);
    let (head, _) = truncation.split(&text);
    let now_lines = truncation.head_lines(&text);
    let now_len = head.len();

    format!(
        "{head}...内容过长，只返回了前 {now_lines} 行/ {now_len} 字节，原文共 {total_lines} 行/ {total_len} 字节\n"
    )
}

fn truncate_head_tail(text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let (total_len, total_lines) = (text.len(), text.lines().count());

    let truncation = Truncation::get_truncate(&text, max_bytes - max_bytes / 4, max_bytes / 4);
    let (head, tail) = truncation.split(&text);
    let middle_len = truncation.omitted_bytes();
    format!(
        "{head}\n...[中间省略 {middle_len} 字节，原文 {total_lines} 行/ {total_len} 字节]\n{tail}"
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
                read_file(&p, args.offset).with_context(|| format!("读取 {} 失败", args.path))
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
                    "成功编辑文件{}, 结果 : {s}",
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
        let raw = inv.raw_invoke(&self.root, self.max_bytes)?;
        match inv {
            Invocation::RunCommand(_) => Ok(truncate_head_tail(raw, self.max_bytes)),
            Invocation::ReadFile(args) => {
                let truncation = Truncation::get_truncate(&raw, self.max_bytes, 0);

                if !truncation.is_truncated() {
                    Ok(raw)
                } else {
                    let skip = args.offset.saturating_sub(1);
                    let full_lines = raw.lines().count() + skip;
                    let (show_content, _) = truncation.split(&raw);
                    if truncation.head_lines(&raw) == 0 {
                        Ok(format!(
                            "{show_content}...[第 {} 行超过单行上限 {} 字节，本工具无法整行返回, 原文共 {} 行；请用 rg -n 定位查看]",
                            skip + 1,
                            self.max_bytes,
                            full_lines,
                        ))
                    } else if !truncation.head_partial(&raw) {
                        let show_lines = truncation.head_lines(&raw);
                        Ok(format!(
                            "{show_content}...[内容过长，只返回 {}-{} 行，原文共 {full_lines} 行；请用 offset={} 继续]",
                            skip + 1,
                            skip + show_lines,
                            skip + show_lines + 1,
                        ))
                    } else {
                        let show_lines = truncation.head_lines(&raw);
                        let partial_bytes =
                            truncation.head_end - show_content.rfind("\n").map_or(0, |p| p + 1);

                        Ok(format!(
                            "{show_content}...[内容过长，只返回完整的 {}-{} 行，第 {} 行仅前 {} 字节，原文共 {full_lines} 行；请用 offset={} 继续]",
                            skip + 1,
                            skip + show_lines,
                            skip + show_lines + 1,
                            partial_bytes,
                            skip + show_lines + 1,
                        ))
                    }
                }
            }
            _ => Ok(truncate_head(raw, self.max_bytes)),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::model::ReadFileArg;

    use super::*;

    fn exec(root: &Path) -> Executor {
        Executor::new(&root.to_string_lossy()).unwrap()
    }

    struct FileHelper {
        path: PathBuf,
    }

    impl FileHelper {
        const TMP_ROOT: &str = "tmp";

        fn new(p: &str) -> Self {
            let dir_path = PathBuf::from(Self::TMP_ROOT).join(p);
            let _ = fs::remove_dir_all(&dir_path);
            fs::create_dir_all(&dir_path).expect("初始化临时目录失败");
            let root = fs::canonicalize(&dir_path).expect("临时目录 canonicalize 失败");
            Self { path: root }
        }

        fn root(&self) -> &Path {
            &self.path
        }

        fn get_path(&self, p: &str) -> PathBuf {
            let mut dir_path = self.path.clone();
            dir_path.push(p);
            dir_path
        }

        fn create_dir(&self, p: &str) -> PathBuf {
            let dir_path = self.get_path(p);
            self.delete_dir(p);
            fs::create_dir_all(&dir_path).expect("创建失败");
            dir_path
        }
        fn delete_dir(&self, p: &str) {
            let dir_path = self.get_path(p);
            let _ = fs::remove_dir_all(&dir_path);
        }

        fn create_file(&self, p: &str, content: &str) {
            let file_path = self.get_path(p);
            fs::write(file_path, content).expect("新建文件失败");
        }
        fn delete_file(&self, p: &str) {
            let file_path = self.get_path(p);
            let _ = fs::remove_file(&file_path);
        }
    }

    impl Drop for FileHelper {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_resolve_no_file_err() {
        let root_file = FileHelper::new("test_resolve_no_file_err");
        let edit_path = "new.txt";

        let err = resolve_in(root_file.root(), edit_path).unwrap_err();
        assert!(format!("{err:#}").contains("无法解析"));
    }

    // test write file
    #[test]
    fn test_write_ok() {
        let root_file = FileHelper::new("test_write_ok");
        let root = root_file.root();
        let p = resolve_for_write(root, "new.txt").unwrap();
        let s = write_file(&p, "x", MAX_TOOL_OUTPUT);
        assert_eq!(s.unwrap(), 1);
        assert_eq!(fs::read_to_string(root.join("new.txt")).unwrap(), "x");
    }

    #[test]
    fn test_write_multi_path_ok() {
        let root_file = FileHelper::new("test_write_multi_path_ok");
        let root = root_file.root();
        let p = resolve_for_write(root, "a/b/c.txt").unwrap();
        let s = write_file(&p, "x", MAX_TOOL_OUTPUT);
        assert_eq!(s.unwrap(), 1);
        assert_eq!(fs::read_to_string(root.join("a/b/c.txt")).unwrap(), "x");
    }

    #[test]
    fn test_write_err() {
        let root_file = FileHelper::new("test_write_err");
        let root = root_file.root();
        let p = resolve_for_write(root, "../c.txt");
        let err = p.unwrap_err();
        assert!(format!("{err:#}").contains("越界"));
    }

    #[test]
    fn test_write_exist_err() {
        let root_file = FileHelper::new("test_write_exist_err");
        root_file.create_dir("a/b");
        let root = root_file.root();

        let p = resolve_for_write(root, "a/b").unwrap();
        let s = write_file(&p, "x", MAX_TOOL_OUTPUT);
        let err = s.unwrap_err();
        assert!(format!("{err:#}").contains("目标已存在"));
    }

    #[test]
    fn test_write_guard() {
        let max_bytes = 10;
        let root_file = FileHelper::new("test_write_guard");
        root_file.create_file("a.txt", "b".repeat(max_bytes + 1).as_str());
        let root = root_file.root();

        let p = resolve_for_write(root, "a.txt").unwrap();
        let s = write_file(&p, "b", max_bytes);
        let err = s.unwrap_err();
        assert!(format!("{err:#}").contains("过大"));
        assert!(format!("{err:#}").contains(&max_bytes.to_string()));
        assert_eq!(fs::read_to_string(&p).unwrap(), "b".repeat(max_bytes + 1));

        let s = write_file(&p, "b", max_bytes + 1);
        assert!(s.is_ok());
        assert_eq!(fs::read_to_string(&p).unwrap(), "b");
    }

    // test edit file
    #[test]
    fn test_edit_ok() {
        let root_file = FileHelper::new("test_edit_ok");
        let edit_path = "new.txt";
        root_file.create_file(edit_path, "abcdefg\n123456\n");

        let p = resolve_in(root_file.root(), edit_path).unwrap();
        let s = edit_file(&p, "12", "ba", false).unwrap();
        assert!(s.contains("起始第 2 行"));
        let r_s = fs::read_to_string(&p).unwrap();
        assert_eq!(r_s.as_str(), "abcdefg\nba3456\n");
    }

    #[test]
    fn test_edit_insert_ok() {
        let root_file = FileHelper::new("test_edit_insert_ok");
        let edit_path = "new.txt";
        root_file.create_file(edit_path, "abcdefg\n123456\n");

        let p = resolve_in(root_file.root(), edit_path).unwrap();
        let s = edit_file(&p, "abcdefg\n", "abcdefg\nhello\n", false).unwrap();
        assert!(s.contains("2 -> 3"));
        let r_s = fs::read_to_string(&p).unwrap();
        assert_eq!(r_s.as_str(), "abcdefg\nhello\n123456\n");
    }

    #[test]
    fn test_edit_replace_all_ok() {
        let root_file = FileHelper::new("test_edit_replace_all_ok");
        let edit_path = "new.txt";
        root_file.create_file(edit_path, "ababababab\nbabababa\n");

        let p = resolve_in(root_file.root(), edit_path).unwrap();
        let s = edit_file(&p, "a", "c", true).unwrap();
        assert!(s.contains("替换 9 处"));
        assert!(s.contains("2 -> 2"));
        let r_s = fs::read_to_string(&p).unwrap();
        assert_eq!(r_s.as_str(), "cbcbcbcbcb\nbcbcbcbc\n");
    }

    #[test]
    fn test_edit_delete_ok() {
        let root_file = FileHelper::new("test_edit_delete_ok");
        let edit_path = "new.txt";
        root_file.create_file(edit_path, "abcdefg\n123456\n");

        let p = resolve_in(root_file.root(), edit_path).unwrap();
        let s = edit_file(&p, "123456\n", "", false).unwrap();
        assert!(s.contains("起始第 2 行"));
        assert!(s.contains("2 -> 1"));
        let r_s = fs::read_to_string(&p).unwrap();
        assert_eq!(r_s.as_str(), "abcdefg\n");
    }

    #[test]
    fn test_edit_chinese_ok() {
        let root_file = FileHelper::new("test_edit_chinese_ok");
        let edit_path = "new.txt";
        root_file.create_file(edit_path, "你好，世界\n你好，未来\n");

        let p = resolve_in(root_file.root(), edit_path).unwrap();
        let s = edit_file(&p, "世界", "World", false).unwrap();
        assert!(s.contains("起始第 1 行"));
        let r_s = fs::read_to_string(&p).unwrap();
        assert_eq!(r_s.as_str(), "你好，World\n你好，未来\n");
    }

    #[test]
    fn test_edit_no_old_err() {
        let root_file = FileHelper::new("test_edit_no_old_err");
        let edit_path = "new.txt";
        root_file.create_file(edit_path, "abcdefg\n123456\n");

        let p = resolve_in(root_file.root(), edit_path).unwrap();
        let err = edit_file(&p, "", "123", false).unwrap_err();
        assert!(format!("{err:#}").contains("旧文本不能为空"));
        let r_s = fs::read_to_string(&p).unwrap();
        assert_eq!(r_s.as_str(), "abcdefg\n123456\n");
    }

    #[test]
    fn test_edit_no_match_err() {
        let root_file = FileHelper::new("test_edit_no_match_err");
        let edit_path = "new.txt";
        root_file.create_file(edit_path, "abcdefg\n123456\n");

        let p = resolve_in(root_file.root(), edit_path).unwrap();
        let err = edit_file(&p, "bbbb", "", false).unwrap_err();
        assert!(format!("{err:#}").contains("未找到旧文本匹配"));
        let r_s = fs::read_to_string(&p).unwrap();
        assert_eq!(r_s.as_str(), "abcdefg\n123456\n");
    }

    #[test]
    fn test_edit_match_many_err() {
        let root_file = FileHelper::new("test_edit_match_many_err");
        let edit_path = "new.txt";
        root_file.create_file(edit_path, "123456\n123456\n123456\n");

        let p = resolve_in(root_file.root(), edit_path).unwrap();
        let err = edit_file(&p, "12", "", false).unwrap_err();
        assert!(format!("{err:#}").contains("找到多处旧文本匹配"));
        let r_s = fs::read_to_string(&p).unwrap();
        assert_eq!(r_s.as_str(), "123456\n123456\n123456\n");
    }

    // test truncate
    #[test]
    fn test_truncate_not_truncated() {
        let origin_text = "abc";
        let t = Truncation::get_truncate(origin_text, 8, 0);
        assert!(!t.is_truncated());
    }

    #[test]
    fn test_truncate_over_head() {
        let origin_text = "abcdefghijklmn";
        let t = Truncation::get_truncate(origin_text, 4, 0);
        let h_line = t.head_lines(origin_text);
        assert_eq!(h_line, 0);
    }

    // test Executor::run
    #[test]
    fn test_run_read_ok() {
        let root_file = FileHelper::new("test_run_read_ok");
        let read_path = "a.txt";
        root_file.create_file(read_path, "abc");

        let s = exec(root_file.root())
            .run(&Invocation::ReadFile(ReadFileArg {
                path: "a.txt".to_string(),
                offset: 1,
            }))
            .unwrap();

        assert_eq!(s, "abc");
    }

    #[test]
    fn test_run_read_long_line() {
        let root_file = FileHelper::new("test_run_read_long_line");
        let read_path = "a.txt";
        root_file.create_file(read_path, "a".repeat(8001).as_str());

        let s = exec(root_file.root())
            .run(&Invocation::ReadFile(ReadFileArg {
                path: "a.txt".to_string(),
                offset: 1,
            }))
            .unwrap();

        assert!(s.contains("rg -n"));
        assert!(s.contains("无法整行返回"));
        assert!(s.contains(&MAX_TOOL_OUTPUT.to_string()));
    }

    #[test]
    fn test_run_read_mult_lines() {
        let root_file = FileHelper::new("test_run_read_mult_lines");
        let read_path = "a.txt";
        let content = (0..2000).fold("".to_string(), |r, acc| format!("{r}line {acc}\n"));
        root_file.create_file(read_path, &content);

        let s = exec(root_file.root())
            .run(&Invocation::ReadFile(ReadFileArg {
                path: "a.txt".to_string(),
                offset: 1,
            }))
            .unwrap();

        let n = if content[..MAX_TOOL_OUTPUT].ends_with("\n") {
            content[..MAX_TOOL_OUTPUT].lines().count()
        } else {
            content[..MAX_TOOL_OUTPUT].lines().count() - 1
        };

        assert!(s.contains(format!("只返回 1-{n} 行").as_str()));
        assert!(s.contains(format!("offset={}", n + 1).as_str()));
        assert!(s.contains("原文共 2000 行"));
    }

    #[test]
    fn test_run_read_mult_big_lines() {
        let root_file = FileHelper::new("test_run_read_mult_big_lines");
        let read_path = "a.txt";
        let content = (0..10).fold("".to_string(), |r, acc| {
            format!("{r}{} {acc}\n", "lines".repeat(1000))
        });
        root_file.create_file(read_path, &content);

        let s = exec(root_file.root())
            .run(&Invocation::ReadFile(ReadFileArg {
                path: "a.txt".to_string(),
                offset: 1,
            }))
            .unwrap();

        let k = MAX_TOOL_OUTPUT - content[..MAX_TOOL_OUTPUT].rfind("\n").map_or(0, |p| p + 1);
        assert!(s.contains(format!("仅前 {k} 字节").as_str()));
    }

    #[test]
    fn test_run_read_mult_lines_offset() {
        let root_file = FileHelper::new("test_run_read_mult_lines_offset");
        let read_path = "a.txt";
        let content = (0..2000).fold("".to_string(), |r, acc| format!("{r}line {acc}\n"));
        root_file.create_file(read_path, &content);

        let s = exec(root_file.root())
            .run(&Invocation::ReadFile(ReadFileArg {
                path: "a.txt".to_string(),
                offset: 3,
            }))
            .unwrap();

        let offset_content = (2..2000).fold("".to_string(), |r, acc| format!("{r}line {acc}\n"));

        let n = if offset_content[..MAX_TOOL_OUTPUT].ends_with("\n") {
            offset_content[..MAX_TOOL_OUTPUT].lines().count()
        } else {
            offset_content[..MAX_TOOL_OUTPUT].lines().count() - 1
        };

        assert!(s.contains(format!("只返回 3-{} 行", n + 2).as_str()));
        assert!(s.contains(format!("offset={}", n + 3).as_str()));
        assert!(s.contains("原文共 2000 行"));
    }

    #[test]
    fn test_truncate_m_short() {
        let base_s = "abc";
        let (he, ts) = truncate_middle(base_s, 8, 0);
        assert_eq!(he, 3);
        assert_eq!(ts, 3);
        assert!(base_s[..he].is_char_boundary(he) && he <= ts);
    }

    #[test]
    fn test_truncate_m_no_ascii_head() {
        let text = "中".repeat(3000);
        let (he, ts) = truncate_middle(&text, 8000, 0);
        assert_eq!(he, 7998);
        assert!(text[..he].is_char_boundary(he) && he <= ts);
    }

    #[test]
    fn test_truncate_m_no_ascii_tail() {
        let text = "中".repeat(3000);
        let (he, ts) = truncate_middle(&text, 0, 8);
        assert_eq!((he, ts), (0, 8994));
        assert!(text[..he].is_char_boundary(he) && text.is_char_boundary(ts) && he <= ts);
    }

    #[test]
    fn test_truncate_m_ascii_head() {
        let text = "a".repeat(9000);
        let (he, ts) = truncate_middle(&text, 8000, 0);
        assert_eq!(he, 8000);
        assert!(text[..he].is_char_boundary(he) && he <= ts);
    }

    #[test]
    fn test_truncate_m_lines_head() {
        let text = "a\n".repeat(8000);
        let (he, ts) = truncate_middle(&text, 6000, 2000);
        assert_eq!((he, ts), (6000, 14002));
        assert!(text[..he].is_char_boundary(he) && text.is_char_boundary(ts) && he <= ts);
    }

    #[test]
    fn test_truncate_m_snap() {
        let mut a = "a".repeat(7745); // 8000 - 7745 = 255 < 256 
        a.push('\n');
        a.push_str(&"b".repeat(1000));

        let (he, _) = truncate_middle(&a, 8000, 0);
        assert_eq!(he, 7746);

        let mut b = "b".repeat(7744); // 8000 - 7744 = 256 
        b.push('\n');
        b.push_str(&"a".repeat(1000));

        let (he, _) = truncate_middle(&b, 8000, 0);
        assert_eq!(he, 8000);
    }
}
