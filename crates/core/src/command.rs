//! Commands are declared once. Discord registers them as application commands;
//! Matrix and Stoat (and Telegram's arguments) parse text with `parse_text`,
//! which lives here because three platforms share it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub name: String,
    pub summary: String,
    pub args: Vec<ArgSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArgSpec {
    pub name: String,
    pub kind: ArgKind,
    pub required: bool,
    /// Only Discord offers suggestions while typing; elsewhere this is ignored.
    pub autocomplete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ArgKind {
    /// Takes the rest of the line when it is the last argument.
    Text,
    Integer { min: i64, max: i64 },
    Choice { options: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgValue {
    Text(String),
    Integer(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub command: String,
    pub args: Vec<(String, ArgValue)>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("not a command")]
    NotACommand,
    #[error("unknown command {0}")]
    Unknown(String),
    #[error("missing {0}")]
    Missing(String),
    #[error("{arg}: {why}")]
    Invalid { arg: String, why: String },
}

/// Parse `<prefix><name> args…` against the specs.
pub fn parse_text(specs: &[CommandSpec], prefix: &str, line: &str) -> Result<Parsed, ParseError> {
    let rest = line.trim().strip_prefix(prefix).ok_or(ParseError::NotACommand)?;
    let (name, mut rest) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let spec = specs
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| ParseError::Unknown(name.to_string()))?;
    let mut args = Vec::new();
    for (i, a) in spec.args.iter().enumerate() {
        rest = rest.trim_start();
        let last = i + 1 == spec.args.len();
        let token = if last && a.kind == ArgKind::Text {
            std::mem::take(&mut rest).trim_end()
        } else {
            let (t, r) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            rest = r;
            t
        };
        if token.is_empty() {
            if a.required {
                return Err(ParseError::Missing(a.name.clone()));
            }
            continue;
        }
        let value = match &a.kind {
            ArgKind::Text => ArgValue::Text(token.to_string()),
            ArgKind::Integer { min, max } => {
                let n: i64 = token.parse().map_err(|_| ParseError::Invalid {
                    arg: a.name.clone(),
                    why: "not a number".into(),
                })?;
                if n < *min || n > *max {
                    return Err(ParseError::Invalid {
                        arg: a.name.clone(),
                        why: format!("must be {min} to {max}"),
                    });
                }
                ArgValue::Integer(n)
            }
            ArgKind::Choice { options } => {
                let hit = options.iter().find(|o| o.eq_ignore_ascii_case(token)).ok_or_else(|| {
                    ParseError::Invalid {
                        arg: a.name.clone(),
                        why: format!("one of {}", options.join(", ")),
                    }
                })?;
                ArgValue::Text(hit.clone())
            }
        };
        args.push((a.name.clone(), value));
    }
    Ok(Parsed {
        command: spec.name.clone(),
        args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs() -> Vec<CommandSpec> {
        vec![
            CommandSpec {
                name: "play".into(),
                summary: "Play a song".into(),
                args: vec![ArgSpec {
                    name: "query".into(),
                    kind: ArgKind::Text,
                    required: true,
                    autocomplete: true,
                }],
            },
            CommandSpec {
                name: "volume".into(),
                summary: "Set the volume".into(),
                args: vec![ArgSpec {
                    name: "percent".into(),
                    kind: ArgKind::Integer { min: 0, max: 200 },
                    required: true,
                    autocomplete: false,
                }],
            },
        ]
    }

    #[test]
    fn the_last_text_argument_takes_the_rest_of_the_line() {
        let p = parse_text(&specs(), "!", "!play  never gonna give you up ").unwrap();
        assert_eq!(p.args, vec![("query".into(), ArgValue::Text("never gonna give you up".into()))]);
    }

    #[test]
    fn ranges_and_missing_arguments_are_reported() {
        assert!(matches!(
            parse_text(&specs(), "!", "!volume 500"),
            Err(ParseError::Invalid { .. })
        ));
        assert_eq!(
            parse_text(&specs(), "!", "!play"),
            Err(ParseError::Missing("query".into()))
        );
        assert_eq!(parse_text(&specs(), "!", "hello"), Err(ParseError::NotACommand));
    }
}
