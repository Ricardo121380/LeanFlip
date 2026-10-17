use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use leanflip_core::Tool;

#[derive(Parser)]
#[command(
    name = "leanflip",
    version,
    about = "Manage coding-tool providers with LeanFlip"
)]
pub struct Cli {
    /// Emit a JSON result; errors go to stderr with a stable error code.
    #[arg(long, global = true)]
    pub json: bool,
    /// Use this home directory (ignores tool-specific environment variables).
    #[arg(long, global = true, value_name = "DIR")]
    pub home: Option<PathBuf>,
    /// Language for terminal output and help; JSON messages stay unchanged.
    #[arg(long, global = true, value_enum, default_value = "zh")]
    pub lang: Language,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Print a shell completion script without reading or changing configuration.
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Open a searchable menu; optionally start with a specific tool.
    #[command(alias = "ui")]
    Menu { tool: Option<ToolArg> },
    /// Open the full-screen terminal workspace; optionally select a tool.
    Tui { tool: Option<ToolArg> },
    /// Show stored selection and detected configuration paths for all tools.
    Status,
    /// Preview the default Claude onboarding file; --apply explicitly marks it complete.
    ClaudeOnboarding {
        #[arg(long)]
        apply: bool,
    },
    /// List providers without exposing their configuration or credentials.
    List { tool: ToolArg },
    /// Activate a provider for a switch-style tool.
    Use { tool: ToolArg, provider_id: String },
    /// Write a provider into a coexist-style tool's live configuration.
    Enable { tool: ToolArg, provider_id: String },
    /// Remove a provider from live configuration; the saved provider is kept.
    Disable { tool: ToolArg, provider_id: String },
    /// Show effective paths, including App directory overrides.
    Paths { tool: Option<ToolArg> },
    /// Export an encrypted backup of providers, managed accounts and settings.
    Export {
        path: PathBuf,
        /// Read a passphrase from a private file; otherwise prompt without echo.
        #[arg(long, value_name = "FILE")]
        password_file: Option<PathBuf>,
    },
    /// Import existing client settings or a cc-switch export.
    Import {
        #[command(subcommand)]
        source: Import,
    },
    /// Add, update, or delete a saved provider.
    #[command(alias = "providers")]
    Provider {
        #[command(subcommand)]
        action: ProviderCommand,
    },
    /// Manage Codex ChatGPT accounts.
    Accounts {
        #[command(subcommand)]
        action: AccountCommand,
    },
    /// Explicitly recover interrupted writes; read-only commands never recover.
    Recover { tool: Option<ToolArg> },
    /// Show Codex and relay quota from this machine; --refresh queries online.
    Quota {
        /// Query managed Codex accounts and relays with a platform set, then save the result.
        #[arg(long)]
        refresh: bool,
    },
    /// List, preview, or apply scenes (create and edit them in the App).
    #[command(alias = "scenes")]
    Scene {
        #[command(subcommand)]
        action: SceneCommand,
    },
}

#[derive(Subcommand)]
pub enum SceneCommand {
    /// List scenes and whether each matches the current configuration.
    List,
    /// Preview what applying a scene would change; nothing is written.
    Show {
        /// Scene name or ID.
        scene: String,
    },
    /// Apply a scene to every tool it lists, or only to the given tools.
    Apply {
        /// Scene name or ID.
        scene: String,
        /// Apply only these tools; repeat or separate by commas.
        #[arg(long = "tool", value_delimiter = ',')]
        tools: Vec<ToolArg>,
    },
}

#[derive(Subcommand)]
pub enum Import {
    /// Detect local CC Switch data; preview first, then add --apply to migrate.
    Local {
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        apply: bool,
        /// Include managed Codex account credentials in the local migration.
        #[arg(long)]
        accounts: bool,
        #[arg(long = "tool", value_delimiter = ',')]
        tools: Vec<ToolArg>,
    },
    /// Unlock a LeanFlip backup and preview it; --apply restores selected data.
    Backup {
        path: PathBuf,
        #[arg(long)]
        apply: bool,
        #[arg(long, value_name = "FILE")]
        password_file: Option<PathBuf>,
        #[arg(long)]
        no_accounts: bool,
        /// Restore accounts/settings without any providers.
        #[arg(long, conflicts_with = "tools")]
        no_providers: bool,
        #[arg(long)]
        no_settings: bool,
        #[arg(long, conflicts_with = "no_settings")]
        restore_paths: bool,
        /// Fingerprint from the directory preview; required with --restore-paths --apply.
        #[arg(long, requires = "restore_paths")]
        confirm_paths: Option<String>,
        #[arg(long, value_enum, default_value = "skip")]
        duplicate: Duplicate,
        #[arg(long = "tool", value_delimiter = ',')]
        tools: Vec<ToolArg>,
    },
    /// Capture a tool's live configuration as saved providers.
    Live { tool: ToolArg },
    /// Preview a SQL, SQLite, or legacy JSON export; use --apply to save providers.
    File {
        path: PathBuf,
        #[arg(long, conflicts_with = "apply")]
        preview: bool,
        #[arg(long)]
        apply: bool,
        #[arg(long, value_enum, default_value = "skip")]
        duplicate: Duplicate,
        /// Limit preview and import to these tools; repeat or separate by commas.
        #[arg(long = "tool", value_delimiter = ',')]
        tools: Vec<ToolArg>,
    },
}

