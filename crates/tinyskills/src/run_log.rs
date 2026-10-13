//! Per-run log layout for skill runs.
//!
//! A run writes a human-readable trace to
//! `<workspace>/skills/.runs/<skill>_<ts>_<run>.log`: a header (skill, inputs,
//! task prompt), one line per step appended by the host, then a footer (status,
//! duration, final output). `.runs` is a sibling of the skill definitions
//! (`<workspace>/skills/<id>/`) so run logs never collide with a skill-id
//! directory.
//!
//! This module owns the path layout, the header and footer formats, and the
//! readers that parse them back. It takes plain strings (timestamps included,
//! so it links no clock) and knows nothing of any agent event type: the host
//! formats its own step lines and hands them to [`append_line`].

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

const RESULT_MARKER: &str = "--- result ---";

/// `<workspace>/skills/.runs`.
pub fn runs_dir(workspace: &Path) -> PathBuf {
    workspace.join("skills").join(".runs")
}

/// Replace every character outside `[A-Za-z0-9_-]` with `-`.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// The first 8 characters of an id (the whole id when shorter).
pub fn short(s: &str) -> &str {
    s.get(..8).unwrap_or(s)
}

/// Collapse newlines to spaces and cap at `n` characters, marking a cut with `…`.
pub fn truncate(s: &str, n: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s
    }
}

/// `<runs_dir>/<skill>_<ts>_<short run id>.log`. `timestamp` is the host's
/// compact UTC stamp, e.g. `20260528T075113Z`.
pub fn run_log_path(workspace: &Path, workflow_id: &str, run_id: &str, timestamp: &str) -> PathBuf {
    runs_dir(workspace).join(format!(
        "{}_{}_{}.log",
        sanitize(workflow_id),
        sanitize(timestamp),
        sanitize(short(run_id))
    ))
}

/// Append one line (newline-terminated) to `path`, creating its directory.
pub fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(line.as_bytes())?;
    if !line.ends_with('\n') {
        f.write_all(b"\n")?;
    }
    f.flush()
}

/// Write the run header (skill, inputs, the resolved task prompt). `started`
/// is an RFC 3339 timestamp.
pub fn write_header(
    path: &Path,
    workflow_id: &str,
    run_id: &str,
    started: &str,
    inputs: &Value,
    task_prompt: &str,
) -> std::io::Result<()> {
    let header = format!(
        "==== workflow_run: {workflow_id} ====\n\
         run_id : {run_id}\n\
         started: {started} UTC\n\
         inputs : {inputs}\n\n\
         --- task prompt ---\n{task_prompt}\n\n\
         --- steps ---",
        inputs = serde_json::to_string(inputs).unwrap_or_default(),
    );
    append_line(path, &header)
}

/// Write the final footer: status, duration, and the final output text.
/// `finished` is an RFC 3339 timestamp.
pub fn write_footer(
    path: &Path,
    status: &str,
    elapsed_ms: u64,
    finished: &str,
    output: &str,
) -> std::io::Result<()> {
    let footer = format!(
        "\n{RESULT_MARKER}\n\
         status  : {status}\n\
         duration: {elapsed_ms} ms\n\
         finished: {finished} UTC\n\n{output}\n",
    );
    append_line(path, &footer)
}

/// Detect the degenerate "same paragraph repeated many times in one
/// generation" failure mode: trims each line, counts those of at least
/// `min_len` bytes, and returns the most-repeated line with its count when the
/// count reaches `min_count`.
pub fn detect_repeated_line(
    text: &str,
    min_len: usize,
    min_count: usize,
) -> Option<(String, usize)> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for line in text.lines() {
        let t = line.trim();
        if t.len() >= min_len {
            *counts.entry(t).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, c)| *c >= min_count)
        .max_by_key(|(_, c)| *c)
        .map(|(line, count)| (line.to_string(), count))
}

/// One run extracted from a `.runs/<skill>_<ts>_<run>.log` file. Status is
/// `RUNNING` until the footer lands.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ScannedRun {
    pub run_id: String,
    pub workflow_id: String,
    /// Header `started:` timestamp (RFC 3339); empty if the header was malformed.
    pub started: String,
    /// `DONE` / `STOPPED` / `DEGENERATE` / `FAILED` / `RUNNING` (no footer yet).
    pub status: String,
    /// Footer `duration: <ms> ms`; `None` while running.
    pub duration_ms: Option<u64>,
    /// Footer `finished:` timestamp; `None` while running.
    pub finished: Option<String>,
    /// Path to the streaming log file.
    pub log_path: String,
}

