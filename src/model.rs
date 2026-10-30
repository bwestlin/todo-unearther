use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;
pub const GENERATOR: &str = "todo-unearther";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Metadata {
    pub generator: String,
    pub schema_version: u32,
    pub source_fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Finding {
    pub path: String,
    pub line: usize,
    pub end_line: usize,
    pub marker: String,
    pub annotation: Option<String>,
    pub message: String,
    pub detection: Detection,
    pub references: Vec<IssueReference>,
    pub attribution: Option<Attribution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ecosystem {
    Cargo,
    Npm,
    Python,
    Go,
    Maven,
    Gradle,
    Dotnet,
}

impl Ecosystem {
    pub const ALL: [Self; 7] = [
        Self::Cargo,
        Self::Npm,
        Self::Python,
        Self::Go,
        Self::Maven,
        Self::Gradle,
        Self::Dotnet,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Cargo => "Cargo",
            Self::Npm => "npm",
            Self::Python => "Python",
            Self::Go => "Go",
            Self::Maven => "Maven",
            Self::Gradle => "Gradle",
            Self::Dotnet => ".NET",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentStatus {
    Detected,
    Inferred,
    Unresolved,
    Shared,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ComponentIdentity {
    pub ecosystem: Ecosystem,
    pub name: String,
    pub manifest: String,
    pub status: ComponentStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Component {
    pub id: String,
    pub root: String,
    pub identities: Vec<ComponentIdentity>,
    pub status: ComponentStatus,
}

impl Component {
    pub fn heading(&self) -> String {
        if self.status == ComponentStatus::Shared {
            return "Repository / shared".into();
        }
        let identities = self
            .identities
            .iter()
            .map(|identity| format!("{} [{}]", identity.name, identity.ecosystem.label()))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(" + ");
        let identities = if identities.is_empty() {
            "Unknown component"
        } else {
            &identities
        };
        let suffix = match self.status {
            ComponentStatus::Unresolved => " · unresolved",
            ComponentStatus::Inferred => " · inferred",
            _ => "",
        };
        format!("{} — {identities}{suffix}", self.root)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Detection {
    Comment,
    Heuristic,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueReference {
    pub raw: String,
    pub host: Option<String>,
    pub repository: Option<String>,
    pub number: u64,
    pub url: Option<String>,
    pub status: ReferenceStatus,
    pub issue: Option<IssueMetadata>,
    pub error: Option<String>,
}

impl IssueReference {
    pub fn key(&self) -> Option<String> {
        Some(format!(
            "{}/{}#{}",
            self.host.as_ref()?.to_ascii_lowercase(),
            self.repository.as_ref()?.to_ascii_lowercase(),
            self.number
        ))
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceStatus {
    NotRequested,
    Unresolved,
    Available,
    Open,
    Closed,
    Unavailable,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueMetadata {
    pub title: String,
    pub kind: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attribution {
    pub blame: Option<GitAuthor>,
    pub origin: Option<GitAuthor>,
    pub origin_status: String,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitAuthor {
    pub name: String,
    pub email: String,
    pub commit: String,
    /// Unix author timestamp; independent of the reporting machine's timezone.
    pub author_time: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanStats {
    pub files_read: usize,
    pub candidate_files: usize,
    pub bytes_read: u64,
    pub skipped_binary: usize,
    pub skipped_encoding: usize,
    pub skipped_oversized: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Diagnostic {
    pub stage: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyViolation {
    pub policy: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanReport {
    pub metadata: Metadata,
    pub findings: Vec<Finding>,
    pub counts: BTreeMap<String, usize>,
    pub stats: ScanStats,
    pub diagnostics: Vec<Diagnostic>,
    pub violations: Vec<PolicyViolation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<Component>,
}

impl ScanReport {
    pub fn warn(&mut self, stage: &str, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            stage: stage.into(),
            message: message.into(),
        });
    }
}