#[derive(Subcommand)]
pub enum ProviderCommand {
    /// Create a provider interactively, with a hidden key and a final preview.
    Create { tool: ToolArg },
    /// Review executable helpers, provider packages and destinations; approve the displayed fingerprint.
    TrustCommands {
        tool: ToolArg,
        provider_id: String,
        #[arg(long, value_name = "FINGERPRINT")]
        approve: Option<String>,
    },
    /// Edit a provider's name, Base URL and API Key in an interactive terminal.
    Edit { tool: ToolArg, provider_id: String },
    /// Save a new provider; does not activate it unless --activate is supplied.
    Add {
        tool: ToolArg,
        #[command(flatten)]
        input: ProviderInput,
        #[arg(long)]
        activate: bool,
    },
    /// Replace a provider using its JSON id; updates live config when active.
    Update {
        tool: ToolArg,
        #[command(flatten)]
        input: ProviderInput,
    },
    /// Delete a provider; an active switch-style provider must be switched away first.
    Delete { tool: ToolArg, provider_id: String },
}

#[derive(Args)]
pub struct ProviderInput {
    /// JSON provider file, or - for stdin. Credentials never belong in arguments.
    #[arg(long, value_name = "PATH", default_value = "-")]
    pub file: PathBuf,
}

#[derive(Subcommand)]
pub enum AccountCommand {
    /// List account identities and state, without tokens.
    List,
    /// Sign in with a device code; prompts go to stderr in JSON mode.
    Login {
        /// Reauthenticate an existing account.
        #[arg(long)]
        account: Option<String>,
        /// Maximum wait in seconds (network calls also have a bounded timeout).
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=1800))]
        timeout: u64,
    },
    /// Choose the default managed account.
    Default { account_id: String },
    /// Remove a managed account; does not revoke the account at OpenAI.
    Remove { account_id: String },
    /// Force a token refresh for one account without printing its credentials.
    Refresh { account_id: String },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Duplicate {
    Skip,
    Rename,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Language {
    Zh,
    En,
}

impl Language {
    pub fn text<'a>(self, zh: &'a str, en: &'a str) -> &'a str {
        match self {
            Self::Zh => zh,
            Self::En => en,
        }
    }
}

impl From<Duplicate> for leanflip_core::ccswitch_export::DuplicatePolicy {
    fn from(value: Duplicate) -> Self {
        match value {
            Duplicate::Skip => Self::Skip,
            Duplicate::Rename => Self::Rename,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ToolArg {
    Claude,
    Codex,
    Gemini,
    #[value(name = "grokbuild", alias = "grok-build")]
    GrokBuild,
    ClaudeDesktop,
    #[value(name = "opencode")]
    OpenCode,
    #[value(name = "openclaw")]
    OpenClaw,
    Hermes,
    Pi,
    Mcode,
}

impl From<ToolArg> for Tool {
    fn from(value: ToolArg) -> Self {
        match value {
            ToolArg::Claude => Self::Claude,
            ToolArg::Codex => Self::Codex,
            ToolArg::Gemini => Self::Gemini,
            ToolArg::GrokBuild => Self::GrokBuild,
            ToolArg::ClaudeDesktop => Self::ClaudeDesktop,
            ToolArg::OpenCode => Self::OpenCode,
            ToolArg::OpenClaw => Self::OpenClaw,
            ToolArg::Hermes => Self::Hermes,
            ToolArg::Pi => Self::Pi,
            ToolArg::Mcode => Self::Mcode,
        }
    }
}

impl From<Tool> for ToolArg {
    fn from(value: Tool) -> Self {
        match value {
            Tool::Claude => Self::Claude,
            Tool::Codex => Self::Codex,
            Tool::Gemini => Self::Gemini,
            Tool::GrokBuild => Self::GrokBuild,
            Tool::ClaudeDesktop => Self::ClaudeDesktop,
            Tool::OpenCode => Self::OpenCode,
            Tool::OpenClaw => Self::OpenClaw,
            Tool::Hermes => Self::Hermes,
            Tool::Pi => Self::Pi,
            Tool::Mcode => Self::Mcode,
        }
    }
}
