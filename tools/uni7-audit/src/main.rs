// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE
//
// Permission to use, copy, modify, and/or distribute this software for any
// purpose with or without fee is hereby granted.

#![deny(warnings)]
#![forbid(unsafe_code)]

use std::env;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process;

const DEFAULT_MATRIX: &str = "docs/unixv7/interface-matrix.toml";

fn main() {
    let config = match Config::from_env(env::args().skip(1)) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("uni7-audit: {err}");
            usage();
            process::exit(2);
        }
    };

    if config.help {
        usage();
        return;
    }

    let report = match run(&config) {
        Ok(report) => report,
        Err(err) => {
            eprintln!("uni7-audit: {err}");
            process::exit(1);
        }
    };

    if config.json {
        print_json(&report);
    } else {
        print_text(&report);
    }

    if config.strict && report.has_failures() {
        process::exit(1);
    }
}

fn usage() {
    eprintln!("Usage: uni7-audit [--matrix PATH] [--root PATH] [--json] [--strict]");
}

fn run(config: &Config) -> Result<Report, AuditError> {
    let source = fs::read_to_string(&config.matrix)
        .map_err(|source| AuditError::ReadMatrix { path: config.matrix.clone(), source })?;
    let matrix = Matrix::parse(&source)?;
    let probe_results = matrix
        .interfaces
        .iter()
        .map(|interface| probe_interface(interface, &config.root))
        .collect();
    Ok(Report { matrix, probe_results })
}

#[derive(Debug)]
struct Config {
    matrix: PathBuf,
    root: PathBuf,
    json: bool,
    strict: bool,
    help: bool,
}

impl Config {
    fn from_env(args: impl Iterator<Item = String>) -> Result<Self, AuditError> {
        let mut matrix = PathBuf::from(DEFAULT_MATRIX);
        let mut root = PathBuf::from("/");
        let mut json = false;
        let mut strict = false;
        let mut help = false;
        let mut pending: Option<PendingArg> = None;

        for arg in args {
            if let Some(kind) = pending.take() {
                match kind {
                    PendingArg::Matrix => matrix = PathBuf::from(arg),
                    PendingArg::Root => root = PathBuf::from(arg),
                }
                continue;
            }

            match arg.as_str() {
                "--matrix" => pending = Some(PendingArg::Matrix),
                "--root" => pending = Some(PendingArg::Root),
                "--json" => json = true,
                "--strict" => strict = true,
                "-h" | "--help" => help = true,
                other => return Err(AuditError::BadArgument(other.to_string())),
            }
        }

        if let Some(kind) = pending {
            return Err(AuditError::MissingArgument(kind));
        }

        Ok(Self { matrix, root, json, strict, help })
    }
}

#[derive(Debug, Clone, Copy)]
enum PendingArg {
    Matrix,
    Root,
}

impl fmt::Display for PendingArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Matrix => f.write_str("--matrix"),
            Self::Root => f.write_str("--root"),
        }
    }
}

#[derive(Debug)]
struct Matrix {
    schema_version: u32,
    interfaces: Vec<Interface>,
}

impl Matrix {
    fn parse(source: &str) -> Result<Self, AuditError> {
        let mut schema_version = None;
        let mut interfaces = Vec::new();
        let mut current: Option<InterfaceBuilder> = None;

        for (index, raw_line) in source.lines().enumerate() {
            let line_number = LineNumber(index + 1);
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }

            if line == "[[interface]]" {
                if let Some(builder) = current.take() {
                    interfaces.push(builder.build()?);
                }
                current = Some(InterfaceBuilder::new(line_number));
                continue;
            }

            let (key, value) = split_key_value(line, line_number)?;
            if current.is_none() && key == "schema_version" {
                schema_version = Some(parse_u32(value, line_number)?);
                continue;
            }

            let builder = current.as_mut().ok_or(AuditError::KeyOutsideInterface {
                key: key.to_string(),
                line: line_number,
            })?;
            builder.set(key, value, line_number)?;
        }

