//! Jenkins Jenkinsfile compatibility layer.
//!
//! Parses Jenkinsfile (Declarative Pipeline syntax) into a
//! cave-pipelines PipelineSpec. Supports the most common directives.

use crate::models::*;
use std::collections::HashMap;

/// Parsed representation of a Declarative Jenkinsfile.
#[derive(Debug, Clone)]
pub struct JenkinsFile {
    pub agent: AgentDeclaration,
    pub environment: HashMap<String, String>,
    pub stages: Vec<JenkinsStage>,
    pub post: Vec<PostAction>,
    pub options: JenkinsOptions,
}

#[derive(Debug, Clone)]
pub enum AgentDeclaration {
    Any,
    None,
    Label(String),
    Docker { image: String, args: Option<String> },
    Kubernetes { yaml: Option<String>, label: Option<String> },
}

#[derive(Debug, Clone)]
pub struct JenkinsStage {
    pub name: String,
    pub steps: Vec<JenkinsStep>,
    pub parallel: Vec<JenkinsStage>,
    pub when: Option<JenkinsWhen>,
    pub environment: HashMap<String, String>,
    pub agent: Option<AgentDeclaration>,
}

#[derive(Debug, Clone)]
pub struct JenkinsStep {
    pub kind: JenkinsStepKind,
}

#[derive(Debug, Clone)]
pub enum JenkinsStepKind {
    Sh(String),
    Echo(String),
    Checkout { repo: Option<String> },
    Script(String),
    WithCredentials { bindings: Vec<String>, body: Vec<JenkinsStep> },
    ArchiveArtifacts { pattern: String },
    JUnit { test_results: String },
    PublishHTML { report_dir: String, report_files: String },
    Input { message: String, ok: Option<String> },
    Custom { name: String, args: HashMap<String, String> },
}

#[derive(Debug, Clone)]
pub struct JenkinsWhen {
    pub branch: Option<String>,
    pub environment: Option<(String, String)>,
    pub expression: Option<String>,
    pub not: Option<Box<JenkinsWhen>>,
    pub all_of: Vec<JenkinsWhen>,
    pub any_of: Vec<JenkinsWhen>,
}

#[derive(Debug, Clone)]
pub struct PostAction {
    pub condition: PostCondition,
    pub steps: Vec<JenkinsStep>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PostCondition {
    Always,
    Success,
    Failure,
    Unstable,
    Changed,
    Cleanup,
}

#[derive(Debug, Clone, Default)]
pub struct JenkinsOptions {
    pub timeout_minutes: Option<u32>,
    pub retry_count: Option<u32>,
    pub disable_concurrent_builds: bool,
    pub timestamps: bool,
    pub build_discarder: Option<BuildDiscarder>,
}

#[derive(Debug, Clone)]
pub struct BuildDiscarder {
    pub days_to_keep: Option<u32>,
    pub num_to_keep: Option<u32>,
}

// ─── Parser ──────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum JenkinsParseError {
    #[error("Syntax error at line {line}: {message}")]
    Syntax { line: usize, message: String },
    #[error("Unsupported directive: {0}")]
    Unsupported(String),
    #[error("Missing required block: {0}")]
    MissingBlock(String),
}

/// Tokenizer for Groovy/Jenkinsfile DSL (simplified).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    StringLit(String),
    LBrace,
    RBrace,
    LParen,
    RParen,
    Newline,
    Equals,
    Comma,
    Number(i64),
    EOF,
}

#[allow(dead_code)]
struct Tokenizer<'a> {
    input: &'a str,
    pos: usize,
    line: usize,
}

