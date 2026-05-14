use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

fn parse_tail_lines(value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("invalid tail line count '{value}'"))?;
    if parsed == 0 {
        return Err("tail line count must be greater than zero".to_owned());
    }
    Ok(parsed)
}

#[derive(Debug, Parser)]
#[command(
    name = "previa",
    version,
    about = "CLI local para operar contexts do Previa"
)]
pub struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    pub home: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    #[command(about = "Authenticate to a protected Previa main and store an API token")]
    Login(LoginArgs),
    #[command(about = "Remove a stored Previa API token")]
    Logout(AuthContextArgs),
    #[command(about = "Show the authenticated Previa principal")]
    Whoami(AuthContextArgs),
    #[command(about = "Manage fixed API tokens")]
    Token(TokenArgs),
    #[command(about = "Create a starter previa-compose.yaml in the current directory")]
    Init(InitArgs),
    #[command(about = "Run Previa commands with project-local home ./.previa")]
    Local(LocalArgs),
    #[command(about = "Start a Previa context")]
    Up(UpArgs),
    #[command(about = "Install and inspect MCP client configuration")]
    Mcp(McpArgs),
    #[command(about = "Manage registered runner endpoints")]
    Runner(RunnerArgs),
    #[command(about = "Pull published runtime images")]
    Pull(PullArgs),
    #[command(about = "Stop a detached context or selected local runners")]
    Down(DownArgs),
    #[command(about = "Restart a detached context")]
    Restart(RestartArgs),
    #[command(about = "Show the current state of a context")]
    Status(StatusArgs),
    #[command(about = "List known contexts")]
    List(ListArgs),
    #[command(about = "Show recorded processes for a context")]
    Ps(PsArgs),
    #[command(about = "Read logs from a detached context")]
    Logs(LogsArgs),
    #[command(about = "Open the Previa IDE with the current context")]
    Open(OpenArgs),
    #[command(about = "Export stored resources from a detached context")]
    Export(ExportArgs),
    #[command(about = "Print the CLI version")]
    Version,
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        conflicts_with = "url",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long = "url", value_name = "URL", conflicts_with = "context")]
    pub url: Option<String>,
    #[arg(long = "username", value_name = "USERNAME")]
    pub username: String,
    #[arg(long = "password-stdin")]
    pub password_stdin: bool,
}

#[derive(Debug, Args)]
pub struct AuthContextArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        conflicts_with = "url",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long = "url", value_name = "URL", conflicts_with = "context")]
    pub url: Option<String>,
}

#[derive(Debug, Args)]
pub struct TokenArgs {
    #[command(subcommand)]
    pub command: TokenCommands,
}

#[derive(Debug, Subcommand)]
pub enum TokenCommands {
    #[command(about = "List API tokens")]
    List(TokenListArgs),
    #[command(about = "Create a fixed API token")]
    Create(TokenCreateArgs),
    #[command(about = "Revoke a fixed API token")]
    Revoke(TokenRevokeArgs),
    #[command(about = "Store an API token from an environment variable")]
    Use(TokenUseArgs),
}

#[derive(Debug, Args)]
pub struct TokenListArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        conflicts_with = "url",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long = "url", value_name = "URL", conflicts_with = "context")]
    pub url: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct TokenCreateArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        conflicts_with = "url",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long = "url", value_name = "URL", conflicts_with = "context")]
    pub url: Option<String>,
    #[arg(long = "name", value_name = "NAME")]
    pub name: String,
    #[arg(long = "role", value_name = "ROLE", default_value = "viewer")]
    pub role: String,
}

#[derive(Debug, Args)]
pub struct TokenRevokeArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        conflicts_with = "url",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long = "url", value_name = "URL", conflicts_with = "context")]
    pub url: Option<String>,
    #[arg(value_name = "TOKEN_ID")]
    pub token_id: String,
}

#[derive(Debug, Args)]
pub struct TokenUseArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        conflicts_with = "url",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long = "url", value_name = "URL", conflicts_with = "context")]
    pub url: Option<String>,
    #[arg(long = "token-env", value_name = "ENV_NAME")]
    pub token_env: String,
}

#[derive(Debug, Args)]
#[command(about = "Run Previa commands with project-local home ./.previa")]
pub struct LocalArgs {
    #[command(subcommand)]
    pub command: LocalCommands,
}

