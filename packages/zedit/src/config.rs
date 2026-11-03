#![allow(dead_code)]
use alloc::{string::String, vec::Vec};

#[derive(Debug, PartialEq)]
pub enum LineKind {
    Blank,
    Comment,
    Section(String),
    Pair { key: String, value: String },
    Invalid,
}

pub fn parse_line(line: &str) -> LineKind {
    let t = line.trim();
    if t.is_empty() { return LineKind::Blank; }
    if t.starts_with('#') || t.starts_with(';') { return LineKind::Comment; }

    if let Some(rest) = t.strip_prefix('[') {
        return match rest.strip_suffix(']') {
            Some(name) if !name.trim().is_empty() => LineKind::Section(name.trim().into()),
            _ => LineKind::Invalid,
        };
    }

    match t.split_once('=') {
        Some((k, v)) if !k.trim().is_empty() => LineKind::Pair {
            key: k.trim().into(),
            value: v.trim().trim_matches('"').into(),
        },
        _ => LineKind::Invalid,
    }
}

pub struct Config {
    entries: Vec<(String, String, String)>, // (section, key, value)
}

impl Config {
    pub fn parse(text: &str) -> Self {
        let mut entries = Vec::new();
        let mut section = String::new();
        for line in text.lines() {
            match parse_line(line) {
                LineKind::Section(s) => section = s,
                LineKind::Pair { key, value } => entries.push((section.clone(), key, value)),
                _ => {}
            }
        }
        Config { entries }
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.entries.iter().rev() // last definition wins
            .find(|(s, k, _)| s == section && k == key)
            .map(|(_, _, v)| v.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_basic() {
        let c = Config::parse("# hi\n[editor]\ntab_width = 4\nname = \"zed\"\n");
        assert_eq!(c.get("editor", "tab_width"), Some("4"));
        assert_eq!(c.get("editor", "name"), Some("zed"));
        assert_eq!(parse_line("oops"), LineKind::Invalid);
    }
}