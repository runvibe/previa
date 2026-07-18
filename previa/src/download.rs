use std::env;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use reqwest::Client;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::paths::PreviaPaths;

const DEFAULT_DOWNLOAD_BASE_URL: &str = "https://github.com/runvibe/previa/releases/download";
const DOWNLOAD_BASE_URL_ENV: &str = "PREVIA_DOWNLOAD_BASE_URL";
const LEGACY_MANIFEST_URL_ENV: &str = "PREVIA_DOWNLOAD_MANIFEST_URL";
#[cfg(test)]
const TEST_DOWNLOAD_OS_ENV: &str = "PREVIA_TEST_DOWNLOAD_OS";
#[cfg(test)]
const TEST_DOWNLOAD_ARCH_ENV: &str = "PREVIA_TEST_DOWNLOAD_ARCH";

#[cfg(test)]
pub(crate) static DOWNLOAD_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub async fn ensure_runtime_binaries(paths: &PreviaPaths, local_runner_count: usize) -> Result<()> {
    let mut required = Vec::new();
    if should_install_binary(paths, "previa-main")? {
        required.push("previa-main");
    }
    if local_runner_count > 0 && should_install_binary(paths, "previa-runner")? {
        required.push("previa-runner");
    }

    if required.is_empty() {
        return Ok(());
    }

    let client = build_download_client()?;
    let version = current_release_version();

    for binary_name in required {
        let mut reporter = DownloadReporter::for_stderr();
        download_binary(&client, paths, binary_name, version, &mut reporter).await?;
    }

    Ok(())
}

fn current_release_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn build_download_client() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .build()
        .context("failed to build binary download HTTP client")
}

fn download_base_url() -> String {
    if let Some(value) = env::var(DOWNLOAD_BASE_URL_ENV)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_owned())
        .filter(|value| !value.is_empty())
    {
        return value;
    }

    if let Some(value) = env::var(LEGACY_MANIFEST_URL_ENV).ok() {
        if let Some(base_url) = legacy_manifest_base_url(&value) {
            return base_url;
        }
    }

    DEFAULT_DOWNLOAD_BASE_URL.to_owned()
}

fn legacy_manifest_base_url(manifest_url: &str) -> Option<String> {
    let manifest_url = manifest_url.trim();
    if manifest_url.is_empty() {
        return None;
    }

    if let Some(path) = manifest_url
        .strip_prefix("https://raw.githubusercontent.com/")
        .or_else(|| manifest_url.strip_prefix("http://raw.githubusercontent.com/"))
    {
        let mut parts = path.split('/');
        if let (Some(owner), Some(repo), Some(_git_ref), Some(_filename)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        {
            return Some(format!(
                "https://github.com/{owner}/{repo}/releases/download"
            ));
        }
    }

    Some(
        manifest_url
            .trim_end_matches("/release-metadata.json")
            .trim_end_matches("/latest.json")
            .trim_end_matches('/')
            .to_owned(),
    )
}

async fn download_binary(
    client: &Client,
    paths: &PreviaPaths,
    binary_name: &str,
    version: &str,
    reporter: &mut impl ProgressReporter,
) -> Result<PathBuf> {
    let (os_slug, arch_slug) = normalized_platform()?;
    let asset_name = asset_filename(binary_name, &os_slug, &arch_slug)?;
    let url = binary_download_url(version, &asset_name);
    let target_path = binary_install_path(paths, binary_name);

    if target_path.exists() && binary_matches_version(&target_path, version) {
        return Ok(target_path);
    }

    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create '{}'", parent.display()))?;
    }

    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| {
            format!(
                "failed to download binary '{binary_name}' for CLI version '{version}' from '{url}'"
            )
        })?
        .error_for_status()
        .with_context(|| {
            format!(
                "failed to download binary '{binary_name}' for CLI version '{version}' from '{url}'"
            )
        })?;

    reporter.begin(binary_name, version, response.content_length());

    let temp_path = temporary_download_path(&target_path);
    let result =
        async {
            let mut file = fs::File::create(&temp_path).await.with_context(|| {
                format!(
                    "failed to create temporary file for binary '{}': '{}'",
                    binary_name,
                    temp_path.display()
                )
            })?;

            let mut response = response;
            while let Some(chunk) = response.chunk().await.with_context(|| {
                format!("failed to read download stream for binary '{binary_name}'")
            })? {
                file.write_all(&chunk).await.with_context(|| {
                    format!(
                        "failed to write downloaded bytes for binary '{}': '{}'",
                        binary_name,
                        temp_path.display()
                    )
                })?;
                reporter.advance(chunk.len() as u64);
            }

            file.flush().await.with_context(|| {
                format!(
                    "failed to flush temporary file for binary '{}': '{}'",
                    binary_name,
                    temp_path.display()
                )
            })?;
            drop(file);

            set_executable(&temp_path)?;
            fs::rename(&temp_path, &target_path)
                .await
                .with_context(|| {
                    format!(
                        "failed to install downloaded binary '{}': '{}' -> '{}'",
                        binary_name,
                        temp_path.display(),
                        target_path.display()
                    )
                })?;

            Result::<(), anyhow::Error>::Ok(())
        }
        .await;

    if result.is_err() {
        let _ = fs::remove_file(&temp_path).await;
    }

    result?;
    reporter.finish();
    Ok(target_path)
}

