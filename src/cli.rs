use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Unearth TODO comments, connect issues, and generate reports"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    /// Scan this directory instead of the containing repository.
    pub path: Option<PathBuf>,
    #[arg(long, value_enum)]
    pub format: Option<Format>,
    /// Write a report; '-' means stdout. Format is inferred from the extension.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Open a temporary HTML preview in the default browser.
    #[arg(long, conflicts_with_all = ["format", "output", "check"])]
    pub open: bool,
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long, value_enum)]
    pub detail: Option<Detail>,
    #[arg(long, value_enum)]
    pub group_by: Option<GroupBy>,
    /// Group findings inside each component (default: file).
    #[arg(long, value_enum)]
    pub component_group_by: Option<InnerGroupBy>,
    /// Arrange component folders as a flat list or a directory tree.
    #[arg(long, value_enum)]
    pub component_layout: Option<ComponentLayout>,
    /// Console/text width in columns (minimum 20); console defaults to terminal width.
    #[arg(long, value_parser = clap::value_parser!(u16).range(20..))]
    pub width: Option<u16>,
    /// Blank rows between every console line (default: 1; 0 is compact).
    #[arg(long)]
    pub line_spacing: Option<u16>,
    /// Literal markers, separated by commas (replaces defaults).
    #[arg(long, value_delimiter = ',')]
    pub markers: Option<Vec<String>>,
    #[arg(long)]
    pub include: Vec<String>,
    #[arg(long)]
    pub exclude: Vec<String>,
    #[arg(long)]
    pub no_default_excludes: bool,
    #[arg(long)]
    pub no_ignore: bool,
    #[arg(long)]
    pub no_fallback: bool,
    #[arg(short, long)]
    pub jobs: Option<usize>,
    #[arg(long)]
    pub max_file_size: Option<u64>,
    #[arg(long, conflicts_with = "no_github")]
    pub github: bool,
    #[arg(long)]
    pub no_github: bool,
    /// Repository for shorthand issue references, in owner/repo form.
    #[arg(long)]
    pub repository: Option<String>,
    #[arg(long)]
    pub github_host: Option<String>,
    #[arg(long)]
    pub api_url: Option<String>,
    #[arg(long, value_enum)]
    pub attribution: Option<AttributionMode>,
    #[arg(long)]
    pub history_limit: Option<usize>,
    #[arg(long)]
    pub git_timeout: Option<u64>,
    #[arg(long, action = clap::ArgAction::Set)]
    pub git_ignore_whitespace: Option<bool>,
    #[arg(long, action = clap::ArgAction::Set)]
    pub git_detect_moves: Option<bool>,
    #[arg(long, action = clap::ArgAction::Set)]
    pub git_detect_copies: Option<bool>,
    /// Compare an exported report without changing it (default: TODO.md).
    #[arg(long)]
    pub check: bool,
    #[arg(long, value_enum)]
    pub compare: Option<Compare>,
    /// Fail when requested enrichment or a dependent policy cannot complete.
    #[arg(long)]
    pub strict: bool,
    #[arg(long)]
    pub force: bool,
    #[arg(long, value_enum)]
    pub links: Option<Links>,
    #[arg(long)]
    pub timings: bool,
    #[arg(long)]
    pub max_total: Option<usize>,
    #[arg(long)]
    pub fail_on_closed: bool,
    #[arg(long)]
    pub fail_on_unavailable: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a documented repository configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Generate shell completions.
    Completions { shell: Shell },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Init {
        #[arg(long)]
        force: bool,
    },
}

macro_rules! choices {
    ($name:ident, $default:ident, $($variant:ident),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            #[default]
            $default,
            $($variant),+
        }
    };
}
choices!(Format, Console, Text, Markdown, Json, Html);
choices!(Detail, Full, Summary);
choices!(GroupBy, File, Marker, Component);
choices!(InnerGroupBy, File, Marker);
choices!(ComponentLayout, Flat, Tree);

impl From<InnerGroupBy> for GroupBy {
    fn from(value: InnerGroupBy) -> Self {
        match value {
            InnerGroupBy::Marker => Self::Marker,
            InnerGroupBy::File => Self::File,
        }
    }
}
choices!(AttributionMode, Off, Blame, History);
choices!(Compare, Source, Full);
choices!(Links, Auto, Enabled, Disabled);
