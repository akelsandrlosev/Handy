//! Structures a dictated bug report with no AI: the speaker says section
//! words ("steps", "expected result", "actual result", ...) and the text is
//! split at them. Russian and English.

use crate::bug_report::{fallback_title, BugReport};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Title,
    Summary,
    Steps,
    Expected,
    Actual,
    Notes,
}

/// Words that start a section when spoken at the start of a sentence.
/// Lowercase, with "е" standing in for "ё". The longest match wins.
const SECTION_MARKERS: &[(Section, &str)] = &[
    (Section::Title, "заголовок"),
    (Section::Title, "название"),
    (Section::Title, "title"),
    (Section::Summary, "краткое описание"),
    (Section::Summary, "описание"),
    (Section::Summary, "summary"),
    (Section::Summary, "description"),
    (Section::Steps, "шаги воспроизведения"),
    (Section::Steps, "шаги для воспроизведения"),
    (Section::Steps, "шаги повторения"),
    (Section::Steps, "как воспроизвести"),
    (Section::Steps, "шаги"),
    (Section::Steps, "steps to reproduce"),
    (Section::Steps, "reproduction steps"),
    (Section::Steps, "repro steps"),
    (Section::Steps, "steps"),
    (Section::Expected, "ожидаемый результат"),
    (Section::Expected, "ожидаемое поведение"),
    (Section::Expected, "ожидалось"),
    (Section::Expected, "expected result"),
    (Section::Expected, "expected behavior"),
    (Section::Expected, "expected behaviour"),
    (Section::Expected, "expected"),
    (Section::Actual, "фактический результат"),
    (Section::Actual, "фактическое поведение"),
    (Section::Actual, "реальный результат"),
    (Section::Actual, "актуальный результат"),
    (Section::Actual, "actual result"),
    (Section::Actual, "actual behavior"),
    (Section::Actual, "actual behaviour"),
    (Section::Actual, "actual"),
    (Section::Notes, "примечания"),
    (Section::Notes, "примечание"),
    (Section::Notes, "заметки"),
    (Section::Notes, "дополнительно"),
    (Section::Notes, "notes"),
    (Section::Notes, "additional info"),
];

/// Words of an opening sentence that only announce a bug report ("Test bug
/// report."), which makes a poor title.
const ANNOUNCEMENT_WORDS: &[&str] = &[
    "тестовый",
    "баг",
    "репорт",
    "баг-репорт",
    "отчет",
    "об",
    "ошибке",
    "новый",
    "bug",
    "report",
    "test",
    "new",
];

fn fold(c: char) -> char {
    let c = c.to_lowercase().next().unwrap_or(c);
    if c == 'ё' {
        'е'
    } else {
        c
    }
}

/// Characters that end a sentence; a section word only counts right after one.
fn ends_sentence(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '\n' | ':' | ';' | '。' | '！' | '？')
}

/// `(section, start of marker, start of its content)` for every section word
/// at the start of the text or of a sentence.
fn find_markers(chars: &[char]) -> Vec<(Section, usize, usize)> {
    let folded: Vec<char> = chars.iter().map(|&c| fold(c)).collect();
    let mut markers = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let at_sentence_start = chars[..i]
            .iter()
            .rev()
            .find(|c| !c.is_whitespace())
            .is_none_or(|&c| ends_sentence(c));
        let hit = if at_sentence_start {
            SECTION_MARKERS
                .iter()
                .filter(|(_, phrase)| {
                    let p: Vec<char> = phrase.chars().collect();
                    folded.get(i..i + p.len()) == Some(&p[..])
                        && folded
                            .get(i + p.len())
                            .is_none_or(|c| !c.is_alphanumeric() && *c != '-')
                })
                .max_by_key(|(_, phrase)| phrase.chars().count())
        } else {
            None
        };
        match hit {
            Some(&(section, phrase)) => {
                let mut content = i + phrase.chars().count();
                while content < chars.len()
                    && (chars[content].is_whitespace()
                        || matches!(chars[content], ':' | '.' | ',' | ';' | '-' | '—' | '–'))
                {
                    content += 1;
                }
                markers.push((section, i, content));
                i = content.max(i + 1);
            }
            None => i += 1,
        }
    }
    markers
}

fn clean(text: &str) -> String {
    text.trim()
        .trim_end_matches(['.', ',', ';', ':', ' '])
        .trim()
        .to_string()
}

/// Splits spoken steps at sentence ends; bare numbers ("1.") are dropped.
fn split_steps(text: &str) -> Vec<String> {
    text.split(['.', '!', '?', ';', '\n', '。'])
        .map(clean)
        .filter(|s| !s.is_empty() && !s.chars().all(|c| c.is_ascii_digit()))
        .collect()
}