        if let Some(builder) = current.take() {
            interfaces.push(builder.build()?);
        }

        Ok(Self {
            schema_version: schema_version.ok_or(AuditError::MissingSchemaVersion)?,
            interfaces,
        })
    }

    fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for interface in &self.interfaces {
            counts.total += 1;
            match interface.status {
                Status::Covered => counts.covered += 1,
                Status::Partial => counts.partial += 1,
                Status::Missing => counts.missing += 1,
                Status::Blocked => counts.blocked += 1,
                Status::Unknown => counts.unknown += 1,
            }
        }
        counts
    }
}

#[derive(Debug)]
struct Interface {
    name: String,
    class: InterfaceClass,
    required_by: String,
    provider: String,
    status: Status,
    probe: Probe,
    paths: Vec<InterfacePath>,
    verification: String,
    notes: String,
    defined_at: LineNumber,
}

#[derive(Debug)]
struct InterfaceBuilder {
    name: Option<String>,
    class: Option<InterfaceClass>,
    required_by: Option<String>,
    provider: Option<String>,
    status: Option<Status>,
    probe: Option<Probe>,
    paths: Option<Vec<InterfacePath>>,
    verification: Option<String>,
    notes: Option<String>,
    defined_at: LineNumber,
}

impl InterfaceBuilder {
    fn new(defined_at: LineNumber) -> Self {
        Self {
            name: None,
            class: None,
            required_by: None,
            provider: None,
            status: None,
            probe: None,
            paths: None,
            verification: None,
            notes: None,
            defined_at,
        }
    }

    fn set(&mut self, key: &str, value: &str, line: LineNumber) -> Result<(), AuditError> {
        match key {
            "name" => set_once(&mut self.name, "name", line, parse_string(value, line)?)?,
            "class" => set_once(&mut self.class, "class", line, parse_string(value, line)?.parse()?)?,
            "required_by" => {
                set_once(&mut self.required_by, "required_by", line, parse_string(value, line)?)?;
            }
            "provider" => set_once(&mut self.provider, "provider", line, parse_string(value, line)?)?,
            "status" => set_once(&mut self.status, "status", line, parse_string(value, line)?.parse()?)?,
            "probe" => set_once(&mut self.probe, "probe", line, parse_string(value, line)?.parse()?)?,
            "paths" => set_once(&mut self.paths, "paths", line, parse_paths(value, line)?)?,
            "verification" => {
                set_once(&mut self.verification, "verification", line, parse_string(value, line)?)?;
            }
            "notes" => set_once(&mut self.notes, "notes", line, parse_string(value, line)?)?,
            other => {
                return Err(AuditError::UnknownKey {
                    key: other.to_string(),
                    line,
                });
            }
        }
        Ok(())
    }

    fn build(self) -> Result<Interface, AuditError> {
        let line = self.defined_at;
        Ok(Interface {
            name: required(self.name, "name", line)?,
            class: required(self.class, "class", line)?,
            required_by: required(self.required_by, "required_by", line)?,
            provider: required(self.provider, "provider", line)?,
            status: required(self.status, "status", line)?,
            probe: required(self.probe, "probe", line)?,
            paths: required(self.paths, "paths", line)?,
            verification: required(self.verification, "verification", line)?,
            notes: required(self.notes, "notes", line)?,
            defined_at: line,
        })
    }
}

fn required<T>(value: Option<T>, field: &'static str, line: LineNumber) -> Result<T, AuditError> {
    value.ok_or(AuditError::MissingField { field, line })
}

fn set_once<T>(
    slot: &mut Option<T>,
    field: &'static str,
    line: LineNumber,
    value: T,
) -> Result<(), AuditError> {
    if slot.is_some() {
        return Err(AuditError::DuplicateKey { field, line });
    }
    *slot = Some(value);
    Ok(())
}

#[derive(Debug, Clone)]
struct InterfacePath(String);