#[derive(Debug, Subcommand)]
pub enum LocalCommands {
    #[command(about = "Start a project-local Previa context")]
    Up(UpArgs),
    #[command(about = "Push a project-local Previa project to a remote Previa main")]
    Push(LocalPushArgs),
    #[command(about = "Import every project from a SQLite export into a project-local context")]
    Import(LocalImportArgs),
    #[command(about = "Export selected project-local projects into a SQLite database")]
    Export(LocalExportArgs),
    #[command(about = "Manage registered runner endpoints in a project-local context")]
    Runner(RunnerArgs),
    #[command(about = "Stop a project-local detached context or selected local runners")]
    Down(DownArgs),
    #[command(about = "Show the current state of a project-local context")]
    Status(StatusArgs),
    #[command(about = "Read logs from a project-local detached context")]
    Logs(LogsArgs),
    #[command(about = "Open the Previa IDE with the project-local context")]
    Open(OpenArgs),
}

#[derive(Debug, Args)]
#[command(about = "Import every project from a SQLite export into a project-local context")]
pub struct LocalImportArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Context name"
    )]
    pub context: String,
    #[arg(value_name = "DB_SQLITE3")]
    pub path: PathBuf,
    #[arg(long = "no-history")]
    pub no_history: bool,
}

#[derive(Debug, Args)]
#[command(about = "Export selected project-local projects into a SQLite database")]
pub struct LocalExportArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long = "all", conflicts_with = "projects")]
    pub all: bool,
    #[arg(long = "project", value_name = "PROJECT_ID", conflicts_with = "all")]
    pub projects: Vec<String>,
    #[arg(short = 'o', long = "output", value_name = "DB_SQLITE3")]
    pub output: PathBuf,
    #[arg(long = "overwrite")]
    pub overwrite: bool,
    #[arg(long = "no-history")]
    pub no_history: bool,
}

#[derive(Debug, Args)]
#[command(about = "Manage registered runner endpoints")]
pub struct RunnerArgs {
    #[command(subcommand)]
    pub command: RunnerCommands,
}

#[derive(Debug, Subcommand)]
pub enum RunnerCommands {
    #[command(about = "List registered runners")]
    List(RunnerListArgs),
    #[command(about = "Add or update a registered runner")]
    Add(RunnerAddArgs),
    #[command(about = "Enable a registered runner")]
    Enable(RunnerSelectorArgs),
    #[command(about = "Disable a registered runner")]
    Disable(RunnerSelectorArgs),
    #[command(about = "Remove a registered runner")]
    Remove(RunnerSelectorArgs),
}

#[derive(Debug, Args)]
pub struct RunnerListArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RunnerAddArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Context name"
    )]
    pub context: String,
    #[arg(value_name = "ENDPOINT")]
    pub endpoint: String,
    #[arg(long = "name", value_name = "NAME")]
    pub name: Option<String>,
    #[arg(long = "disabled")]
    pub disabled: bool,
}

#[derive(Debug, Args)]
pub struct RunnerSelectorArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Context name"
    )]
    pub context: String,
    #[arg(value_name = "ID_ENDPOINT_OR_NAME")]
    pub selector: String,
}

#[derive(Debug, Args)]
#[command(about = "Push a project-local Previa project to a remote Previa main")]
pub struct LocalPushArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Local context name"
    )]
    pub context: String,
    #[arg(long = "project", value_name = "ID_OR_NAME")]
    pub project: String,
    #[arg(long = "to", value_name = "REMOTE_URL")]
    pub to: String,
    #[arg(long = "remote-project-id", value_name = "PROJECT_ID")]
    pub remote_project_id: Option<String>,
    #[arg(long = "overwrite")]
    pub overwrite: bool,
    #[arg(long = "include-history")]
    pub include_history: bool,
}

#[derive(Debug, Args)]
#[command(about = "Install and inspect MCP client configuration")]
pub struct McpArgs {
    #[command(subcommand)]
    pub action: McpAction,
}

