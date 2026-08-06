//! The benchmark loop: generate → check → feed diagnostics back →
//! retry → simulate → feed scenario failures back → retry, bounded.
//! Authors are pluggable behind one seam; the dry `canned` author
//! replays recorded attempts, so the whole loop runs and measures
//! without any model spend.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const USAGE: &str = "usage: vibe-runner --tasks <dir> --author <canned:<dir> | command:<template>> \
     --rastok <bin> --prompt <file> --out <dir> [--max-attempts <n>]";

/// One authoring seam: given the full prompt and the feedback so
/// far, produce the next attempt's source, or nothing when the
/// author has no more to offer.
trait Author {
    fn attempt(&mut self, task_id: &str, prompt: &str, feedback: &[String]) -> Option<String>;
}

/// Replays pre-recorded attempts from
/// `<dir>/<task-id>/attempt-<n>.rkg` — the dry mode.
struct Canned {
    root: PathBuf,
}

impl Author for Canned {
    fn attempt(&mut self, task_id: &str, _prompt: &str, feedback: &[String]) -> Option<String> {
        let next = feedback.len() + 1;
        std::fs::read_to_string(self.root.join(task_id).join(format!("attempt-{next}.rkg"))).ok()
    }
}

/// Pipes the prompt (and any feedback appended) to a shell command
/// and reads the attempt from its stdout — the live mode; the
/// template is the model CLI invocation.
struct Shell {
    template: String,
}

impl Author for Shell {
    fn attempt(&mut self, _task_id: &str, prompt: &str, feedback: &[String]) -> Option<String> {
        let mut full = String::from(prompt);
        for (index, item) in feedback.iter().enumerate() {
            let _ = write!(
                full,
                "\n\n## Feedback on attempt {}\n\nYour previous submission was refused:\n\n{}\n\
                 Submit a corrected full file.",
                index + 1,
                item
            );
        }
        let output = Command::new("sh")
            .arg("-c")
            .arg(&self.template)
            .env("VIBE_PROMPT", &full)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// Models often wrap sources in markdown fences despite the
/// instructions; stripping them is measurement hygiene, not mercy —
/// the measured skill is the language, not the chat etiquette.
fn strip_fences(source: &str) -> String {
    let trimmed = source.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed.to_string();
    };
    let body = rest.split_once('\n').map_or("", |(_, body)| body);
    body.rsplit_once("```").map_or(body, |(kept, _)| kept).trim().to_string()
}

/// Every `error[E-...]` code in a diagnostic stream, deduplicated in
/// first-seen order — the recurring-families metric.
fn diagnostic_codes(stderr: &str) -> Vec<String> {
    let mut codes = Vec::new();
    for part in stderr.split("error[").skip(1) {
        if let Some(code) = part.split(']').next() {
            let code = code.to_string();
            if code.starts_with("E-") && !codes.contains(&code) {
                codes.push(code);
            }
        }
    }
    codes
}

fn json_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

struct TaskResult {
    task: String,
    attempts: usize,
    first_try_check: bool,
    first_try_scenarios: bool,
    outcome: &'static str,
    codes: Vec<String>,
}

impl TaskResult {
    fn to_json(&self, author: &str, prompt_version: &str) -> String {
        let codes = self
            .codes
            .iter()
            .map(|code| json_text(code))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"task\":{},\"author\":{},\"prompt\":{},\"attempts\":{},\
             \"first_try_check\":{},\"first_try_scenarios\":{},\"outcome\":{},\
             \"codes\":[{}],\"human_minutes\":null}}",
            json_text(&self.task),
            json_text(author),
            json_text(prompt_version),
            self.attempts,
            self.first_try_check,
            self.first_try_scenarios,
            json_text(self.outcome),
            codes,
        )
    }
}