#[allow(dead_code)]
impl<'a> Tokenizer<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0, line: 1 }
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek_char()?;
        self.pos += c.len_utf8();
        if c == '\n' { self.line += 1; }
        Some(c)
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek_char() {
            if c == ' ' || c == '\t' || c == '\r' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn skip_comment(&mut self) -> bool {
        if self.input[self.pos..].starts_with("//") {
            while let Some(c) = self.advance() {
                if c == '\n' { break; }
            }
            return true;
        }
        if self.input[self.pos..].starts_with("/*") {
            self.pos += 2;
            loop {
                if self.input[self.pos..].starts_with("*/") {
                    self.pos += 2;
                    break;
                }
                if self.advance().is_none() { break; }
            }
            return true;
        }
        false
    }

    fn read_string(&mut self, delim: char) -> String {
        let mut s = String::new();
        loop {
            match self.advance() {
                None | Some('\n') => break,
                Some(c) if c == delim => break,
                Some('\\') => {
                    if let Some(escaped) = self.advance() {
                        match escaped {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            _ => s.push(escaped),
                        }
                    }
                }
                Some(c) => s.push(c),
            }
        }
        s
    }

    fn read_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek_char() {
            if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        s
    }

    fn next_token(&mut self) -> (Token, usize) {
        loop {
            self.skip_whitespace();
            if self.skip_comment() { continue; }
            break;
        }
        let line = self.line;
        match self.peek_char() {
            None => (Token::EOF, line),
            Some('\n') => { self.advance(); (Token::Newline, line) },
            Some('{') => { self.advance(); (Token::LBrace, line) },
            Some('}') => { self.advance(); (Token::RBrace, line) },
            Some('(') => { self.advance(); (Token::LParen, line) },
            Some(')') => { self.advance(); (Token::RParen, line) },
            Some('=') => { self.advance(); (Token::Equals, line) },
            Some(',') => { self.advance(); (Token::Comma, line) },
            Some('\'') | Some('"') => {
                let delim = self.advance().unwrap();
                let s = self.read_string(delim);
                (Token::StringLit(s), line)
            }
            Some(c) if c.is_alphabetic() || c == '_' => {
                let ident = self.read_ident();
                (Token::Ident(ident), line)
            }
            Some(c) if c.is_ascii_digit() || c == '-' => {
                let mut n = String::new();
                if c == '-' { self.advance(); n.push('-'); }
                while let Some(d) = self.peek_char() {
                    if d.is_ascii_digit() { n.push(d); self.advance(); }
                    else { break; }
                }
                let num = n.parse().unwrap_or(0);
                (Token::Number(num), line)
            }
            Some(_) => { self.advance(); self.next_token() }
        }
    }
}

/// Parse a Jenkinsfile string into a structured representation.
/// This is a best-effort parser for common Declarative Pipeline patterns.
pub fn parse_jenkinsfile(content: &str) -> Result<JenkinsFile, JenkinsParseError> {
    let mut stages = Vec::new();
    let mut environment = HashMap::new();
    let mut post = Vec::new();
    let mut options = JenkinsOptions::default();
    let mut agent = AgentDeclaration::Any;

    // Simple line-by-line parser for declarative syntax
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        if line.starts_with("agent ") {
            agent = parse_agent_line(line);
        } else if line == "environment {" || line.starts_with("environment {") {
            let (env, advance) = parse_environment_block(&lines, i);
            environment.extend(env);
            i += advance;
        } else if line.starts_with("stage(") || line.starts_with("stage '") {
            let (stage, advance) = parse_stage_block(&lines, i);
            stages.push(stage);
            i += advance;
        } else if line == "post {" {
            let (post_actions, advance) = parse_post_block(&lines, i);
            post.extend(post_actions);
            i += advance;
        } else if line == "options {" {
            let (opts, advance) = parse_options_block(&lines, i);
            options = opts;
            i += advance;
        }

        i += 1;
    }

    Ok(JenkinsFile { agent, environment, stages, post, options })
}

