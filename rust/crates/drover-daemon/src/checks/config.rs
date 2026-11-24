//! `.drover/checks.toml`: `[[step]]` tables with `name`, `run` (argv array or a shell string) and `timeout_s`.

use std::path::Path;

pub const DEFAULT_TIMEOUT_S: u64 = 600;
const MAX_TIMEOUT_S: u64 = 3600;
const MAX_STEPS: usize = 50;

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub name: String,
    /// argv, or a single shell string when `shell` is set (shown to the user as-is).
    pub command: Vec<String>,
    pub shell: bool,
    pub timeout_s: u64,
}

pub fn parse(text: &str) -> Result<Vec<Step>, String> {
    let doc: toml::Table = text.parse().map_err(|e| format!("invalid checks.toml: {e}"))?;
    let Some(list) = doc.get("step") else { return Ok(vec![]) };
    let list = list.as_array().ok_or("checks.toml: `step` must be an array of tables ([[step]])")?;
    if list.len() > MAX_STEPS {
        return Err(format!("checks.toml: more than {MAX_STEPS} steps"));
    }
    let mut steps: Vec<Step> = vec![];
    for (i, item) in list.iter().enumerate() {
        let at = |m: &str| format!("checks.toml: step {}: {m}", i + 1);
        let t = item.as_table().ok_or_else(|| at("must be a table"))?;
        let (command, shell) = match t.get("run") {
            Some(toml::Value::String(s)) if !s.trim().is_empty() => (vec![s.trim().to_string()], true),
            Some(toml::Value::Array(a)) if !a.is_empty() => {
                let argv: Option<Vec<String>> = a.iter().map(|v| v.as_str().map(String::from)).collect();
                match argv {
                    Some(v) if !v[0].trim().is_empty() => (v, false),
                    _ => return Err(at("`run` must be a list of strings")),
                }
            }
            _ => return Err(at("`run` is missing (a list like [\"cargo\", \"test\"] or a string)")),
        };
        let name = match t.get("name") {
            Some(toml::Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
            None => command[0].split_whitespace().next().unwrap_or("step").to_string(),
            _ => return Err(at("`name` must be a non-empty string")),
        };
        if steps.iter().any(|s| s.name == name) {
            return Err(at(&format!("duplicate name '{name}'")));
        }
        let timeout_s = match t.get("timeout_s") {
            None => DEFAULT_TIMEOUT_S,
            Some(v) => v.as_integer().filter(|n| *n > 0).ok_or_else(|| at("`timeout_s` must be a positive integer"))?.unsigned_abs().min(MAX_TIMEOUT_S),
        };
        steps.push(Step { name, command, shell, timeout_s });
    }
    Ok(steps)
}

/// Steps from `<dir>/.drover/checks.toml`; Ok(None) when the file does not exist.
pub fn load(dir: &Path) -> Result<Option<Vec<Step>>, String> {
    let file = dir.join(".drover").join("checks.toml");
    match std::fs::read_to_string(&file) {
        Ok(text) => parse(&text).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", file.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_argv_and_shell_steps() {
        let s = parse("[[step]]\nname = \"test\"\nrun = [\"cargo\", \"test\"]\ntimeout_s = 30\n\n[[step]]\nrun = \"npm run lint\"\n").unwrap();
        assert_eq!(s[0], Step { name: "test".into(), command: vec!["cargo".into(), "test".into()], shell: false, timeout_s: 30 });
        assert_eq!(s[1], Step { name: "npm".into(), command: vec!["npm run lint".into()], shell: true, timeout_s: DEFAULT_TIMEOUT_S });
    }

    #[test]
    fn empty_and_bad_files() {
        assert!(parse("").unwrap().is_empty());
        assert!(parse("[[step]]\nname = \"x\"\n").is_err());
        assert!(parse("[[step]]\nrun = [\"a\", 1]\n").is_err());
        assert!(parse("[[step]]\nrun = \"a\"\ntimeout_s = -1\n").is_err());
        assert!(parse("[[step]]\nname=\"a\"\nrun=\"x\"\n[[step]]\nname=\"a\"\nrun=\"y\"\n").is_err());
        assert!(parse("not toml [").is_err());
        assert_eq!(parse("[[step]]\nrun=\"x\"\ntimeout_s = 999999\n").unwrap()[0].timeout_s, MAX_TIMEOUT_S);
    }
}