fn run_task(
    rastok: &str,
    task_dir: &Path,
    prompt_card: &str,
    author: &mut dyn Author,
    work_dir: &Path,
    max_attempts: usize,
) -> Option<TaskResult> {
    let task_id = task_dir.file_name()?.to_string_lossy().into_owned();
    let statement = std::fs::read_to_string(task_dir.join("task.md")).ok()?;
    let referee = task_dir.join("referee.rkg");
    let prompt = format!("{prompt_card}\n\n---\n\n{statement}");
    std::fs::create_dir_all(work_dir).ok()?;

    let mut feedback: Vec<String> = Vec::new();
    let mut codes: Vec<String> = Vec::new();
    let mut first_try_check = false;
    let mut first_try_scenarios = false;
    while feedback.len() < max_attempts {
        let attempt_index = feedback.len() + 1;
        let Some(raw) = author.attempt(&task_id, &prompt, &feedback) else {
            return Some(TaskResult {
                task: task_id,
                attempts: feedback.len(),
                first_try_check,
                first_try_scenarios,
                outcome: "exhausted",
                codes,
            });
        };
        let source = strip_fences(&raw);
        let solution = work_dir.join(format!("attempt-{attempt_index}.rkg"));
        std::fs::write(&solution, &source).ok()?;

        let check = Command::new(rastok).arg("check").arg(&solution).output().ok()?;
        let check_err = String::from_utf8_lossy(&check.stderr).into_owned();
        if !check.status.success() {
            for code in diagnostic_codes(&check_err) {
                if !codes.contains(&code) {
                    codes.push(code);
                }
            }
            let feedback_path = work_dir.join(format!("feedback-{attempt_index}.txt"));
            std::fs::write(&feedback_path, &check_err).ok()?;
            feedback.push(check_err);
            continue;
        }
        if attempt_index == 1 {
            first_try_check = true;
        }

        let sim = Command::new(rastok)
            .arg("simulate")
            .arg(&solution)
            .arg(&referee)
            .output()
            .ok()?;
        let sim_text = format!(
            "{}{}",
            String::from_utf8_lossy(&sim.stdout),
            String::from_utf8_lossy(&sim.stderr)
        );
        if !sim.status.success() {
            for code in diagnostic_codes(&sim_text) {
                if !codes.contains(&code) {
                    codes.push(code);
                }
            }
            let feedback_path = work_dir.join(format!("feedback-{attempt_index}.txt"));
            std::fs::write(&feedback_path, &sim_text).ok()?;
            feedback.push(sim_text);
            continue;
        }
        if attempt_index == 1 {
            first_try_scenarios = true;
        }
        return Some(TaskResult {
            task: task_id,
            attempts: attempt_index,
            first_try_check,
            first_try_scenarios,
            outcome: "passed",
            codes,
        });
    }
    Some(TaskResult {
        task: task_id,
        attempts: max_attempts,
        first_try_check,
        first_try_scenarios,
        outcome: "failed",
        codes,
    })
}