fn parse_agent_line(line: &str) -> AgentDeclaration {
    if line.contains("any") { return AgentDeclaration::Any; }
    if line.contains("none") { return AgentDeclaration::None; }
    if let Some(img_start) = line.find("image:") {
        let rest = &line[img_start + 6..].trim().trim_matches('\'').trim_matches('"');
        return AgentDeclaration::Docker { image: rest.to_string(), args: None };
    }
    if let Some(label_start) = line.find("label:") {
        let rest = line[label_start + 6..].trim().trim_matches('\'').trim_matches('"');
        return AgentDeclaration::Label(rest.to_string());
    }
    AgentDeclaration::Any
}

fn parse_environment_block(lines: &[&str], start: usize) -> (HashMap<String, String>, usize) {
    let mut env = HashMap::new();
    let mut i = start + 1;
    let mut depth = 1;
    while i < lines.len() && depth > 0 {
        let line = lines[i].trim();
        if line.contains('{') { depth += 1; }
        if line.contains('}') { depth -= 1; }
        if depth > 0 {
            if let Some(eq_pos) = line.find('=') {
                let key = line[..eq_pos].trim().to_string();
                let val = line[eq_pos + 1..].trim().trim_matches('\'').trim_matches('"').to_string();
                if !key.is_empty() { env.insert(key, val); }
            }
        }
        i += 1;
    }
    (env, i - start - 1)
}

fn parse_stage_block(lines: &[&str], start: usize) -> (JenkinsStage, usize) {
    let name = extract_stage_name(lines[start]);
    let mut steps = Vec::new();
    let mut parallel_stages = Vec::new();
    let mut i = start + 1;
    let mut depth = 0;

    while i < lines.len() {
        let line = lines[i].trim();
        if line.contains('{') { depth += 1; }
        if line.contains('}') {
            if depth == 0 { break; }
            depth -= 1;
        }
        if line.starts_with("sh ") || line.starts_with("sh(") {
            steps.push(JenkinsStep { kind: JenkinsStepKind::Sh(extract_string_arg(line)) });
        } else if line.starts_with("echo ") {
            steps.push(JenkinsStep { kind: JenkinsStepKind::Echo(extract_string_arg(line)) });
        } else if line.starts_with("checkout") {
            steps.push(JenkinsStep { kind: JenkinsStepKind::Checkout { repo: None } });
        } else if line.starts_with("archiveArtifacts") {
            let pattern = extract_kwarg(line, "artifacts").unwrap_or_else(|| extract_string_arg(line));
            steps.push(JenkinsStep { kind: JenkinsStepKind::ArchiveArtifacts { pattern } });
        } else if line.starts_with("junit ") {
            steps.push(JenkinsStep { kind: JenkinsStepKind::JUnit { test_results: extract_string_arg(line) } });
        } else if line.starts_with("stage(") && i > start {
            let (sub_stage, advance) = parse_stage_block(lines, i);
            parallel_stages.push(sub_stage);
            i += advance;
        }
        i += 1;
    }

    (JenkinsStage {
        name,
        steps,
        parallel: parallel_stages,
        when: None,
        environment: HashMap::new(),
        agent: None,
    }, i - start)
}

fn parse_post_block(lines: &[&str], start: usize) -> (Vec<PostAction>, usize) {
    let mut actions = Vec::new();
    let mut i = start + 1;
    let mut depth = 1;
    let mut current_condition: Option<PostCondition> = None;
    let mut current_steps: Vec<JenkinsStep> = Vec::new();

    while i < lines.len() {
        let line = lines[i].trim();

        // Detect condition name BEFORE updating depth
        if depth == 1 && line.contains('{') {
            let keyword = line.trim_end_matches('{').trim().trim_end_matches('{').trim();
            current_condition = match keyword {
                "always" => Some(PostCondition::Always),
                "success" => Some(PostCondition::Success),
                "failure" => Some(PostCondition::Failure),
                "cleanup" => Some(PostCondition::Cleanup),
                "unstable" => Some(PostCondition::Unstable),
                "changed" => Some(PostCondition::Changed),
                _ => current_condition.clone(),
            };
        }

        if line.contains('{') { depth += 1; }
        if line.contains('}') {
            depth -= 1;
            if depth == 1 {
                if let Some(cond) = current_condition.take() {
                    actions.push(PostAction { condition: cond, steps: current_steps.clone() });
                    current_steps.clear();
                }
            }
            if depth == 0 { break; }
        }

        if depth == 2 && line.starts_with("sh ") {
            current_steps.push(JenkinsStep { kind: JenkinsStepKind::Sh(extract_string_arg(line)) });
        } else if depth == 2 && line.starts_with("echo ") {
            current_steps.push(JenkinsStep { kind: JenkinsStepKind::Echo(extract_string_arg(line)) });
        }
        i += 1;
    }
    (actions, i - start)
}

