use std::fs;

use anyhow::Context;

pub fn read_file(path: &str) -> anyhow::Result<String> {
    fs::read_to_string(path).with_context(|| format!("读取 {path} 失败"))
}

pub fn list_files(path: &str) -> anyhow::Result<String> {
    _list_files(path).with_context(|| format!("列取 {path} 失败"))
}

fn _list_files(path: &str) -> anyhow::Result<String> {
    let mut files = vec![];
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        let path = path.into_string().unwrap_or_else(|p| {
            p.to_string_lossy().to_string() + " (原名含有非UTF-8字节，无法使用本工具打开)"
        });
        files.push(path);
    }

    if files.is_empty() {
        return Ok("空目录".to_string());
    }

    files.sort();
    Ok(files.join("\n"))
}

#[cfg(test)]
mod test {

    use super::*;

    #[test]
    fn test_read() {
        let file_content = read_file("plan.md").unwrap();
        assert!(!&file_content.is_empty());
    }
}