fn should_install_binary(paths: &PreviaPaths, binary_name: &str) -> Result<bool> {
    let expected_version = current_release_version();
    let candidates = paths.binary_candidates(binary_name)?;
    Ok(!candidates
        .iter()
        .filter(|path| path.exists())
        .any(|path| binary_matches_version(path, expected_version)))
}

fn binary_matches_version(path: &Path, expected_version: &str) -> bool {
    read_binary_version(path).is_some_and(|version| version == expected_version)
}

fn read_binary_version(path: &Path) -> Option<String> {
    let output = Command::new(path).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    stdout
        .split_whitespace()
        .last()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn binary_download_url(version: &str, asset_name: &str) -> String {
    let base_url = download_base_url();
    if uses_github_release_layout(&base_url) {
        format!("{base_url}/v{version}/{asset_name}")
    } else {
        format!("{base_url}/{version}/files/{asset_name}")
    }
}

fn uses_github_release_layout(base_url: &str) -> bool {
    base_url
        .trim_end_matches('/')
        .ends_with("/releases/download")
}

fn binary_install_path(paths: &PreviaPaths, binary_name: &str) -> PathBuf {
    paths.home.join("bin").join(binary_name)
}

fn temporary_download_path(target_path: &Path) -> PathBuf {
    let pid = std::process::id();
    target_path.with_extension(format!("download-{pid}.tmp"))
}

fn normalized_platform() -> Result<(String, String)> {
    let os = normalized_platform_os();
    let arch = normalized_platform_arch();

    let os_slug = match os.as_str() {
        "linux" => "linux",
        other => {
            bail!(
                "unsupported operating system: {other}. Previa binaries are published for Linux only."
            )
        }
    };
    let arch_slug = match arch.as_str() {
        "x86_64" | "amd64" => "amd64",
        "aarch64" | "arm64" => "arm64",
        other => bail!("unsupported architecture: {other}."),
    };

    Ok((os_slug.to_owned(), arch_slug.to_owned()))
}

fn normalized_platform_os() -> String {
    #[cfg(test)]
    if let Ok(value) = env::var(TEST_DOWNLOAD_OS_ENV) {
        return value;
    }

    env::consts::OS.to_owned()
}

fn normalized_platform_arch() -> String {
    #[cfg(test)]
    if let Ok(value) = env::var(TEST_DOWNLOAD_ARCH_ENV) {
        return value;
    }

    env::consts::ARCH.to_owned()
}

fn asset_filename(binary_name: &str, os_slug: &str, arch_slug: &str) -> Result<String> {
    match binary_name {
        "previa-main" | "previa-runner" => Ok(format!("{binary_name}-{os_slug}-{arch_slug}")),
        other => bail!("unsupported auto-download binary '{other}'"),
    }
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)
        .with_context(|| format!("failed to read '{}'", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)
        .with_context(|| format!("failed to chmod '{}'", path.display()))
}

#[cfg(not(unix))]
fn set_executable(path: &Path) -> Result<()> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("failed to read '{}'", path.display()))?;
    let mut permissions = metadata.permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(path, permissions)
        .with_context(|| format!("failed to update permissions for '{}'", path.display()))
}