#[derive(Debug, Subcommand)]
pub enum McpAction {
    #[command(about = "Install Previa MCP into a supported client")]
    Install(McpInstallArgs),
    #[command(about = "Remove Previa MCP from a supported client")]
    Uninstall(McpUninstallArgs),
    #[command(about = "Show current Previa MCP configuration for a supported client")]
    Status(McpStatusArgs),
    #[command(about = "Print the MCP snippet or command for a supported client")]
    Print(McpPrintArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum McpTarget {
    Codex,
    Cursor,
    ClaudeDesktop,
    ClaudeCode,
    Warp,
    CopilotVscode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum McpScope {
    Global,
    Project,
}

#[derive(Debug, Args)]
#[command(about = "Install Previa MCP into a supported client")]
pub struct McpInstallArgs {
    #[arg(value_enum)]
    pub target: McpTarget,
    #[arg(long = "context", value_name = "CONTEXT", conflicts_with = "url")]
    pub context: Option<String>,
    #[arg(long = "url", value_name = "MCP_URL", conflicts_with = "context")]
    pub url: Option<String>,
    #[arg(long = "scope", value_enum, default_value_t = McpScope::Global)]
    pub scope: McpScope,
    #[arg(long = "name", value_name = "SERVER_NAME", default_value = "previa")]
    pub name: String,
    #[arg(long = "token-env", value_name = "ENV_NAME")]
    pub token_env: Option<String>,
    #[arg(long = "force")]
    pub force: bool,
    #[arg(long = "no-verify")]
    pub no_verify: bool,
}

#[derive(Debug, Args)]
#[command(about = "Remove Previa MCP from a supported client")]
pub struct McpUninstallArgs {
    #[arg(value_enum)]
    pub target: McpTarget,
    #[arg(long = "scope", value_enum, default_value_t = McpScope::Global)]
    pub scope: McpScope,
    #[arg(long = "name", value_name = "SERVER_NAME", default_value = "previa")]
    pub name: String,
}

#[derive(Debug, Args)]
#[command(about = "Show current Previa MCP configuration for a supported client")]
pub struct McpStatusArgs {
    #[arg(value_enum)]
    pub target: McpTarget,
    #[arg(long = "scope", value_enum, default_value_t = McpScope::Global)]
    pub scope: McpScope,
    #[arg(long = "name", value_name = "SERVER_NAME", default_value = "previa")]
    pub name: String,
    #[arg(long = "token-env", value_name = "ENV_NAME")]
    pub token_env: Option<String>,
}

#[derive(Debug, Args)]
#[command(about = "Print the MCP snippet or command for a supported client")]
pub struct McpPrintArgs {
    #[arg(value_enum)]
    pub target: McpTarget,
    #[arg(long = "context", value_name = "CONTEXT", conflicts_with = "url")]
    pub context: Option<String>,
    #[arg(long = "url", value_name = "MCP_URL", conflicts_with = "context")]
    pub url: Option<String>,
    #[arg(long = "scope", value_enum, default_value_t = McpScope::Global)]
    pub scope: McpScope,
    #[arg(long = "name", value_name = "SERVER_NAME", default_value = "previa")]
    pub name: String,
    #[arg(long = "token-env", value_name = "ENV_NAME")]
    pub token_env: Option<String>,
}

#[derive(Debug, Args)]
#[command(about = "Create a starter previa-compose.yaml in the current directory")]
pub struct InitArgs {
    #[arg(long = "force")]
    pub force: bool,
}

#[derive(Debug, Args)]
#[command(about = "Export stored resources from a detached context")]
pub struct ExportArgs {
    #[command(subcommand)]
    pub target: ExportTarget,
}

#[derive(Debug, Subcommand)]
pub enum ExportTarget {
    #[command(about = "Export stored project pipelines into local files")]
    Pipelines(PipelineExportArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PipelineExportFormat {
    Yaml,
    Json,
}

#[derive(Debug, Args)]
#[command(about = "Export stored project pipelines into local files")]
pub struct PipelineExportArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long = "project", value_name = "ID_OR_NAME")]
    pub project: String,
    #[arg(long = "output-dir", value_name = "PATH")]
    pub output_dir: PathBuf,
    #[arg(long = "pipeline", value_name = "ID_OR_NAME")]
    pub pipelines: Vec<String>,
    #[arg(long = "format", value_enum, default_value_t = PipelineExportFormat::Yaml)]
    pub format: PipelineExportFormat,
    #[arg(long = "overwrite")]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PullTarget {
    Main,
    Runner,
    All,
}

#[derive(Debug, Args)]
#[command(about = "Pull published runtime images")]
pub struct PullArgs {
    #[arg(value_enum, default_value_t = PullTarget::All)]
    pub target: PullTarget,
    #[arg(long, default_value = env!("CARGO_PKG_VERSION"))]
    pub version: String,
}

#[derive(Debug, Args)]
#[command(about = "Start a Previa context")]
pub struct UpArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Context name"
    )]
    pub context: String,
    pub source: Option<String>,
    #[arg(long)]
    pub main_address: Option<String>,
    #[arg(short = 'p', long)]
    pub main_port: Option<u16>,
    #[arg(long)]
    pub runner_address: Option<String>,
    #[arg(short = 'P', long = "runner-port-range")]
    pub runner_port_range: Option<String>,
    #[arg(long)]
    pub runners: Option<usize>,
    #[arg(short = 'i', long = "import", value_name = "PATH")]
    pub import_path: Option<String>,
    #[arg(short = 'r', long)]
    pub recursive: bool,
    #[arg(short = 's', long = "stack", value_name = "STACK")]
    pub stack: Option<String>,
    #[arg(short = 'a', long = "attach-runner")]
    pub attach_runners: Vec<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(short = 'd', long)]
    pub detach: bool,
    #[arg(long = "protected", conflicts_with = "anonymous")]
    pub protected: bool,
    #[arg(long = "anonymous", conflicts_with = "protected")]
    pub anonymous: bool,
    #[arg(long = "root-username", value_name = "USERNAME")]
    pub root_username: Option<String>,
    #[arg(long = "root-password-stdin")]
    pub root_password_stdin: bool,
    #[arg(skip = None)]
    pub root_password: Option<String>,
    #[cfg(target_os = "linux")]
    #[arg(long = "bin")]
    pub bin: bool,
    #[arg(long, default_value = env!("CARGO_PKG_VERSION"))]
    pub version: String,
}