fn is_announcement(sentence: &str) -> bool {
    let mut words = sentence
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-'))
        .map(|w| w.chars().map(fold).collect::<String>())
        .peekable();
    words.peek().is_some() && words.all(|w| ANNOUNCEMENT_WORDS.contains(&w.as_str()))
}

fn join(existing: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    if !existing.is_empty() {
        existing.push(' ');
    }
    existing.push_str(text);
}

/// Builds a report from the section words the speaker said. Text before the
/// first section word becomes the title and summary. `None` when no section
/// word was said, so the caller can fall back to the raw transcript.
pub fn parse_spoken(transcript: &str) -> Option<BugReport> {
    let chars: Vec<char> = transcript.chars().collect();
    let markers = find_markers(&chars);
    let first_start = markers.first()?.1;

    let slice = |from: usize, to: usize| chars[from..to].iter().collect::<String>();
    let mut report = BugReport::default();
    for (n, &(section, _, content)) in markers.iter().enumerate() {
        let end = markers.get(n + 1).map_or(chars.len(), |next| next.1);
        let text = slice(content, end);
        match section {
            Section::Title => {
                // One sentence is the title; anything after it is summary.
                let mut sentences = text
                    .split(['.', '!', '?', '\n', '。'])
                    .map(clean)
                    .filter(|s| !s.is_empty());
                join(&mut report.title, &sentences.next().unwrap_or_default());
                let rest: Vec<String> = sentences.collect();
                join(&mut report.summary, &rest.join(". "));
            }
            Section::Summary => join(&mut report.summary, &clean(&text)),
            Section::Steps => report.steps.extend(split_steps(&text)),
            Section::Expected => join(&mut report.expected, &clean(&text)),
            Section::Actual => join(&mut report.actual, &clean(&text)),
            Section::Notes => join(&mut report.notes, &clean(&text)),
        }
    }

    // Whatever came before the first section word: the title (unless one was
    // said) and the rest as summary.
    let preamble = slice(0, first_start);
    let mut sentences = preamble
        .split(['.', '!', '?', '\n', '。'])
        .map(str::trim)
        .filter(|s| !s.is_empty() && !is_announcement(s));
    if report.title.is_empty() {
        if let Some(title) = sentences.next() {
            report.title = clean(title);
        }
    }
    let rest: Vec<&str> = sentences.collect();
    join(&mut report.summary, &rest.join(". "));

    if report.title.is_empty() {
        let seed = [&report.actual, &report.summary, &report.expected]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .or_else(|| report.steps.first().cloned())
            .unwrap_or_default();
        report.title = fallback_title(&seed);
    }
    Some(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_russian_report() {
        let report = parse_spoken(
            "Тестовый баг репорт. Приложение не запускается. Шаги повторения. Открыть папку приложения, найти Exe файл, кликнуть по нему два раза. Фактический результат приложения выдает ошибку при запуске. Ожидаемый результат. Приложение запускается.",
        )
        .unwrap();
        assert_eq!(report.title, "Приложение не запускается");
        assert_eq!(report.summary, "");
        assert_eq!(
            report.steps,
            vec!["Открыть папку приложения, найти Exe файл, кликнуть по нему два раза"]
        );
        assert_eq!(report.actual, "приложения выдает ошибку при запуске");
        assert_eq!(report.expected, "Приложение запускается");
    }

    #[test]
    fn splits_an_english_report_with_a_spoken_title() {
        let report = parse_spoken(
            "Title: Crash on save. It happens a lot. Steps: Open a file. Press save. Expected: it saves. Actual: it crashes. Notes: only on Windows.",
        )
        .unwrap();
        assert_eq!(report.title, "Crash on save");
        assert_eq!(report.summary, "It happens a lot");
        assert_eq!(report.steps, vec!["Open a file", "Press save"]);
        assert_eq!(report.expected, "it saves");
        assert_eq!(report.actual, "it crashes");
        assert_eq!(report.notes, "only on Windows");
    }

    #[test]
    fn section_words_inside_a_sentence_are_not_markers() {
        assert!(parse_spoken("The app crashes and the steps are unclear, expected more").is_none());
        assert!(parse_spoken("Приложение падает, а шаги непонятны").is_none());
        assert!(parse_spoken("").is_none());
    }

    #[test]
    fn a_report_with_no_opening_text_borrows_its_title() {
        let report =
            parse_spoken("Actual result: the overlay flickers. Expected result: it stays still.")
                .unwrap();
        assert_eq!(report.title, "the overlay flickers");
        assert_eq!(report.expected, "it stays still");
    }

    #[test]
    fn repeated_sections_are_joined() {
        let report = parse_spoken("Steps. Open it. Notes. One. Notes. Two.").unwrap();
        assert_eq!(report.notes, "One Two");
    }
}