trait ProgressReporter {
    fn begin(&mut self, binary_name: &str, version: &str, total_bytes: Option<u64>);
    fn advance(&mut self, bytes: u64);
    fn finish(&mut self);
}

enum DownloadReporter {
    Visible(ProgressBar),
    Hidden,
}

impl DownloadReporter {
    fn for_stderr() -> Self {
        Self::for_terminal(io::stderr().is_terminal())
    }

    fn for_terminal(is_terminal: bool) -> Self {
        if is_terminal {
            let bar = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr_with_hz(10));
            Self::Visible(bar)
        } else {
            Self::Hidden
        }
    }

    #[cfg(test)]
    fn is_visible(&self) -> bool {
        matches!(self, Self::Visible(_))
    }
}

impl ProgressReporter for DownloadReporter {
    fn begin(&mut self, binary_name: &str, version: &str, total_bytes: Option<u64>) {
        let Self::Visible(bar) = self else {
            return;
        };

        let message = format!("Downloading {binary_name} {version}...");
        match total_bytes {
            Some(total) => {
                bar.set_length(total);
                let style = ProgressStyle::with_template(
                    "{spinner:.cyan} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes}",
                )
                .expect("valid progress template")
                .progress_chars("#>-");
                bar.set_style(style);
            }
            None => {
                let style = ProgressStyle::with_template("{spinner:.cyan} {msg} {bytes}")
                    .expect("valid progress template");
                bar.set_style(style);
            }
        }
        bar.set_message(message);
        bar.enable_steady_tick(Duration::from_millis(100));
    }

    fn advance(&mut self, bytes: u64) {
        let Self::Visible(bar) = self else {
            return;
        };
        bar.inc(bytes);
    }