impl UpArgs {
    pub fn bin_requested(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.bin
        }

        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }
}

#[derive(Debug, Args)]
#[command(about = "Stop a detached context or selected local runners")]
pub struct DownArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long = "all-contexts")]
    pub all_context: bool,
    #[arg(long = "runner")]
    pub runners: Vec<String>,
}

#[derive(Debug, Args)]
#[command(about = "Restart a detached context")]
pub struct RestartArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long)]
    pub version: Option<String>,
}

#[derive(Debug, Args)]
#[command(about = "Show the current state of a context")]
pub struct StatusArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long)]
    pub main: bool,
    #[arg(long)]
    pub runner: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
#[command(about = "List known contexts")]
pub struct ListArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
#[command(about = "Show recorded processes for a context")]
pub struct PsArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
#[command(about = "Read logs from a detached context")]
pub struct LogsArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long)]
    pub main: bool,
    #[arg(long)]
    pub runner: Option<String>,
    #[arg(long)]
    pub follow: bool,
    #[arg(
        short = 't',
        long,
        num_args = 0..=1,
        default_missing_value = "10",
        value_parser = parse_tail_lines
    )]
    pub tail: Option<usize>,
}

#[derive(Debug, Args)]
#[command(about = "Open the Previa IDE with the current context")]
pub struct OpenArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Context name"
    )]
    pub context: String,
}

#[cfg(test)]
mod tests {
    use super::{Cli, Commands, TokenCommands};
    use clap::Parser;

    #[test]
    fn parses_local_up() {
        let cli = Cli::try_parse_from(["previa", "local", "up", "-d"]).expect("parse local up");

        assert!(cli.home.is_none());
        let Commands::Local(local) = cli.command else {
            panic!("expected local command");
        };
        let super::LocalCommands::Up(args) = local.command else {
            panic!("expected local up");
        };
        assert!(args.detach);
        assert_eq!(args.context, "default");
    }

    #[test]
    fn parses_protected_up_flags() {
        let cli = Cli::try_parse_from([
            "previa",
            "up",
            "--protected",
            "--root-username",
            "admin",
            "--root-password-stdin",
            "-d",
        ])
        .expect("parse protected up");

        let Commands::Up(args) = cli.command else {
            panic!("expected up command");
        };
        assert!(args.protected);
        assert!(!args.anonymous);
        assert_eq!(args.root_username.as_deref(), Some("admin"));
        assert!(args.root_password_stdin);
        assert!(args.detach);
    }

    #[test]
    fn parses_login_with_password_stdin() {
        let cli = Cli::try_parse_from([
            "previa",
            "login",
            "--context",
            "default",
            "--username",
            "root",
            "--password-stdin",
        ])
        .expect("parse login");

        let Commands::Login(args) = cli.command else {
            panic!("expected login command");
        };
        assert_eq!(args.context, "default");
        assert_eq!(args.username, "root");
        assert!(args.password_stdin);
    }

    #[test]
    fn parses_token_use_with_env() {
        let cli = Cli::try_parse_from([
            "previa",
            "token",
            "use",
            "--context",
            "default",
            "--token-env",
            "PREVIA_API_TOKEN",
        ])
        .expect("parse token use");

        let Commands::Token(args) = cli.command else {
            panic!("expected token command");
        };
        let TokenCommands::Use(args) = args.command else {
            panic!("expected token use");
        };
        assert_eq!(args.context, "default");
        assert_eq!(args.token_env, "PREVIA_API_TOKEN");
    }