fn parse_options_block(lines: &[&str], start: usize) -> (JenkinsOptions, usize) {
    let mut opts = JenkinsOptions::default();
    let mut i = start + 1;
    let mut depth = 1;
    while i < lines.len() && depth > 0 {
        let line = lines[i].trim();
        if line.contains('{') { depth += 1; }
        if line.contains('}') { depth -= 1; }
        if depth > 0 {
            if line.starts_with("timeout(") {
                if let Some(mins) = extract_kwarg(line, "time") {
                    opts.timeout_minutes = mins.parse().ok();
                }
            } else if line.starts_with("retry(") {
                if let Some(n) = extract_paren_arg(line) {
                    opts.retry_count = n.parse().ok();
                }
            } else if line.contains("disableConcurrentBuilds") {
                opts.disable_concurrent_builds = true;
            } else if line.contains("timestamps") {
                opts.timestamps = true;
            }
        }
        i += 1;
    }
    (opts, i - start)
}

fn extract_stage_name(line: &str) -> String {
    // stage('Build') or stage "Build"
    if let Some(start) = line.find('\'') {
        if let Some(end) = line[start + 1..].find('\'') {
            return line[start + 1..start + 1 + end].to_string();
        }
    }
    if let Some(start) = line.find('"') {
        if let Some(end) = line[start + 1..].find('"') {
            return line[start + 1..start + 1 + end].to_string();
        }
    }
    "unnamed-stage".to_string()
}

fn extract_string_arg(line: &str) -> String {
    let s = line.trim();
    for delim in &['\'', '"'] {
        if let Some(start) = s.find(*delim) {
            if let Some(end) = s[start + 1..].find(*delim) {
                return s[start + 1..start + 1 + end].to_string();
            }
        }
    }
    s.to_string()
}

fn extract_kwarg(line: &str, key: &str) -> Option<String> {
    let search = format!("{}:", key);
    let idx = line.find(&search)?;
    let rest = line[idx + search.len()..].trim();
    Some(rest.trim_matches('\'').trim_matches('"').trim_end_matches(',').trim_end_matches(')').to_string())
}

fn extract_paren_arg(line: &str) -> Option<String> {
    let start = line.find('(')?;
    let end = line.find(')')?;
    Some(line[start + 1..end].trim().trim_matches('\'').trim_matches('"').to_string())
}

// ─── Converter: JenkinsFile → PipelineSpec ────────────────────────────────────