impl InterfacePath {
    fn parse(value: String, line: LineNumber) -> Result<Self, AuditError> {
        if value.is_empty() {
            return Err(AuditError::BadPath {
                value,
                line,
                reason: "path is empty",
            });
        }
        if !value.starts_with('/') {
            return Err(AuditError::BadPath {
                value,
                line,
                reason: "path must be absolute",
            });
        }

        let mut has_normal_component = false;
        for component in Path::new(&value).components() {
            match component {
                Component::RootDir => {}
                Component::Normal(_) => has_normal_component = true,
                Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                    return Err(AuditError::BadPath {
                        value,
                        line,
                        reason: "path must be normalized under the root",
                    });
                }
            }
        }

        if !has_normal_component {
            return Err(AuditError::BadPath {
                value,
                line,
                reason: "path must name an interface object",
            });
        }

        Ok(Self(value))
    }

    fn as_str(&self) -> &str {
        &self.0
    }

    fn join_under(&self, root: &Path) -> PathBuf {
        let mut joined = root.to_path_buf();
        for component in Path::new(&self.0).components() {
            if let Component::Normal(part) = component {
                joined.push(part);
            }
        }
        joined
    }
}

#[derive(Debug, Clone, Copy)]
enum InterfaceClass {
    System,
    Utility,
    Header,
    Curses,
}

impl std::str::FromStr for InterfaceClass {
    type Err = AuditError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "system" => Ok(Self::System),
            "utility" => Ok(Self::Utility),
            "header" => Ok(Self::Header),
            "curses" => Ok(Self::Curses),
            other => Err(AuditError::BadEnum {
                kind: "class",
                value: other.to_string(),
            }),
        }
    }
}

impl fmt::Display for InterfaceClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::System => f.write_str("system"),
            Self::Utility => f.write_str("utility"),
            Self::Header => f.write_str("header"),
            Self::Curses => f.write_str("curses"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Status {
    Covered,
    Partial,
    Missing,
    Blocked,
    Unknown,
}

impl std::str::FromStr for Status {
    type Err = AuditError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "covered" => Ok(Self::Covered),
            "partial" => Ok(Self::Partial),
            "missing" => Ok(Self::Missing),
            "blocked" => Ok(Self::Blocked),
            "unknown" => Ok(Self::Unknown),
            other => Err(AuditError::BadEnum {
                kind: "status",
                value: other.to_string(),
            }),
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Covered => f.write_str("covered"),
            Self::Partial => f.write_str("partial"),
            Self::Missing => f.write_str("missing"),
            Self::Blocked => f.write_str("blocked"),
            Self::Unknown => f.write_str("unknown"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Probe {
    Path,
    Header,
    Compile,
    Manual,
}

impl std::str::FromStr for Probe {
    type Err = AuditError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "path" => Ok(Self::Path),
            "header" => Ok(Self::Header),
            "compile" => Ok(Self::Compile),
            "manual" => Ok(Self::Manual),
            other => Err(AuditError::BadEnum {
                kind: "probe",
                value: other.to_string(),
            }),
        }
    }
}

impl fmt::Display for Probe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path => f.write_str("path"),
            Self::Header => f.write_str("header"),
            Self::Compile => f.write_str("compile"),
            Self::Manual => f.write_str("manual"),
        }
    }
}

#[derive(Debug, Default)]
struct Counts {
    total: usize,
    covered: usize,
    partial: usize,
    missing: usize,
    blocked: usize,
    unknown: usize,
}

#[derive(Debug)]
struct Report {
    matrix: Matrix,
    probe_results: Vec<ProbeResult>,
}

impl Report {
    fn has_failures(&self) -> bool {
        self.probe_results
            .iter()
            .any(|result| matches!(result.outcome, ProbeOutcome::Fail(_) | ProbeOutcome::Unimplemented(_)))
    }
}

#[derive(Debug)]
struct ProbeResult {
    name: String,
    outcome: ProbeOutcome,
}

#[derive(Debug)]
enum ProbeOutcome {
    Pass(String),
    Fail(String),
    Skip(String),
    Unimplemented(String),
}