    fn finish(&mut self) {
        if let Self::Visible(bar) = self {
            bar.finish_and_clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::env;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use axum::Router;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::get;
    use tempfile::TempDir;
    use tokio::net::TcpListener;

    use super::{
        DEFAULT_DOWNLOAD_BASE_URL, DOWNLOAD_BASE_URL_ENV, DOWNLOAD_ENV_LOCK, DownloadReporter,
        LEGACY_MANIFEST_URL_ENV, TEST_DOWNLOAD_ARCH_ENV, TEST_DOWNLOAD_OS_ENV, asset_filename,
        binary_download_url, binary_install_path, binary_matches_version, current_release_version,
        download_base_url, download_binary, legacy_manifest_base_url, normalized_platform,
        read_binary_version, should_install_binary, uses_github_release_layout,
    };
    use crate::paths::PreviaPaths;

    #[derive(Default)]
    struct RecordingReporter {
        started: bool,
        finished: bool,
        binary_name: Option<String>,
        version: Option<String>,
        total_bytes: Option<u64>,
        advanced: u64,
    }

    impl super::ProgressReporter for RecordingReporter {
        fn begin(&mut self, binary_name: &str, version: &str, total_bytes: Option<u64>) {
            self.started = true;
            self.binary_name = Some(binary_name.to_owned());
            self.version = Some(version.to_owned());
            self.total_bytes = total_bytes;
        }

        fn advance(&mut self, bytes: u64) {
            self.advanced += bytes;
        }

        fn finish(&mut self) {
            self.finished = true;
        }
    }

    #[derive(Clone)]
    struct TestServerState {
        binaries: BTreeMap<String, Vec<u8>>,
        binary_status: BTreeMap<String, StatusCode>,
        requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    async fn binary_asset(
        State(state): State<TestServerState>,
        axum::extract::Path((version, name)): axum::extract::Path<(String, String)>,
    ) -> (StatusCode, Vec<u8>) {
        state
            .requests
            .lock()
            .expect("requests lock")
            .push(format!("/{version}/files/{name}"));
        if let Some(status) = state.binary_status.get(&name) {
            return (*status, Vec::new());
        }
        match state.binaries.get(&name) {
            Some(bytes) => (StatusCode::OK, bytes.clone()),
            None => (StatusCode::NOT_FOUND, Vec::new()),
        }
    }

    async fn spawn_test_server(
        binaries: BTreeMap<String, Vec<u8>>,
        binary_status: BTreeMap<String, StatusCode>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let state = TestServerState {
            binaries,
            binary_status,
            requests: requests.clone(),
        };
        let app = Router::new()
            .route("/{version}/files/{name}", get(binary_asset))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let address = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve app");
        });
        (format!("http://{address}"), requests)
    }

    fn temp_paths() -> (TempDir, PreviaPaths) {
        let temp = TempDir::new().expect("tempdir");
        let paths = PreviaPaths {
            home: temp.path().to_path_buf(),
            workspace_root: None,
        };
        (temp, paths)
    }

    fn set_linux_amd64_download_platform() {
        unsafe {
            env::set_var(TEST_DOWNLOAD_OS_ENV, "linux");
            env::set_var(TEST_DOWNLOAD_ARCH_ENV, "amd64");
        }
    }

    fn clear_download_platform() {
        unsafe {
            env::remove_var(TEST_DOWNLOAD_OS_ENV);
            env::remove_var(TEST_DOWNLOAD_ARCH_ENV);
        }
    }

    fn write_version_script(path: &Path, binary_name: &str, version: &str) {
        let script = r#"#!/bin/sh
if [ "$1" = "--version" ] || [ "$1" = "-v" ]; then
  printf '%s __VERSION__\n' "__BINARY__"
  exit 0
fi
exit 1
"#
        .replace("__VERSION__", version)
        .replace("__BINARY__", binary_name);
        std::fs::write(path, script).expect("write script");
        let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("chmod");
    }

    #[test]
    fn asset_filename_resolves_expected_binary_names() {
        assert_eq!(
            asset_filename("previa-main", "linux", "amd64").expect("main asset"),
            "previa-main-linux-amd64"
        );
        assert_eq!(
            asset_filename("previa-runner", "linux", "arm64").expect("runner asset"),
            "previa-runner-linux-arm64"
        );
    }

    #[test]
    fn asset_filename_rejects_unsupported_binary_names() {
        let err = asset_filename("previa", "linux", "amd64").expect_err("invalid binary");
        assert!(err.to_string().contains("unsupported auto-download binary"));
    }

    #[test]
    fn platform_normalization_matches_supported_linux_targets() {
        let _guard = DOWNLOAD_ENV_LOCK.lock().expect("download env lock");
        set_linux_amd64_download_platform();
        let (os_slug, arch_slug) = normalized_platform().expect("platform");
        clear_download_platform();

        assert_eq!(os_slug, "linux");
        assert!(matches!(arch_slug.as_str(), "amd64" | "arm64"));
    }

    #[test]
    fn download_reporter_respects_terminal_visibility() {
        assert!(DownloadReporter::for_terminal(true).is_visible());
        assert!(!DownloadReporter::for_terminal(false).is_visible());
    }

    #[test]
    fn reads_binary_version_from_version_output() {
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("previa-main");
        write_version_script(&path, "previa-main", "1.2.3");

        assert_eq!(read_binary_version(&path).as_deref(), Some("1.2.3"));
        assert!(binary_matches_version(&path, "1.2.3"));
        assert!(!binary_matches_version(&path, "9.9.9"));
    }

    #[tokio::test]
    async fn downloads_missing_binary_for_current_cli_version() {
        let (temp, paths) = temp_paths();
        let binary_name = "previa-main";
        let asset_name = "previa-main-linux-amd64";
        let payload = b"#!/bin/sh\necho downloaded\n".to_vec();
        let (base_url, requests) = spawn_test_server(
            BTreeMap::from([(asset_name.to_owned(), payload.clone())]),
            BTreeMap::new(),
        )
        .await;

        let _guard = DOWNLOAD_ENV_LOCK.lock().expect("download env lock");
        unsafe {
            env::set_var(DOWNLOAD_BASE_URL_ENV, &base_url);
        }
        set_linux_amd64_download_platform();

        let mut reporter = RecordingReporter::default();
        let client = super::build_download_client().expect("client");

        let installed = download_binary(
            &client,
            &paths,
            binary_name,
            current_release_version(),
            &mut reporter,
        )
        .await
        .expect("downloaded binary");

        unsafe {
            env::remove_var(DOWNLOAD_BASE_URL_ENV);
        }
        clear_download_platform();

        assert_eq!(installed, binary_install_path(&paths, binary_name));
        assert_eq!(std::fs::read(&installed).expect("binary bytes"), payload);
        assert!(reporter.started);
        assert!(reporter.finished);
        assert_eq!(reporter.binary_name.as_deref(), Some(binary_name));
        assert_eq!(reporter.version.as_deref(), Some(current_release_version()));
        assert_eq!(reporter.total_bytes, Some(26));
        assert_eq!(reporter.advanced, 26);
        assert_eq!(
            requests.lock().expect("requests lock").as_slice(),
            &[format!("/{}/files/{asset_name}", current_release_version())]
        );
        drop(temp);
    }

    #[test]
    fn install_is_skipped_when_existing_workspace_binary_matches_cli_version() {
        let (_temp, paths) = temp_paths();
        let workspace = TempDir::new().expect("workspace");
        let debug_dir = workspace.path().join("target/debug");
        std::fs::create_dir_all(&debug_dir).expect("debug dir");
        let workspace_binary = debug_dir.join("previa-main");
        write_version_script(&workspace_binary, "previa-main", current_release_version());

        let paths = PreviaPaths {
            home: paths.home.clone(),
            workspace_root: Some(workspace.path().to_path_buf()),
        };

        assert_eq!(
            read_binary_version(&workspace_binary).as_deref(),
            Some(current_release_version()),
            "workspace binary should report the current CLI version"
        );
        let candidates = paths.binary_candidates("previa-main").expect("candidates");
        let candidate_versions = candidates
            .iter()
            .map(|candidate| (candidate.clone(), read_binary_version(candidate)))
            .collect::<Vec<_>>();
        assert!(
            !should_install_binary(&paths, "previa-main").expect("should install"),
            "candidate versions: {candidate_versions:?}"
        );
    }

    #[test]
    fn install_replaces_existing_home_binary_when_version_differs() {
        let (temp, paths) = temp_paths();
        let install_path = binary_install_path(&paths, "previa-main");
        std::fs::create_dir_all(install_path.parent().expect("bin dir")).expect("bin dir");
        write_version_script(&install_path, "previa-main", "0.0.7");

        let paths = PreviaPaths {
            home: paths.home.clone(),
            workspace_root: Some(temp.path().join("isolated-workspace")),
        };

        assert!(should_install_binary(&paths, "previa-main").expect("should install"));
    }

    #[test]
    fn install_is_skipped_when_workspace_binary_matches_even_if_home_binary_is_stale() {
        let (_temp, paths) = temp_paths();
        let install_path = binary_install_path(&paths, "previa-main");
        std::fs::create_dir_all(install_path.parent().expect("bin dir")).expect("bin dir");
        write_version_script(&install_path, "previa-main", "0.0.7");

        let workspace = TempDir::new().expect("workspace");
        let debug_dir = workspace.path().join("target/debug");
        std::fs::create_dir_all(&debug_dir).expect("debug dir");
        write_version_script(
            &debug_dir.join("previa-main"),
            "previa-main",
            current_release_version(),
        );

        let paths = PreviaPaths {
            home: paths.home.clone(),
            workspace_root: Some(workspace.path().to_path_buf()),
        };

        assert!(!should_install_binary(&paths, "previa-main").expect("should install"));
    }

    #[tokio::test]
    async fn download_fails_when_binary_download_fails() {
        let (_temp, paths) = temp_paths();
        let asset_name = "previa-main-linux-amd64";
        let (base_url, _) = spawn_test_server(
            BTreeMap::new(),
            BTreeMap::from([(asset_name.to_owned(), StatusCode::INTERNAL_SERVER_ERROR)]),
        )
        .await;

        let _guard = DOWNLOAD_ENV_LOCK.lock().expect("download env lock");
        unsafe {
            env::set_var(DOWNLOAD_BASE_URL_ENV, &base_url);
        }
        set_linux_amd64_download_platform();

        let err = download_binary(
            &super::build_download_client().expect("client"),
            &paths,
            "previa-main",
            current_release_version(),
            &mut RecordingReporter::default(),
        )
        .await
        .expect_err("download failure");

        unsafe {
            env::remove_var(DOWNLOAD_BASE_URL_ENV);
        }
        clear_download_platform();

        assert!(
            err.to_string()
                .contains("failed to download binary 'previa-main'")
        );
    }

    #[test]
    fn download_base_url_uses_environment_override() {
        let _guard = DOWNLOAD_ENV_LOCK.lock().expect("download env lock");
        unsafe {
            env::set_var(DOWNLOAD_BASE_URL_ENV, "http://downloads.test");
        }
        assert_eq!(download_base_url(), "http://downloads.test");
        unsafe {
            env::remove_var(DOWNLOAD_BASE_URL_ENV);
        }
    }

    #[test]
    fn download_base_url_supports_legacy_manifest_override() {
        let _guard = DOWNLOAD_ENV_LOCK.lock().expect("download env lock");
        unsafe {
            env::set_var(LEGACY_MANIFEST_URL_ENV, "http://downloads.test/latest.json");
        }
        assert_eq!(download_base_url(), "http://downloads.test");
        unsafe {
            env::remove_var(LEGACY_MANIFEST_URL_ENV);
        }
    }

    #[test]
    fn download_base_url_supports_raw_github_manifest_override() {
        assert_eq!(
            legacy_manifest_base_url(
                "https://raw.githubusercontent.com/runvibe/previa/main/release-metadata.json"
            ),
            Some("https://github.com/runvibe/previa/releases/download".to_owned())
        );
    }

    #[test]
    fn github_release_layout_is_detected() {
        assert!(uses_github_release_layout(
            "https://github.com/runvibe/previa/releases/download"
        ));
        assert!(!uses_github_release_layout("http://downloads.test"));
    }

    #[test]
    fn versioned_url_uses_exact_cli_version() {
        let _guard = DOWNLOAD_ENV_LOCK.lock().expect("download env lock");
        unsafe {
            env::remove_var(DOWNLOAD_BASE_URL_ENV);
            env::remove_var(LEGACY_MANIFEST_URL_ENV);
        }
        assert_eq!(
            binary_download_url(current_release_version(), "previa-main-linux-amd64"),
            format!(
                "{}/v{}/previa-main-linux-amd64",
                DEFAULT_DOWNLOAD_BASE_URL,
                current_release_version()
            )
        );
    }
}