/// Convert a parsed Jenkinsfile into a cave-pipelines PipelineSpec.
pub fn to_pipeline_spec(jf: &JenkinsFile) -> PipelineSpec {
    let tasks: Vec<PipelineTask> = jf.stages.iter().enumerate().map(|(idx, stage)| {
        let run_after = if idx > 0 {
            vec![slug(&jf.stages[idx - 1].name)]
        } else {
            vec![]
        };

        let steps: Vec<Step> = stage.steps.iter().map(jenkins_step_to_cave_step).collect();
        let embedded = EmbeddedTaskSpec {
            steps,
            params: vec![],
            workspaces: vec![],
            results: vec![],
        };

        PipelineTask {
            name: slug(&stage.name),
            task_ref: None,
            task_spec: Some(embedded),
            run_after,
            params: stage.environment.iter().map(|(k, v)| Param {
                name: k.clone(),
                value: ParamValue::String(v.clone()),
            }).collect(),
            workspaces: vec![],
            when: stage.when.as_ref().map(jenkins_when_to_cave).unwrap_or_default(),
            matrix: None,
            retry_policy: None,
            timeout: None,
            custom_task_ref: None,
        }
    }).collect();

    let finally: Vec<PipelineTask> = jf.post.iter()
        .filter(|p| p.condition == PostCondition::Always || p.condition == PostCondition::Cleanup)
        .enumerate()
        .map(|(idx, post)| {
            let steps: Vec<Step> = post.steps.iter().map(jenkins_step_to_cave_step).collect();
            PipelineTask {
                name: format!("post-{}-{}", format!("{:?}", post.condition).to_lowercase(), idx),
                task_ref: None,
                task_spec: Some(EmbeddedTaskSpec { steps, params: vec![], workspaces: vec![], results: vec![] }),
                run_after: vec![],
                params: vec![],
                workspaces: vec![],
                when: vec![],
                matrix: None,
                retry_policy: None,
                timeout: None,
                custom_task_ref: None,
            }
        })
        .collect();

    let env_params: Vec<ParamSpec> = jf.environment.iter().map(|(k, _v)| ParamSpec {
        name: k.clone(),
        param_type: ParamType::String,
        description: None,
        default: jf.environment.get(k).map(|v| ParamValue::String(v.clone())),
        enum_values: None,
    }).collect();

    PipelineSpec {
        params: env_params,
        workspaces: vec![],
        results: vec![],
        tasks,
        finally,
        description: None,
        timeout: jf.options.timeout_minutes.map(|m| PipelineTimeout {
            pipeline: Some(format!("{}m", m)),
            tasks: None,
            finally: None,
        }),
    }
}

fn jenkins_step_to_cave_step(step: &JenkinsStep) -> Step {
    match &step.kind {
        JenkinsStepKind::Sh(cmd) => Step {
            name: "sh".to_string(),
            image: "alpine:3.18".to_string(),
            command: Some(vec!["sh".to_string(), "-c".to_string()]),
            args: vec![cmd.clone()],
            env: vec![],
            volume_mounts: vec![],
            script: None,
            working_dir: None,
            resources: None,
            security_context: None,
            timeout: None,
            ref_: None,
            results: vec![],
        },
        JenkinsStepKind::Echo(msg) => Step {
            name: "echo".to_string(),
            image: "alpine:3.18".to_string(),
            command: Some(vec!["echo".to_string()]),
            args: vec![msg.clone()],
            env: vec![],
            volume_mounts: vec![],
            script: None,
            working_dir: None,
            resources: None,
            security_context: None,
            timeout: None,
            ref_: None,
            results: vec![],
        },
        JenkinsStepKind::Checkout { .. } => Step {
            name: "checkout".to_string(),
            image: "alpine/git:2.40".to_string(),
            command: Some(vec!["git".to_string(), "checkout".to_string()]),
            args: vec![],
            env: vec![],
            volume_mounts: vec![],
            script: None,
            working_dir: None,
            resources: None,
            security_context: None,
            timeout: None,
            ref_: Some(StepActionRef { name: "git-clone".to_string(), version: None }),
            results: vec![],
        },
        JenkinsStepKind::ArchiveArtifacts { pattern } => Step {
            name: "archive-artifacts".to_string(),
            image: "alpine:3.18".to_string(),
            command: Some(vec!["tar".to_string(), "-czf".to_string(), "artifacts.tar.gz".to_string()]),
            args: vec![pattern.clone()],
            env: vec![],
            volume_mounts: vec![],
            script: None,
            working_dir: None,
            resources: None,
            security_context: None,
            timeout: None,
            ref_: None,
            results: vec![],
        },
        _ => Step {
            name: "custom-step".to_string(),
            image: "alpine:3.18".to_string(),
            command: Some(vec!["sh".to_string(), "-c".to_string(), "true".to_string()]),
            args: vec![],
            env: vec![],
            volume_mounts: vec![],
            script: None,
            working_dir: None,
            resources: None,
            security_context: None,
            timeout: None,
            ref_: None,
            results: vec![],
        },
    }
}

