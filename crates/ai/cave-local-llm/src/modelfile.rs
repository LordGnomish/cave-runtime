// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pure-Rust Modelfile parser.
//!
//! Cite ollama/ollama `parser/parser.go` — the `File`/`Command` structs, the
//! recognised command keywords, the `"""…"""` multiline literal, comment
//! handling, and the `errMissingFrom` / `errInvalidCommand` /
//! `errInvalidMessageRole` validation. This is the *parsing* half of the
//! upstream `/api/create` flow; writing the resulting blobs + manifest into a
//! model store stays an explicit scope-cut (owned by the registry/runtime).
//!
//! The parser is line-oriented but understands triple-quoted values that span
//! multiple physical lines. Keywords are matched case-insensitively; `FROM`
//! maps to the canonical command name `model` (matching upstream), `PARAMETER`
//! lifts its first token into the command name, and `MESSAGE` packs
//! `"role: content"` into `Args` after validating the role.

use std::fmt;

/// A parsed Modelfile: an ordered list of commands. Cite parser.go `File`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Modelfile {
    /// Commands in source order.
    pub commands: Vec<Command>,
}

/// A single Modelfile command. Cite parser.go `Command{Name, Args}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// Canonical command name (`model`, `system`, `template`, `adapter`,
    /// `license`, `message`, or a parameter name like `temperature`).
    pub name: String,
    /// The command argument payload. For `message` this is `"role: content"`.
    pub args: String,
}

/// Errors raised while parsing a Modelfile. Cite parser.go error vars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelfileError {
    /// No `FROM` line present — cite `errMissingFrom`.
    MissingFrom,
    /// Unrecognised command keyword — cite `errInvalidCommand`.
    InvalidCommand(String),
    /// `MESSAGE` role not in {system, user, assistant} — cite
    /// `errInvalidMessageRole`.
    InvalidMessageRole(String),
    /// A quoted value opened with `"` or `"""` but never closed.
    UnterminatedQuote,
}

impl fmt::Display for ModelfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelfileError::MissingFrom => write!(f, "no FROM line"),
            ModelfileError::InvalidCommand(c) => write!(f, "command must be one of \"from\", \"license\", \"template\", \"system\", \"adapter\", \"parameter\", or \"message\": {c}"),
            ModelfileError::InvalidMessageRole(r) => {
                write!(f, "role must be one of \"system\", \"user\", or \"assistant\": {r}")
            }
            ModelfileError::UnterminatedQuote => write!(f, "unterminated quoted value"),
        }
    }
}

impl std::error::Error for ModelfileError {}

/// Valid `MESSAGE` roles. Cite parser.go role validation.
const VALID_ROLES: [&str; 3] = ["system", "user", "assistant"];

impl Modelfile {
    /// Parse a Modelfile from source text. Cite parser.go `ParseFile`.
    pub fn parse(_input: &str) -> Result<Modelfile, ModelfileError> {
        // RED placeholder — implemented in the GREEN commit.
        unimplemented!("Modelfile::parse")
    }

    /// The base model named by the first `FROM` command, if any.
    pub fn from(&self) -> Option<&str> {
        self.commands
            .iter()
            .find(|c| c.name == "model")
            .map(|c| c.args.as_str())
    }

    /// The `SYSTEM` prompt, if present.
    pub fn system(&self) -> Option<&str> {
        self.command_args("system")
    }

    /// The `TEMPLATE`, if present.
    pub fn template(&self) -> Option<&str> {
        self.command_args("template")
    }

    fn command_args(&self, name: &str) -> Option<&str> {
        self.commands
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.args.as_str())
    }

    /// All `PARAMETER` commands as `(name, value)` pairs, in source order.
    /// A parameter is any command whose name is not one of the reserved
    /// command names.
    pub fn parameters(&self) -> Vec<(&str, &str)> {
        const RESERVED: [&str; 6] = ["model", "system", "template", "adapter", "license", "message"];
        self.commands
            .iter()
            .filter(|c| !RESERVED.contains(&c.name.as_str()))
            .map(|c| (c.name.as_str(), c.args.as_str()))
            .collect()
    }

    /// All `MESSAGE` commands as `(role, content)` pairs, in source order.
    pub fn messages(&self) -> Vec<(&str, &str)> {
        self.commands
            .iter()
            .filter(|c| c.name == "message")
            .filter_map(|c| split_message(&c.args))
            .collect()
    }

    /// Serialise back to canonical Modelfile text. Cite parser.go
    /// `Command.String()` / `File.String()`.
    pub fn to_modelfile_string(&self) -> String {
        // RED placeholder — implemented in the GREEN commit.
        unimplemented!("Modelfile::to_modelfile_string")
    }
}

