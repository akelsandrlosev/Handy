//! Dictated bug reports: the transcript is structured by the configured LLM
//! (title, steps, expected, actual) and saved as one Markdown file per report.
//! Without a usable LLM the raw transcript is saved instead, so a dictation is
//! never lost.

use crate::settings::{AppSettings, APPLE_INTELLIGENCE_PROVIDER_ID};
use anyhow::Context;
use chrono::{DateTime, Local};
use log::{debug, warn};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// Same budget as post-processing: a hung endpoint must not hold the pipeline.
const STRUCTURE_TIMEOUT: Duration = Duration::from_secs(60);

/// Folder name under the user's Documents folder when no folder is set.
const DEFAULT_DIR_NAME: &str = "Handy Bug Reports";

const MAX_SLUG_CHARS: usize = 60;

const SYSTEM_PROMPT: &str = "You turn a dictated, possibly rambling bug report into a \
structured report. Use only what the speaker said; never invent details, versions or \
steps. Fix obvious speech-recognition errors and drop filler words. Keep the speaker's \
language. Leave a field empty (empty string or empty list) when it was not mentioned. \
`title` is a short, specific one-line summary. `steps` are the actions needed to \
reproduce the problem, in order, one per item, without numbering. Reply with a single \
JSON object with the keys: title, summary, steps, expected, actual, notes.";

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct BugReport {
    pub title: String,
    pub summary: String,
    pub steps: Vec<String>,
    pub expected: String,
    pub actual: String,
    pub notes: String,
}

impl BugReport {
    /// The report used when the LLM can't structure the transcript.
    pub fn from_raw(transcript: &str) -> Self {
        Self {
            title: fallback_title(transcript),
            notes: transcript.trim().to_string(),
            ..Self::default()
        }
    }
}

/// Longest fallback title, in words.
const FALLBACK_TITLE_WORDS: usize = 10;

/// The first sentence of the transcript, cut to a few words, as a title. Cut
/// at sentence ends only, and never ending on punctuation that reads as
/// unfinished.
fn fallback_title(transcript: &str) -> String {
    let first_sentence = transcript
        .trim()
        .split(['.', '!', '?', '。', '！', '？', '\n'])
        .next()
        .unwrap_or_default();
    let words: Vec<&str> = first_sentence
        .split_whitespace()
        .take(FALLBACK_TITLE_WORDS)
        .collect();
    let title = words
        .join(" ")
        .trim_end_matches(|c: char| c.is_ascii_punctuation() || "—–…«»".contains(c))
        .trim()
        .to_string();
    if title.is_empty() {
        "Bug report".to_string()
    } else {
        title
    }
}

/// Why a report couldn't be structured, so the user can be told what to fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureError {
    /// No provider, model or API key is set up for post-processing.
    NotConfigured,
    /// A provider is set up, but the request or its reply was unusable.
    Failed,
}

fn json_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "summary": { "type": "string" },
            "steps": { "type": "array", "items": { "type": "string" } },
            "expected": { "type": "string" },
            "actual": { "type": "string" },
            "notes": { "type": "string" }
        },
        "required": ["title", "summary", "steps", "expected", "actual", "notes"],
        "additionalProperties": false
    })
}

/// Parses the model's reply, tolerating a ```json fence or prose around the
/// object. A report with no title and no content is rejected.
fn parse_report(reply: &str) -> Option<BugReport> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    let report: BugReport = serde_json::from_str(reply.get(start..=end)?).ok()?;
    let has_content = !report.title.trim().is_empty()
        || !report.summary.trim().is_empty()
        || !report.actual.trim().is_empty()
        || !report.steps.is_empty();
    has_content.then_some(report)
}

