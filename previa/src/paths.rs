use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub struct PreviaPaths {
    pub home: PathBuf,
    pub workspace_root: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct StackPaths {
    pub name: String,
    pub config_dir: PathBuf,
    pub main_env: PathBuf,
    pub runner_env: PathBuf,
    pub main_data_dir: PathBuf,
    pub runner_logs_dir: PathBuf,
    pub main_log: PathBuf,
    pub run_dir: PathBuf,
    pub runtime_file: PathBuf,
    pub lock_file: PathBuf,
    pub compose_file: PathBuf,
}

impl PreviaPaths {
    pub fn discover(home_override: Option<&Path>) -> Result<Self> {
        let home = match home_override {
            Some(path) => absolutize(path.to_path_buf())?,
            None => match env::var("PREVIA_HOME") {
                Ok(value) => absolutize(PathBuf::from(value))?,
                Err(_) => {
                    let user_home =
                        env::var("HOME").context("HOME is not set and PREVIA_HOME is unset")?;
                    absolutize(PathBuf::from(user_home).join(".previa"))?
                }
            },
        };

        Ok(Self {
            home,
            workspace_root: discover_workspace_root()?,
        })
    }

    pub fn main_binary(&self) -> Result<PathBuf> {
        resolve_binary(&self.home, self.workspace_root.as_deref(), "previa-main")
    }

    pub fn runner_binary(&self) -> Result<PathBuf> {
        resolve_binary(&self.home, self.workspace_root.as_deref(), "previa-runner")
    }

    pub(crate) fn binary_candidates(&self, binary_name: &str) -> Result<Vec<PathBuf>> {
        binary_candidates(&self.home, self.workspace_root.as_deref(), binary_name)
    }

    pub fn stack(&self, name: &str) -> StackPaths {
        let root = self.home.join("stacks").join(name);
        let config_dir = root.join("config");
        let main_data_dir = root.join("data").join("main");
        let logs_dir = root.join("logs");
        let runner_logs_dir = logs_dir.join("runners");
        let run_dir = root.join("run");
        StackPaths {
            name: name.to_owned(),
            main_env: config_dir.join("main.env"),
            runner_env: config_dir.join("runner.env"),
            main_log: logs_dir.join("main.log"),
            runtime_file: run_dir.join("state.json"),
            lock_file: run_dir.join("lock"),
            compose_file: run_dir.join("docker-compose.generated.yaml"),
            config_dir,
            main_data_dir,
            runner_logs_dir,
            run_dir,
        }
    }

    pub fn stack_roots(&self) -> Result<Vec<StackPaths>> {
        let stacks_dir = self.home.join("stacks");
        if !stacks_dir.exists() {
            return Ok(Vec::new());
        }
        let mut stacks = Vec::new();
        for entry in std::fs::read_dir(&stacks_dir).with_context(|| {
            format!("failed to read stacks directory '{}'", stacks_dir.display())
        })? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                stacks.push(self.stack(&name));
            }
        }
        Ok(stacks)
    }
}

impl StackPaths {
    pub fn runner_log(&self, port: u16) -> PathBuf {
        self.runner_logs_dir.join(format!("{port}.log"))
    }

    pub fn ensure_parent_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.config_dir)
            .with_context(|| format!("failed to create '{}'", self.config_dir.display()))?;
        std::fs::create_dir_all(&self.main_data_dir)
            .with_context(|| format!("failed to create '{}'", self.main_data_dir.display()))?;
        std::fs::create_dir_all(&self.runner_logs_dir)
            .with_context(|| format!("failed to create '{}'", self.runner_logs_dir.display()))?;
        std::fs::create_dir_all(&self.run_dir)
            .with_context(|| format!("failed to create '{}'", self.run_dir.display()))?;
        Ok(())
    }
}

fn absolutize(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path);
    }
    Ok(env::current_dir()
        .context("failed to read current directory")?
        .join(path))
}

fn resolve_binary(
    home: &Path,
    workspace_root: Option<&Path>,
    binary_name: &str,
) -> Result<PathBuf> {
    let candidates = binary_candidates(home, workspace_root, binary_name)?;
    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }

    let searched = candidates
        .iter()
        .map(|candidate| format!("'{}'", candidate.display()))
        .collect::<Vec<_>>()
        .join(", ");
    bail!("missing binary '{}'; searched {}", binary_name, searched);
}

fn binary_candidates(
    home: &Path,
    workspace_root: Option<&Path>,
    binary_name: &str,
) -> Result<Vec<PathBuf>> {
    let mut candidates = Vec::new();

    let discovered_workspace_root = match workspace_root {
        Some(path) => Some(path.to_path_buf()),
        None => discover_workspace_root()?,
    };

    if let Some(workspace_root) = discovered_workspace_root {
        candidates.push(workspace_root.join("target/debug").join(binary_name));
        candidates.push(workspace_root.join("target/release").join(binary_name));
    }

    candidates.push(home.join("bin").join(binary_name));

    candidates.dedup();
    Ok(candidates)
}

fn discover_workspace_root() -> Result<Option<PathBuf>> {
    let current_dir = env::current_dir().context("failed to read current directory")?;
    Ok(find_workspace_root(&current_dir))
}

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let manifest = dir.join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        let contents = std::fs::read_to_string(&manifest).ok()?;
        if contents.contains("[workspace]") {
            return Some(dir.to_path_buf());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{binary_candidates, find_workspace_root};
    use std::path::{Path, PathBuf};

    #[test]
    fn finds_workspace_root_from_nested_directory() {
        let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = find_workspace_root(&crate_dir);
        assert_eq!(root, crate_dir.parent().map(Path::to_path_buf));
    }

    #[test]
    fn binary_candidates_prioritize_workspace_targets_before_previa_home() {
        let home = PathBuf::from("/tmp/previa-home");
        let workspace_root = PathBuf::from("/tmp/workspace");
        let candidates =
            binary_candidates(&home, Some(&workspace_root), "previa-main").expect("candidates");
        assert_eq!(
            candidates[0],
            PathBuf::from("/tmp/workspace/target/debug/previa-main")
        );
        if candidates.len() >= 3 {
            assert_eq!(
                candidates[1],
                PathBuf::from("/tmp/workspace/target/release/previa-main")
            );
            assert_eq!(
                candidates[2],
                PathBuf::from("/tmp/previa-home/bin/previa-main")
            );
        }
    }
}