/// Split a stored `message` arg `"role: content"` into its parts.
fn split_message(args: &str) -> Option<(&str, &str)> {
    args.split_once(": ").map(|(r, c)| (r, c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_from() {
        let mf = Modelfile::parse("FROM llama3.2").unwrap();
        assert_eq!(mf.commands.len(), 1);
        assert_eq!(mf.commands[0].name, "model");
        assert_eq!(mf.commands[0].args, "llama3.2");
        assert_eq!(mf.from(), Some("llama3.2"));
    }

    #[test]
    fn missing_from_is_error() {
        let err = Modelfile::parse("SYSTEM you are helpful").unwrap_err();
        assert_eq!(err, ModelfileError::MissingFrom);
    }

    #[test]
    fn comments_and_blank_lines_skipped() {
        let src = "# a comment\n\nFROM base\n   # indented comment\nSYSTEM hi";
        let mf = Modelfile::parse(src).unwrap();
        assert_eq!(mf.commands.len(), 2);
        assert_eq!(mf.from(), Some("base"));
        assert_eq!(mf.system(), Some("hi"));
    }

    #[test]
    fn parameter_name_lowercased_value_preserved() {
        let src = "FROM base\nPARAMETER Temperature 0.8\nPARAMETER top_k 40";
        let mf = Modelfile::parse(src).unwrap();
        let params = mf.parameters();
        assert_eq!(params, vec![("temperature", "0.8"), ("top_k", "40")]);
    }

    #[test]
    fn keywords_are_case_insensitive() {
        let mf = Modelfile::parse("from base\nsystem hello").unwrap();
        assert_eq!(mf.from(), Some("base"));
        assert_eq!(mf.system(), Some("hello"));
    }

    #[test]
    fn triple_quoted_multiline_template() {
        let src = "FROM base\nTEMPLATE \"\"\"{{ .System }}\nUser: {{ .Prompt }}\nAssistant: \"\"\"";
        let mf = Modelfile::parse(src).unwrap();
        assert_eq!(
            mf.template(),
            Some("{{ .System }}\nUser: {{ .Prompt }}\nAssistant: ")
        );
    }

    #[test]
    fn single_quoted_value() {
        let mf = Modelfile::parse("FROM base\nSYSTEM \"be concise\"").unwrap();
        assert_eq!(mf.system(), Some("be concise"));
    }

    #[test]
    fn message_role_validated_and_packed() {
        let src = "FROM base\nMESSAGE user Hello there\nMESSAGE assistant Hi!";
        let mf = Modelfile::parse(src).unwrap();
        assert_eq!(
            mf.messages(),
            vec![("user", "Hello there"), ("assistant", "Hi!")]
        );
    }

    #[test]
    fn invalid_message_role_errors() {
        let err = Modelfile::parse("FROM base\nMESSAGE captain ahoy").unwrap_err();
        assert_eq!(err, ModelfileError::InvalidMessageRole("captain".into()));
    }

    #[test]
    fn invalid_command_errors() {
        let err = Modelfile::parse("FROM base\nBOGUS whatever").unwrap_err();
        assert_eq!(err, ModelfileError::InvalidCommand("bogus".into()));
    }

    #[test]
    fn unterminated_triple_quote_errors() {
        let err = Modelfile::parse("FROM base\nSYSTEM \"\"\"never closed").unwrap_err();
        assert_eq!(err, ModelfileError::UnterminatedQuote);
    }

    #[test]
    fn roundtrip_serialization() {
        let src = "FROM base\nPARAMETER temperature 0.7\nSYSTEM be nice\nMESSAGE user hi";
        let mf = Modelfile::parse(src).unwrap();
        let out = mf.to_modelfile_string();
        // Re-parsing the serialized form must yield the same commands.
        let reparsed = Modelfile::parse(&out).unwrap();
        assert_eq!(reparsed, mf);
        // Spot-check the canonical line shapes.
        assert!(out.contains("FROM base"), "out={out}");
        assert!(out.contains("PARAMETER temperature \"0.7\""), "out={out}");
        assert!(out.contains("SYSTEM \"be nice\""), "out={out}");
        assert!(out.contains("MESSAGE user \"hi\""), "out={out}");
    }

    #[test]
    fn valid_roles_constant_matches_upstream() {
        assert_eq!(VALID_ROLES, ["system", "user", "assistant"]);
    }
}
