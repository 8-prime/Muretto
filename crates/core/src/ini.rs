//! The game's INI dialect: `[SECTION]` headers, `KEY=VALUE` lines, and
//! comments introduced by `;` or `//` anywhere on a line. Section and key
//! order is preserved so written files stay readable and diffable.

use std::fmt::{self, Display, Write as _};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Ini {
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl Ini {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn parse(text: &str) -> Self {
        let mut ini = Ini::new();
        let mut current: Option<usize> = None;
        for line in text.lines() {
            let line = strip_comment(line).trim();
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                current = Some(ini.section_index(name.trim()));
            } else if let (Some(index), Some((key, value))) = (current, line.split_once('=')) {
                ini.sections[index]
                    .1
                    .push((key.trim().to_owned(), value.trim().to_owned()));
            }
        }
        ini
    }

    fn section_index(&mut self, name: &str) -> usize {
        if let Some(index) = self.sections.iter().position(|(n, _)| n == name) {
            return index;
        }
        self.sections.push((name.to_owned(), Vec::new()));
        self.sections.len() - 1
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .iter()
            .find(|(n, _)| n == section)?
            .1
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn get_number(&self, section: &str, key: &str) -> Option<f64> {
        self.get(section, key)?.parse().ok()
    }

    pub fn set(&mut self, section: &str, key: &str, value: impl Display) -> &mut Self {
        let index = self.section_index(section);
        let entries = &mut self.sections[index].1;
        let value = value.to_string();
        match entries.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = value,
            None => entries.push((key.to_owned(), value)),
        }
        self
    }

    pub fn section_names(&self) -> impl Iterator<Item = &str> {
        self.sections.iter().map(|(n, _)| n.as_str())
    }
}

impl Display for Ini {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, (name, entries)) in self.sections.iter().enumerate() {
            if index > 0 {
                f.write_char('\n')?;
            }
            writeln!(f, "[{name}]")?;
            for (key, value) in entries {
                writeln!(f, "{key}={value}")?;
            }
        }
        Ok(())
    }
}

fn strip_comment(line: &str) -> &str {
    let end = [line.find(';'), line.find("//")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(line.len());
    &line[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_whitespace_are_ignored() {
        let ini = Ini::parse(
            "[LAUNCHER]\nNAME=Light Clouds ; shown in menus\nTEMPERATURE_COEFF = -0.3 // range -1..1\n",
        );
        assert_eq!(ini.get("LAUNCHER", "NAME"), Some("Light Clouds"));
        assert_eq!(ini.get_number("LAUNCHER", "TEMPERATURE_COEFF"), Some(-0.3));
    }

    #[test]
    fn written_files_read_back_in_order() {
        let mut ini = Ini::new();
        ini.set("RACE", "TRACK", "spa").set("RACE", "CARS", 24);
        ini.set("CAR_0", "MODEL", "-");
        ini.set("RACE", "CARS", 12);
        let text = ini.to_string();
        assert_eq!(text, "[RACE]\nTRACK=spa\nCARS=12\n\n[CAR_0]\nMODEL=-\n");
        assert_eq!(Ini::parse(&text), ini);
    }
}