fn jenkins_when_to_cave(when: &JenkinsWhen) -> Vec<WhenExpression> {
    let mut exprs = Vec::new();
    if let Some(branch) = &when.branch {
        exprs.push(WhenExpression {
            input: "$(params.BRANCH_NAME)".to_string(),
            operator: WhenOperator::In,
            values: vec![branch.clone()],
        });
    }
    exprs
}

fn slug(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_JENKINSFILE: &str = r#"
pipeline {
    agent any

    environment {
        APP_NAME = 'my-app'
        REGISTRY = 'registry.example.com'
    }

    stages {
        stage('Checkout') {
            steps {
                checkout scm
            }
        }
        stage('Build') {
            steps {
                sh 'make build'
                sh 'docker build -t ${REGISTRY}/${APP_NAME}:${BUILD_NUMBER} .'
            }
        }
        stage('Test') {
            steps {
                sh 'make test'
                junit 'target/surefire-reports/*.xml'
            }
        }
    }

    post {
        always {
            sh 'make clean'
        }
        failure {
            echo 'Build failed!'
        }
    }
}
"#;

    #[test]
    fn parse_simple_jenkinsfile() {
        let jf = parse_jenkinsfile(SIMPLE_JENKINSFILE).unwrap();
        assert_eq!(jf.stages.len(), 3);
        assert_eq!(jf.stages[0].name, "Checkout");
        assert_eq!(jf.stages[1].name, "Build");
        assert_eq!(jf.stages[2].name, "Test");
    }

    #[test]
    fn parse_environment() {
        let jf = parse_jenkinsfile(SIMPLE_JENKINSFILE).unwrap();
        assert!(jf.environment.contains_key("APP_NAME"));
        assert_eq!(jf.environment.get("APP_NAME").map(|s| s.as_str()), Some("my-app"));
    }

    #[test]
    fn parse_post_always() {
        let jf = parse_jenkinsfile(SIMPLE_JENKINSFILE).unwrap();
        assert!(jf.post.iter().any(|p| p.condition == PostCondition::Always));
    }

    #[test]
    fn convert_to_pipeline_spec() {
        let jf = parse_jenkinsfile(SIMPLE_JENKINSFILE).unwrap();
        let spec = to_pipeline_spec(&jf);
        assert_eq!(spec.tasks.len(), 3);
        // Build runs after Checkout
        assert!(spec.tasks[1].run_after.contains(&"checkout".to_string()));
        // Test runs after Build
        assert!(spec.tasks[2].run_after.contains(&"build".to_string()));
    }

    #[test]
    fn convert_post_always_to_finally() {
        let jf = parse_jenkinsfile(SIMPLE_JENKINSFILE).unwrap();
        let spec = to_pipeline_spec(&jf);
        assert!(!spec.finally.is_empty());
    }

    #[test]
    fn slug_normalizes_names() {
        assert_eq!(slug("Build Image"), "build-image");
        assert_eq!(slug("Test & Deploy"), "test---deploy");
        assert_eq!(slug("run-tests"), "run-tests");
    }

    #[test]
    fn parse_agent_any() {
        let content = "pipeline { agent any }";
        let jf = parse_jenkinsfile(content).unwrap();
        assert!(matches!(jf.agent, AgentDeclaration::Any));
    }

    #[test]
    fn parse_agent_docker() {
        let content = "agent { docker { image: 'maven:3.8' } }";
        let agent = parse_agent_line("agent { docker { image: 'maven:3.8' } }");
        assert!(matches!(agent, AgentDeclaration::Docker { .. }));
    }
}