fn probe_interface(interface: &Interface, root: &Path) -> ProbeResult {
    let outcome = match interface.probe {
        Probe::Path | Probe::Header => probe_paths(interface, root),
        Probe::Compile => ProbeOutcome::Unimplemented("compile probes are not implemented yet".to_string()),
        Probe::Manual => ProbeOutcome::Unimplemented("manual verification adapter is not implemented yet".to_string()),
    };
    ProbeResult {
        name: interface.name.clone(),
        outcome,
    }
}

fn probe_paths(interface: &Interface, root: &Path) -> ProbeOutcome {
    if interface.paths.is_empty() {
        return ProbeOutcome::Skip("no paths declared".to_string());
    }

    let mut missing = Vec::new();
    for path in &interface.paths {
        let candidate = path.join_under(root);
        if !candidate.exists() {
            missing.push(path.as_str());
        }
    }

    if missing.is_empty() && matches!(interface.status, Status::Missing | Status::Blocked) {
        ProbeOutcome::Skip(format!(
            "declared status is {}; paths exist but semantic/provider gap remains",
            interface.status
        ))
    } else if missing.is_empty() {
        ProbeOutcome::Pass("all declared paths exist".to_string())
    } else if matches!(interface.status, Status::Missing | Status::Blocked | Status::Unknown) {
        ProbeOutcome::Skip(format!("declared status is {}; missing {}", interface.status, missing.join(", ")))
    } else {
        ProbeOutcome::Fail(format!("missing {}", missing.join(", ")))
    }
}

fn print_text(report: &Report) {
    let counts = report.matrix.counts();
    println!("uni7-audit: schema {}", report.matrix.schema_version);
    println!(
        "interfaces: total={} covered={} partial={} missing={} blocked={} unknown={}",
        counts.total, counts.covered, counts.partial, counts.missing, counts.blocked, counts.unknown
    );
    println!();
    for (interface, result) in report.matrix.interfaces.iter().zip(&report.probe_results) {
        let (outcome_name, outcome_message) = outcome_parts(&result.outcome);
        println!(
            "{} class={} provider={} status={} required_by={} line={} probe={} outcome={} message=\"{}\" verification=\"{}\"",
            result.name,
            interface.class,
            interface.provider,
            interface.status,
            interface.required_by,
            interface.defined_at.0,
            interface.probe,
            outcome_name,
            outcome_message,
            interface.verification
        );
        if !interface.notes.is_empty() {
            println!("  notes: {}", interface.notes);
        }
    }
}

fn print_json(report: &Report) {
    let counts = report.matrix.counts();
    println!("{{");
    println!("  \"schema_version\": {},", report.matrix.schema_version);
    println!(
        "  \"counts\": {{\"total\": {}, \"covered\": {}, \"partial\": {}, \"missing\": {}, \"blocked\": {}, \"unknown\": {}}},",
        counts.total, counts.covered, counts.partial, counts.missing, counts.blocked, counts.unknown
    );
    println!("  \"interfaces\": [");
    for (index, (interface, result)) in report
        .matrix
        .interfaces
        .iter()
        .zip(&report.probe_results)
        .enumerate()
    {
        let comma = if index + 1 == report.matrix.interfaces.len() { "" } else { "," };
        let (probe_status, probe_message) = outcome_parts(&result.outcome);
        println!("    {{");
        println!("      \"name\": \"{}\",", json_escape(&interface.name));
        println!("      \"class\": \"{}\",", interface.class);
        println!("      \"required_by\": \"{}\",", json_escape(&interface.required_by));
        println!("      \"provider\": \"{}\",", json_escape(&interface.provider));
        println!("      \"status\": \"{}\",", interface.status);
        println!("      \"probe\": \"{}\",", interface.probe);
        println!("      \"paths\": {},", json_paths(&interface.paths));
        println!("      \"verification\": \"{}\",", json_escape(&interface.verification));
        println!("      \"notes\": \"{}\",", json_escape(&interface.notes));
        println!("      \"defined_at\": {},", interface.defined_at.0);
        println!("      \"probe_status\": \"{}\",", probe_status);
        println!("      \"probe_message\": \"{}\"", json_escape(probe_message));
        println!("    }}{comma}");
    }
    println!("  ]");
    println!("}}");
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '"' if !escaped => in_string = !in_string,
            '#' if !in_string => return &line[..index],
            '\\' if in_string => {
                escaped = !escaped;
                continue;
            }
            _ => {}
        }
        escaped = false;
    }
    line
}