    #[test]
    fn parses_token_create_with_role() {
        let cli = Cli::try_parse_from([
            "previa",
            "token",
            "create",
            "--context",
            "default",
            "--name",
            "ci",
            "--role",
            "operator",
        ])
        .expect("parse token create");

        let Commands::Token(args) = cli.command else {
            panic!("expected token command");
        };
        let TokenCommands::Create(args) = args.command else {
            panic!("expected token create");
        };
        assert_eq!(args.context, "default");
        assert_eq!(args.name, "ci");
        assert_eq!(args.role, "operator");
    }

    #[test]
    fn parses_mcp_install_with_token_env() {
        let cli = Cli::try_parse_from([
            "previa",
            "mcp",
            "install",
            "codex",
            "--token-env",
            "PREVIA_API_TOKEN",
            "--no-verify",
        ])
        .expect("parse mcp install");

        let Commands::Mcp(args) = cli.command else {
            panic!("expected mcp command");
        };
        let super::McpAction::Install(args) = args.action else {
            panic!("expected mcp install");
        };
        assert_eq!(args.token_env.as_deref(), Some("PREVIA_API_TOKEN"));
    }

    #[test]
    fn parses_local_status() {
        let cli = Cli::try_parse_from(["previa", "local", "status"]).expect("parse local status");

        assert!(cli.home.is_none());
        let Commands::Local(local) = cli.command else {
            panic!("expected local command");
        };
        let super::LocalCommands::Status(args) = local.command else {
            panic!("expected local status");
        };
        assert_eq!(args.context, "default");
        assert!(!args.json);
    }

    #[test]
    fn preserves_explicit_home_for_local_command() {
        let cli = Cli::try_parse_from(["previa", "--home", "./custom", "local", "status"])
            .expect("parse local status with home");

        assert_eq!(cli.home.as_deref(), Some(std::path::Path::new("./custom")));
        assert!(matches!(cli.command, Commands::Local(_)));
    }

    #[test]
    fn parses_local_push() {
        let cli = Cli::try_parse_from([
            "previa",
            "local",
            "push",
            "--project",
            "my_app",
            "--to",
            "https://remote.example",
            "--overwrite",
            "--include-history",
        ])
        .expect("parse local push");

        let Commands::Local(local) = cli.command else {
            panic!("expected local command");
        };
        let super::LocalCommands::Push(args) = local.command else {
            panic!("expected local push");
        };
        assert_eq!(args.context, "default");
        assert_eq!(args.project, "my_app");
        assert_eq!(args.to, "https://remote.example");
        assert!(args.overwrite);
        assert!(args.include_history);
    }

    #[test]
    fn parses_local_import() {
        let cli = Cli::try_parse_from(["previa", "local", "import", "./db.sqlite3"])
            .expect("parse local import");

        let Commands::Local(local) = cli.command else {
            panic!("expected local command");
        };
        let super::LocalCommands::Import(args) = local.command else {
            panic!("expected local import");
        };
        assert_eq!(args.context, "default");
        assert_eq!(args.path, std::path::PathBuf::from("./db.sqlite3"));
        assert!(!args.no_history);
    }

    #[test]
    fn parses_local_export_all() {
        let cli = Cli::try_parse_from([
            "previa",
            "local",
            "export",
            "--all",
            "--output",
            "./db.sqlite3",
        ])
        .expect("parse local export");

        let Commands::Local(local) = cli.command else {
            panic!("expected local command");
        };
        let super::LocalCommands::Export(args) = local.command else {
            panic!("expected local export");
        };
        assert_eq!(args.context, "default");
        assert!(args.all);
        assert!(args.projects.is_empty());
        assert_eq!(args.output, std::path::PathBuf::from("./db.sqlite3"));
    }

    #[test]
    fn parses_runner_add() {
        let cli = Cli::try_parse_from([
            "previa",
            "runner",
            "add",
            "localhost:5590",
            "--name",
            "local-a",
        ])
        .expect("parse runner add");

        let Commands::Runner(args) = cli.command else {
            panic!("expected runner command");
        };
        let super::RunnerCommands::Add(args) = args.command else {
            panic!("expected runner add");
        };
        assert_eq!(args.context, "default");
        assert_eq!(args.endpoint, "localhost:5590");
        assert_eq!(args.name.as_deref(), Some("local-a"));
        assert!(!args.disabled);
    }
}