/// Scan `<workspace>/skills/.runs/`, parse each log's header and footer, and
/// return runs sorted by `started` descending. `workflow_id` filters by skill;
/// `limit` caps the result after filtering and sorting. Malformed files are
/// skipped.
pub fn scan_runs(workspace: &Path, workflow_id: Option<&str>, limit: usize) -> Vec<ScannedRun> {
    let mut runs: Vec<ScannedRun> = Vec::new();
    let Ok(entries) = std::fs::read_dir(runs_dir(workspace)) else {
        return runs;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".log") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut sid = String::new();
        let mut rid = String::new();
        let mut started = String::new();
        let mut status = String::from("RUNNING");
        let mut duration_ms: Option<u64> = None;
        let mut finished: Option<String> = None;
        let mut seen_result = false;
        // Labels are right-padded (`status  : DONE`); split on the first `:`
        // and trim both halves.
        for line in text.lines() {
            if line.starts_with("==== workflow_run:") {
                sid = line
                    .trim_start_matches("==== workflow_run:")
                    .trim()
                    .trim_end_matches('=')
                    .trim()
                    .to_string();
                continue;
            }
            if line.starts_with(RESULT_MARKER) {
                seen_result = true;
                continue;
            }
            let Some((label_raw, value_raw)) = line.split_once(':') else {
                continue;
            };
            let label = label_raw.trim();
            let value = value_raw.trim();
            match (label, seen_result) {
                ("run_id", false) => rid = value.to_string(),
                ("started", false) => started = value.to_string(),
                ("status", true) => status = value.to_string(),
                ("duration", true) => {
                    if let Ok(n) = value.trim_end_matches(" ms").trim().parse::<u64>() {
                        duration_ms = Some(n);
                    }
                }
                ("finished", true) => {
                    finished = Some(value.trim_end_matches(" UTC").trim().to_string());
                }
                _ => {}
            }
        }
        if sid.is_empty() || rid.is_empty() {
            continue;
        }
        if workflow_id.is_some_and(|want| sid != want) {
            continue;
        }
        runs.push(ScannedRun {
            run_id: rid,
            workflow_id: sid,
            started,
            status,
            duration_ms,
            finished,
            log_path: path.to_string_lossy().into_owned(),
        });
    }
    runs.sort_by(|a, b| b.started.cmp(&a.started));
    runs.truncate(limit);
    runs
}

/// Resolve a run id to its log path by matching the `_<first 8 chars>.log`
/// file-name suffix, so the caller never supplies a path.
pub fn find_run_log_path(workspace: &Path, run_id: &str) -> Option<PathBuf> {
    if run_id.is_empty() {
        return None;
    }
    let suffix = format!("_{}.log", short(run_id));
    for entry in std::fs::read_dir(runs_dir(workspace)).ok()?.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.ends_with(".log") && name.contains(&suffix) {
            return Some(path);
        }
    }
    None
}

/// Terminal outcome of a finished run, parsed from the footer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub status: String,
    pub output: String,
}

/// Read a run log and, once the footer has landed, return the status and the
/// final output body. `None` while the run is still running, the footer is
/// partially written (no closing `finished:` line), or the file is unreadable.
pub fn read_terminal_outcome(path: &Path) -> Option<RunOutcome> {
    let text = std::fs::read_to_string(path).ok()?;
    let idx = text.find(RESULT_MARKER)?;
    let after = &text[idx + RESULT_MARKER.len()..];
    let mut status = String::new();
    let mut saw_finished = false;
    let mut lines = after.lines();
    for line in lines.by_ref() {
        let Some((label, value)) = line.split_once(':') else {
            if line.trim().is_empty() {
                continue;
            }
            break;
        };
        match label.trim() {
            "status" => status = value.trim().to_string(),
            "finished" => {
                saw_finished = true;
                break;
            }
            _ => {}
        }
    }
    if status.is_empty() || !saw_finished {
        return None;
    }
    let output = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    Some(RunOutcome { status, output })
}

/// One slice of a run log. `offset` is the new read cursor; `complete` is true
/// once the footer landed, so a viewer can stop polling.
#[derive(Debug, Clone, Serialize)]
pub struct RunLogSlice {
    pub offset: u64,
    pub bytes_read: u64,
    pub content: String,
    pub eof: bool,
    pub complete: bool,
}

/// Read up to `max_bytes` of the log from `offset`, for tailing.
pub fn read_run_log_slice(
    path: &Path,
    offset: u64,
    max_bytes: usize,
) -> std::io::Result<RunLogSlice> {
    let mut f = std::fs::File::open(path)?;
    let file_size = f.metadata()?.len();
    if offset >= file_size {
        return Ok(RunLogSlice {
            offset,
            bytes_read: 0,
            content: String::new(),
            eof: true,
            complete: file_has_footer(path)?,
        });
    }
    f.seek(SeekFrom::Start(offset))?;
    let want = ((file_size - offset).min(max_bytes as u64)) as usize;
    let mut buf = vec![0u8; want];
    f.read_exact(&mut buf)?;
    let content = String::from_utf8_lossy(&buf).into_owned();
    let bytes_read = buf.len() as u64;
    let new_offset = offset + bytes_read;
    let eof = new_offset >= file_size;
    let complete = (eof
        && (content.contains("\n--- result ---\n") || content.starts_with("--- result ---\n")))
        || file_has_footer(path)?;
    Ok(RunLogSlice {
        offset: new_offset,
        bytes_read,
        content,
        eof,
        complete,
    })
}

fn file_has_footer(path: &Path) -> std::io::Result<bool> {
    Ok(std::fs::read_to_string(path)?.contains("\n--- result ---\n"))
}

#[cfg(test)]
#[path = "run_log_tests.rs"]
mod tests;
