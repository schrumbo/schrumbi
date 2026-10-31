use super::cmd::{BULK, Git, GitError};

/// One earlier state of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub rev: String,
    /// Unix time.
    pub time: i64,
    pub subject: String,
    /// The file's path in that state; it may have had another name then.
    pub path: String,
}

/// Parses `git log --follow --name-only -z --format=%x01%H%x1f%ct%x1f%s`.
pub fn parse_versions(out: &str, fallback: &str) -> Vec<Version> {
    let mut list: Vec<Version> = Vec::new();
    for piece in out.split('\0') {
        let piece = piece.trim_start_matches('\n');
        if let Some(header) = piece.strip_prefix('\u{1}') {
            let (line, rest) = header.split_once('\n').unwrap_or((header, ""));
            let mut fields = line.split('\u{1f}');
            let (Some(rev), Some(time), Some(subject)) = (fields.next(), fields.next(), fields.next()) else { continue };
            list.push(Version { rev: rev.to_string(), time: time.parse().unwrap_or(0), subject: subject.to_string(), path: fallback.to_string() });
            let rest = rest.trim();
            if !rest.is_empty()
                && let Some(v) = list.last_mut()
            {
                v.path = rest.to_string();
            }
        } else if !piece.trim().is_empty()
            && let Some(v) = list.last_mut()
        {
            v.path = piece.trim().to_string();
        }
    }
    list
}

impl Git {
    /// States of a file, newest first, one page at a time; follows renames.
    pub fn versions(&self, path: &str, skip: usize, limit: usize) -> Result<Vec<Version>, GitError> {
        let (skip, limit) = (format!("--skip={skip}"), format!("--max-count={limit}"));
        let out = self.run_bulk(&["log", "--follow", "--name-only", "-z", "--format=%x01%H%x1f%ct%x1f%s", &skip, &limit, "--", path], None)?;
        Ok(parse_versions(&out, path))
    }

    /// A file's bytes in an earlier state.
    pub fn version_bytes(&self, v: &Version) -> Result<Vec<u8>, GitError> {
        let out = self.output(&["cat-file", "blob", &format!("{}:{}", v.rev, v.path)], None, BULK)?;
        if out.code == 0 { Ok(out.raw) } else { Err(GitError::Other(out.stderr.trim().to_string())) }
    }
}