/// Asks the configured post-processing provider to structure the transcript.
/// On error the caller falls back to [`BugReport::from_raw`].
pub async fn structure(
    settings: &AppSettings,
    transcript: &str,
) -> Result<BugReport, StructureError> {
    let provider = settings
        .active_post_process_provider()
        .ok_or(StructureError::NotConfigured)?
        .clone();
    if provider.id == APPLE_INTELLIGENCE_PROVIDER_ID {
        debug!("Bug report structuring is not supported with Apple Intelligence");
        return Err(StructureError::Failed);
    }
    let model = settings
        .post_process_models
        .get(&provider.id)
        .map(|m| m.trim().to_string())
        .unwrap_or_default();
    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    // Only a custom endpoint (e.g. a local server) can work without a key.
    if model.is_empty() || (api_key.trim().is_empty() && provider.id != "custom") {
        debug!(
            "Bug report structuring skipped: '{}' has no model or API key",
            provider.id
        );
        return Err(StructureError::NotConfigured);
    }
    let disable_reasoning = matches!(provider.id.as_str(), "custom" | "openrouter");

    let schema = provider.supports_structured_output.then(json_schema);
    let request = crate::llm_client::send_chat_completion_with_schema(
        &provider,
        api_key,
        &model,
        transcript.to_string(),
        Some(SYSTEM_PROMPT.to_string()),
        schema,
        disable_reasoning,
    );
    match tokio::time::timeout(STRUCTURE_TIMEOUT, request).await {
        Ok(Ok(Some(reply))) => parse_report(&reply).ok_or_else(|| {
            warn!("Bug report structuring returned an unusable reply");
            StructureError::Failed
        }),
        Ok(Ok(None)) => {
            warn!("Bug report structuring returned no content");
            Err(StructureError::Failed)
        }
        Ok(Err(e)) => {
            warn!("Bug report structuring failed: {e}");
            Err(StructureError::Failed)
        }
        Err(_) => {
            warn!("Bug report structuring timed out after {STRUCTURE_TIMEOUT:?}");
            Err(StructureError::Failed)
        }
    }
}

/// Lowercase ASCII-and-digit slug for the file name; empty when the title has
/// none (e.g. non-Latin script), in which case the timestamp alone names it.
fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= MAX_SLUG_CHARS {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

fn push_section(out: &mut String, heading: &str, body: &str) {
    let body = body.trim();
    if !body.is_empty() {
        out.push_str(&format!("## {heading}\n\n{body}\n\n"));
    }
}

/// One line per call, so a title can't break out of its heading.
fn single_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn render_markdown(
    report: &BugReport,
    transcript: &str,
    created: DateTime<Local>,
    environment: &str,
) -> String {
    let title = single_line(&report.title);
    let title = if title.is_empty() {
        "Bug report".to_string()
    } else {
        title
    };

    let mut out = format!(
        "# {title}\n\n- **Reported:** {}\n- **Environment:** {environment}\n\n",
        created.format("%Y-%m-%d %H:%M")
    );
    push_section(&mut out, "Summary", &report.summary);

    let steps: Vec<String> = report
        .steps
        .iter()
        .map(|s| single_line(s))
        .filter(|s| !s.is_empty())
        .enumerate()
        .map(|(i, s)| format!("{}. {s}", i + 1))
        .collect();
    push_section(&mut out, "Steps to reproduce", &steps.join("\n"));

    push_section(&mut out, "Expected behavior", &report.expected);
    push_section(&mut out, "Actual behavior", &report.actual);
    push_section(&mut out, "Notes", &report.notes);

    // Dictated text could contain "</details>", so it goes in a fenced block
    // with a fence longer than any backtick run inside it.
    let transcript = transcript.trim();
    if !transcript.is_empty() {
        let longest_run = transcript
            .split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0);
        let fence = "`".repeat((longest_run + 1).max(3));
        out.push_str(&format!(
            "---\n\n<details>\n<summary>Original dictation</summary>\n\n{fence}text\n{transcript}\n{fence}\n\n</details>\n"
        ));
    }
    out
}

/// The folder reports are saved to: the user's choice, else
/// `Documents/Handy Bug Reports`.
pub fn reports_dir(app: &AppHandle, settings: &AppSettings) -> anyhow::Result<PathBuf> {
    if let Some(dir) = settings
        .bug_report_dir
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        return Ok(PathBuf::from(dir));
    }
    let docs = app
        .path()
        .document_dir()
        .context("couldn't find the Documents folder")?;
    Ok(docs.join(DEFAULT_DIR_NAME))
}

/// `<dir>/<timestamp>[_<slug>][-N].md`, never an existing file.
fn unique_path(dir: &Path, created: DateTime<Local>, title: &str) -> PathBuf {
    let stamp = created.format("%Y-%m-%d_%H%M%S");
    let slug = slug(title);
    let stem = if slug.is_empty() {
        stamp.to_string()
    } else {
        format!("{stamp}_{slug}")
    };
    let mut path = dir.join(format!("{stem}.md"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}.md"));
        n += 1;
    }
    path
}