fn main() -> std::process::ExitCode {
    let mut tasks_dir = None;
    let mut author_spec = None;
    let mut rastok = None;
    let mut prompt_path = None;
    let mut out_dir = None;
    let mut max_attempts = 4usize;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let value = it.next();
        match (flag.as_str(), value) {
            ("--tasks", Some(v)) => tasks_dir = Some(PathBuf::from(v)),
            ("--author", Some(v)) => author_spec = Some(v.clone()),
            ("--rastok", Some(v)) => rastok = Some(v.clone()),
            ("--prompt", Some(v)) => prompt_path = Some(PathBuf::from(v)),
            ("--out", Some(v)) => out_dir = Some(PathBuf::from(v)),
            ("--max-attempts", Some(v)) => match v.parse() {
                Ok(n) => max_attempts = n,
                Err(_) => {
                    eprintln!("{USAGE}");
                    return std::process::ExitCode::from(2);
                }
            },
            _ => {
                eprintln!("{USAGE}");
                return std::process::ExitCode::from(2);
            }
        }
    }
    let (Some(tasks_dir), Some(author_spec), Some(rastok), Some(prompt_path), Some(out_dir)) =
        (tasks_dir, author_spec, rastok, prompt_path, out_dir)
    else {
        eprintln!("{USAGE}");
        return std::process::ExitCode::from(2);
    };

    let Ok(prompt_card) = std::fs::read_to_string(&prompt_path) else {
        eprintln!("cannot read the prompt card {}", prompt_path.display());
        return std::process::ExitCode::from(2);
    };
    let prompt_version = prompt_path
        .file_stem()
        .map_or_else(|| String::from("unknown"), |stem| stem.to_string_lossy().into_owned());
    let (author_name, mut author): (String, Box<dyn Author>) = match author_spec.split_once(':') {
        Some(("canned", dir)) => (
            format!("canned:{dir}"),
            Box::new(Canned {
                root: PathBuf::from(dir),
            }),
        ),
        Some(("command", template)) => (
            String::from("command"),
            Box::new(Shell {
                template: template.to_string(),
            }),
        ),
        _ => {
            eprintln!("{USAGE}");
            return std::process::ExitCode::from(2);
        }
    };

    let mut task_dirs: Vec<PathBuf> = match std::fs::read_dir(&tasks_dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.join("task.md").is_file())
            .collect(),
        Err(error) => {
            eprintln!("cannot list {}: {error}", tasks_dir.display());
            return std::process::ExitCode::from(2);
        }
    };
    task_dirs.sort();

    let mut rows = Vec::new();
    let mut skipped = 0usize;
    for task_dir in &task_dirs {
        let task_id = task_dir.file_name().map_or_else(
            || String::from("?"),
            |name| name.to_string_lossy().into_owned(),
        );
        let work_dir = out_dir.join(&author_name.replace([':', '/'], "_")).join(&task_id);
        match run_task(&rastok, task_dir, &prompt_card, author.as_mut(), &work_dir, max_attempts) {
            Some(result) if result.outcome == "exhausted" && result.attempts == 0 => {
                // The author had nothing for this task (a partial
                // canned set): skipped, and said so — never counted
                // as covered.
                skipped += 1;
            }
            Some(result) => {
                println!(
                    "{}: {} in {} attempt(s)",
                    result.task, result.outcome, result.attempts
                );
                rows.push(result.to_json(&author_name, &prompt_version));
            }
            None => {
                eprintln!("{task_id}: the loop itself failed (io); aborting");
                return std::process::ExitCode::from(1);
            }
        }
    }
    if skipped > 0 {
        println!("{skipped} task(s) skipped: the author offered no attempt");
    }

    if let Err(error) = std::fs::create_dir_all(&out_dir) {
        eprintln!("cannot create {}: {error}", out_dir.display());
        return std::process::ExitCode::from(1);
    }
    let metrics = out_dir.join("metrics.jsonl");
    let mut body = rows.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(&metrics) {
        Ok(mut file) => {
            use std::io::Write as _;
            if let Err(error) = file.write_all(body.as_bytes()) {
                eprintln!("cannot append to {}: {error}", metrics.display());
                return std::process::ExitCode::from(1);
            }
        }
        Err(error) => {
            eprintln!("cannot open {}: {error}", metrics.display());
            return std::process::ExitCode::from(1);
        }
    }
    println!("{} row(s) appended to {}", rows.len(), metrics.display());
    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fences_are_stripped_whatever_the_wrapping() {
        assert_eq!(strip_fences("graph x"), "graph x");
        assert_eq!(strip_fences("```rkg\ngraph x\n```"), "graph x");
        assert_eq!(strip_fences("```\ngraph x\n```\n"), "graph x");
    }

    #[test]
    fn codes_deduplicate_in_first_seen_order() {
        let stderr = "error[E-PARSE-EXPECTED]: a\nerror[E-CEL-EXPR]: b\nerror[E-PARSE-EXPECTED]: c";
        assert_eq!(diagnostic_codes(stderr), vec!["E-PARSE-EXPECTED", "E-CEL-EXPR"]);
    }

    #[test]
    fn json_rows_escape_their_text() {
        assert_eq!(json_text("a\"b\nc"), "\"a\\\"b\\nc\"");
    }
}