fn split_key_value(line: &str, line_number: LineNumber) -> Result<(&str, &str), AuditError> {
    let (key, value) = line.split_once('=').ok_or(AuditError::MalformedLine(line_number))?;
    Ok((key.trim(), value.trim()))
}

fn parse_u32(value: &str, line: LineNumber) -> Result<u32, AuditError> {
    value.parse().map_err(|_| AuditError::BadInteger {
        value: value.to_string(),
        line,
    })
}

fn parse_string(value: &str, line: LineNumber) -> Result<String, AuditError> {
    let value = value.trim();
    if !value.starts_with('"') || !value.ends_with('"') || value.len() < 2 {
        return Err(AuditError::BadString { value: value.to_string(), line });
    }

    let mut out = String::new();
    let mut escaped = false;
    for ch in value[1..value.len() - 1].chars() {
        if escaped {
            match ch {
                '"' | '\\' => out.push(ch),
                _ => {
                    return Err(AuditError::BadString {
                        value: value.to_string(),
                        line,
                    });
                }
            }
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '"' => {
                return Err(AuditError::BadString {
                    value: value.to_string(),
                    line,
                });
            }
            _ => out.push(ch),
        }
    }

    if escaped {
        return Err(AuditError::BadString {
            value: value.to_string(),
            line,
        });
    }

    Ok(out)
}

fn parse_paths(value: &str, line: LineNumber) -> Result<Vec<InterfacePath>, AuditError> {
    parse_string_array(value, line)?
        .into_iter()
        .map(|path| InterfacePath::parse(path, line))
        .collect()
}

fn parse_string_array(value: &str, line: LineNumber) -> Result<Vec<String>, AuditError> {
    let value = value.trim();
    if !value.starts_with('[') || !value.ends_with(']') {
        return Err(AuditError::BadArray { value: value.to_string(), line });
    }
    let inner = value[1..value.len() - 1].trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }

    let mut rest = inner;
    let mut values = Vec::new();
    while !rest.trim_start().is_empty() {
        rest = rest.trim_start();
        let end = find_string_end(rest, line)?;
        let token = &rest[..=end];
        values.push(parse_string(token, line)?);
        rest = rest[end + 1..].trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some(after_comma) = rest.strip_prefix(',') {
            rest = after_comma;
        } else {
            return Err(AuditError::BadArray {
                value: value.to_string(),
                line,
            });
        }
    }

    Ok(values)
}

fn find_string_end(value: &str, line: LineNumber) -> Result<usize, AuditError> {
    if !value.starts_with('"') {
        return Err(AuditError::BadString {
            value: value.to_string(),
            line,
        });
    }

    let mut escaped = false;
    for (index, ch) in value.char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Ok(index),
            _ => {}
        }
    }

    Err(AuditError::BadString {
        value: value.to_string(),
        line,
    })
}

fn outcome_parts(outcome: &ProbeOutcome) -> (&'static str, &str) {
    match outcome {
        ProbeOutcome::Pass(message) => ("pass", message),
        ProbeOutcome::Fail(message) => ("fail", message),
        ProbeOutcome::Skip(message) => ("skip", message),
        ProbeOutcome::Unimplemented(message) => ("unimplemented", message),
    }
}

fn json_escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

fn json_paths(paths: &[InterfacePath]) -> String {
    let mut out = String::from("[");
    for (index, path) in paths.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push('"');
        out.push_str(&json_escape(path.as_str()));
        out.push('"');
    }
    out.push(']');
    out
}

#[derive(Debug, Clone, Copy)]
struct LineNumber(usize);