/// Writes the report and returns the file's path. Blocking: call from a
/// blocking context.
pub fn save(
    dir: &Path,
    report: &BugReport,
    transcript: &str,
    environment: &str,
) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("couldn't create {}", dir.display()))?;
    let created = Local::now();
    let path = unique_path(dir, created, &report.title);
    let markdown = render_markdown(report, transcript, created, environment);
    std::fs::write(&path, markdown)
        .with_context(|| format!("couldn't write {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn when() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, 25, 14, 32, 5).unwrap()
    }

    #[test]
    fn slug_keeps_ascii_words_and_collapses_the_rest() {
        assert_eq!(slug("App crashes on  save!"), "app-crashes-on-save");
        assert_eq!(slug("  --Weird__title--  "), "weird-title");
        assert_eq!(slug("崩溃"), "");
        assert!(slug(&"word ".repeat(40)).len() <= MAX_SLUG_CHARS);
    }

    #[test]
    fn parse_report_accepts_fenced_json_and_partial_objects() {
        let reply = "```json\n{\"title\":\"Crash\",\"steps\":[\"Open app\"]}\n```";
        let report = parse_report(reply).unwrap();
        assert_eq!(report.title, "Crash");
        assert_eq!(report.steps, vec!["Open app"]);
        assert_eq!(report.expected, "");
    }

    #[test]
    fn parse_report_rejects_garbage_and_empty_objects() {
        assert!(parse_report("sorry, I can't").is_none());
        assert!(parse_report("{\"title\":\"\",\"steps\":[]}").is_none());
        assert!(parse_report("{not json}").is_none());
    }

    #[test]
    fn raw_fallback_keeps_the_transcript_and_titles_it() {
        let report = BugReport::from_raw("  the overlay flickers. Steps: stop recording quickly ");
        assert_eq!(report.title, "the overlay flickers");
        assert_eq!(
            report.notes,
            "the overlay flickers. Steps: stop recording quickly"
        );
        assert_eq!(BugReport::from_raw("   ").title, "Bug report");
        assert_eq!(BugReport::from_raw("...").title, "Bug report");
    }

    #[test]
    fn fallback_title_never_ends_on_a_comma_or_runs_long() {
        assert_eq!(
            fallback_title(
                "Приложение не запускается, когда ты пытаешься его запустить, подается ошибка при запуске"
            ),
            "Приложение не запускается, когда ты пытаешься его запустить, подается ошибка"
        );
        assert_eq!(
            fallback_title("one two three four five six seven eight nine ten eleven twelve"),
            "one two three four five six seven eight nine ten"
        );
        assert_eq!(
            fallback_title("It crashes on save, then"),
            "It crashes on save, then"
        );
        assert_eq!(
            fallback_title("Crash on save,\nmore details"),
            "Crash on save"
        );
    }

    #[test]
    fn markdown_skips_empty_sections_and_numbers_steps() {
        let report = BugReport {
            title: "Crash\non save".into(),
            steps: vec!["Open a file".into(), "".into(), "Press save".into()],
            actual: "It crashes".into(),
            ..BugReport::default()
        };
        let md = render_markdown(&report, "raw words", when(), "Handy 1.0 (windows)");
        assert!(md.starts_with("# Crash on save\n"));
        assert!(md.contains("- **Reported:** 2026-11-25 14:32"));
        assert!(md.contains("1. Open a file\n2. Press save"));
        assert!(md.contains("## Actual behavior\n\nIt crashes"));
        assert!(!md.contains("## Summary"));
        assert!(!md.contains("## Expected behavior"));
        assert!(md.contains("```text\nraw words\n```"));
    }

    #[test]
    fn dictation_fence_outgrows_backticks_in_the_transcript() {
        let md = render_markdown(&BugReport::default(), "see ```code``` here", when(), "env");
        assert!(md.contains("````text\nsee ```code``` here\n````"));
    }

    #[test]
    fn save_never_overwrites_an_existing_report() {
        let dir = tempfile::tempdir().unwrap();
        let report = BugReport::from_raw("same title every time");
        let first = unique_path(dir.path(), when(), &report.title);
        std::fs::write(&first, "x").unwrap();
        let second = unique_path(dir.path(), when(), &report.title);
        assert_ne!(first, second);
        assert!(second.to_string_lossy().ends_with("-2.md"));

        let saved = save(&dir.path().join("nested"), &report, "t", "env").unwrap();
        assert!(std::fs::read_to_string(saved)
            .unwrap()
            .starts_with("# same title"));
    }
}