#[derive(Debug)]
enum AuditError {
    BadArgument(String),
    BadArray { value: String, line: LineNumber },
    BadEnum { kind: &'static str, value: String },
    BadInteger { value: String, line: LineNumber },
    BadPath { value: String, line: LineNumber, reason: &'static str },
    BadString { value: String, line: LineNumber },
    DuplicateKey { field: &'static str, line: LineNumber },
    KeyOutsideInterface { key: String, line: LineNumber },
    MalformedLine(LineNumber),
    MissingArgument(PendingArg),
    MissingField { field: &'static str, line: LineNumber },
    MissingSchemaVersion,
    ReadMatrix { path: PathBuf, source: std::io::Error },
    UnknownKey { key: String, line: LineNumber },
}

impl fmt::Display for AuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadArgument(arg) => write!(f, "unknown argument {arg}"),
            Self::BadArray { value, line } => write!(f, "line {}: bad array {value}", line.0),
            Self::BadEnum { kind, value } => write!(f, "bad {kind} value {value}"),
            Self::BadInteger { value, line } => write!(f, "line {}: bad integer {value}", line.0),
            Self::BadPath { value, line, reason } => {
                write!(f, "line {}: bad interface path {value}: {reason}", line.0)
            }
            Self::BadString { value, line } => write!(f, "line {}: bad string {value}", line.0),
            Self::DuplicateKey { field, line } => {
                write!(f, "line {}: duplicate interface key {field}", line.0)
            }
            Self::KeyOutsideInterface { key, line } => {
                write!(f, "line {}: key {key} appears outside [[interface]]", line.0)
            }
            Self::MalformedLine(line) => write!(f, "line {}: malformed key/value line", line.0),
            Self::MissingArgument(arg) => write!(f, "{arg} requires a value"),
            Self::MissingField { field, line } => {
                write!(f, "interface starting at line {} is missing {field}", line.0)
            }
            Self::MissingSchemaVersion => f.write_str("missing schema_version"),
            Self::ReadMatrix { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::UnknownKey { key, line } => write!(f, "line {}: unknown key {key}", line.0),
        }
    }
}

impl std::error::Error for AuditError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_seed_shape() {
        let source = r#"
schema_version = 1

[[interface]]
name = "pax"
class = "utility"
required_by = "posix"
provider = "missing"
status = "missing"
probe = "path"
paths = ["/bin/pax"]
verification = "missing"
notes = "seed"
"#;
        let matrix = Matrix::parse(source).expect("matrix should parse");
        assert_eq!(matrix.schema_version, 1);
        assert_eq!(matrix.interfaces.len(), 1);
        assert_eq!(matrix.interfaces[0].name, "pax");
        assert_eq!(matrix.interfaces[0].paths[0].as_str(), "/bin/pax");
    }

    #[test]
    fn strips_comments_outside_strings() {
        assert_eq!(strip_comment("name = \"# not comment\" # comment").trim(), "name = \"# not comment\"");
    }

    #[test]
    fn parses_array_commas_inside_strings() {
        let parsed = parse_string_array("[\"/bin/a,b\", \"/bin/c\"]", LineNumber(1))
            .expect("array should parse");
        assert_eq!(parsed, vec!["/bin/a,b".to_string(), "/bin/c".to_string()]);
    }

    #[test]
    fn rejects_duplicate_keys() {
        let source = r#"
schema_version = 1

[[interface]]
name = "pax"
name = "ed"
class = "utility"
required_by = "posix"
provider = "missing"
status = "missing"
probe = "path"
paths = ["/bin/pax"]
verification = "missing"
notes = "seed"
"#;
        let err = Matrix::parse(source).expect_err("duplicate key should fail");
        assert!(matches!(err, AuditError::DuplicateKey { field: "name", .. }));
    }

    #[test]
    fn rejects_parent_components_in_paths() {
        let err = InterfacePath::parse("/bin/../sh".to_string(), LineNumber(1))
            .expect_err("parent component should fail");
        assert!(matches!(err, AuditError::BadPath { .. }));
    }
}
