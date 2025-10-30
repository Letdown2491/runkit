//! Core domain layer for discovering and describing Void Linux runit services.
use once_cell::sync::Lazy;
use regex::Regex;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use thiserror::Error;

pub const DEFAULT_SERVICE_DIR: &str = "/etc/sv";
pub const DEFAULT_ENABLED_DIR: &str = "/var/service";

static RUNNING_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^run:\s+(?P<name>[^:]+):\s+\(pid\s+(?P<pid>\d+)\)\s+(?P<uptime>\d+)s").unwrap()
});
static DOWN_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^down:\s+(?P<name>[^:]+):\s+(?P<since>\d+)s(,)?").unwrap());
static FAIL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^fail:\s+(?P<name>[^:]+):\s+\(pid\s+(?P<pid>\d+)\)\s+(?P<uptime>\d+)s,\s+exit\s+(?P<code>[-]?\d+)",
    )
    .unwrap()
});

/// High-level state of a runit service instance.
#[derive(Debug, Clone)]
pub enum ServiceRuntimeState {
    Running {
        pid: u32,
        uptime: Duration,
    },
    Down {
        since: Duration,
        normally_up: bool,
    },
    Failed {
        pid: u32,
        uptime: Duration,
        exit_code: i32,
    },
    Unknown {
        raw: String,
    },
}

impl ServiceRuntimeState {
    pub fn from_sv_status(status_output: &str) -> Self {
        let line = status_output.lines().next().unwrap_or("").trim();

        if let Some(caps) = RUNNING_REGEX.captures(line) {
            let pid = caps
                .name("pid")
                .and_then(|m| m.as_str().parse::<u32>().ok());
            let uptime = caps
                .name("uptime")
                .and_then(|m| m.as_str().parse::<u64>().ok())
                .map(Duration::from_secs);
            if let (Some(pid), Some(uptime)) = (pid, uptime) {
                return ServiceRuntimeState::Running { pid, uptime };
            }
        }

        if let Some(caps) = DOWN_REGEX.captures(line) {
            let since = caps
                .name("since")
                .and_then(|m| m.as_str().parse::<u64>().ok())
                .map(Duration::from_secs)
                .unwrap_or_default();
            let normally_up = line.contains("normally up");
            return ServiceRuntimeState::Down { since, normally_up };
        }

        if let Some(caps) = FAIL_REGEX.captures(line) {
            let pid = caps
                .name("pid")
                .and_then(|m| m.as_str().parse::<u32>().ok());
            let uptime = caps
                .name("uptime")
                .and_then(|m| m.as_str().parse::<u64>().ok())
                .map(Duration::from_secs);
            let exit_code = caps
                .name("code")
                .and_then(|m| m.as_str().parse::<i32>().ok())
                .unwrap_or_default();
            if let (Some(pid), Some(uptime)) = (pid, uptime) {
                return ServiceRuntimeState::Failed {
                    pid,
                    uptime,
                    exit_code,
                };
            }
        }

        ServiceRuntimeState::Unknown {
            raw: line.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ServiceManager, ServiceRuntimeState};
    use std::time::Duration;

    #[test]
    fn parses_running_status() {
        let state = ServiceRuntimeState::from_sv_status("run: sshd: (pid 1234) 42s\n");
        match state {
            ServiceRuntimeState::Running { pid, uptime } => {
                assert_eq!(pid, 1234);
                assert_eq!(uptime, Duration::from_secs(42));
            }
            other => panic!("unexpected state: {:?}", other),
        }
    }

    #[test]
    fn parses_down_status() {
        let state = ServiceRuntimeState::from_sv_status("down: cron: 5s, normally up\n");
        match state {
            ServiceRuntimeState::Down { since, normally_up } => {
                assert_eq!(since, Duration::from_secs(5));
                assert!(normally_up);
            }
            other => panic!("unexpected state: {:?}", other),
        }
    }

    #[test]
    fn validates_service_name() {
        let manager = ServiceManager::default();
        assert!(manager.validate_service_name("valid_name-01").is_ok());
        assert!(manager.validate_service_name("../bad").is_err());
        assert!(manager.validate_service_name("").is_err());
    }
}

/// Desired state of a service as configured by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesiredState {
    AutoStart,
    Manual,
}

/// Immutable snapshot of a runit service.
#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub name: String,
    pub definition_path: PathBuf,
    pub enabled: bool,
    pub desired_state: DesiredState,
    pub runtime_state: ServiceRuntimeState,
    pub description: Option<String>,
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("I/O error while accessing {path:?}: {source}")]
    Io {
        #[source]
        source: std::io::Error,
        path: PathBuf,
    },

    #[error("sv command failed for service {service}: {message}")]
    SvCommand { service: String, message: String },

    #[error("invalid service name: {0}")]
    InvalidServiceName(String),

    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl ServiceError {
    pub fn from_io(path: impl Into<PathBuf>, err: std::io::Error) -> Self {
        ServiceError::Io {
            source: err,
            path: path.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, ServiceError>;

/// Discover and interrogate runit services.
#[derive(Debug, Clone)]
pub struct ServiceManager {
    definitions_dir: PathBuf,
    enabled_dir: PathBuf,
    sv_command: PathBuf,
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new(DEFAULT_SERVICE_DIR, DEFAULT_ENABLED_DIR)
    }
}

impl ServiceManager {
    pub fn new(definitions_dir: impl Into<PathBuf>, enabled_dir: impl Into<PathBuf>) -> Self {
        ServiceManager {
            definitions_dir: definitions_dir.into(),
            enabled_dir: enabled_dir.into(),
            sv_command: PathBuf::from("sv"),
        }
    }

    pub fn with_sv_command(mut self, cmd: impl Into<PathBuf>) -> Self {
        self.sv_command = cmd.into();
        self
    }

    pub fn definitions_dir(&self) -> &Path {
        &self.definitions_dir
    }

    pub fn enabled_dir(&self) -> &Path {
        &self.enabled_dir
    }

    pub fn sv_command_path(&self) -> &Path {
        &self.sv_command
    }

    /// Enumerate all services available on the system.
    pub fn list_services(&self) -> Result<Vec<ServiceInfo>> {
        let mut services = Vec::new();

        let read_dir = std::fs::read_dir(&self.definitions_dir)
            .map_err(|e| ServiceError::from_io(&self.definitions_dir, e))?;

        for entry in read_dir {
            let entry = entry.map_err(|e| ServiceError::from_io(&self.definitions_dir, e))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(OsStr::to_str) {
                if let Some(info) = self.build_service_info(name, &path)? {
                    services.push(info);
                }
            }
        }

        services.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(services)
    }

    fn build_service_info(
        &self,
        name: &str,
        definition_path: &Path,
    ) -> Result<Option<ServiceInfo>> {
        // Skip hidden directories or invalid names.
        if name.starts_with('.') {
            return Ok(None);
        }

        let enabled_path = self.enabled_dir.join(name);
        let enabled = enabled_path.exists();
        let desired_state = if enabled {
            DesiredState::AutoStart
        } else {
            DesiredState::Manual
        };

        let runtime_state = self.status(name)?;
        let description = self.read_description(definition_path);

        Ok(Some(ServiceInfo {
            name: name.to_string(),
            definition_path: definition_path.to_path_buf(),
            enabled,
            desired_state,
            runtime_state,
            description,
        }))
    }

    /// Fetch the runtime status for a single service via `sv status`.
    pub fn status(&self, service: &str) -> Result<ServiceRuntimeState> {
        self.validate_service_name(service)?;

        let output = Command::new(&self.sv_command)
            .arg("status")
            .arg(service)
            .output()
            .map_err(|err| ServiceError::from_io(&self.sv_command, err))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if !stderr.is_empty() {
            return Err(ServiceError::SvCommand {
                service: service.to_string(),
                message: stderr,
            });
        }

        if stdout.trim().is_empty() {
            let status_desc = output
                .status
                .code()
                .map(|code| format!("exit status {code}"))
                .unwrap_or_else(|| output.status.to_string());
            return Err(ServiceError::SvCommand {
                service: service.to_string(),
                message: format!("sv status returned no output ({status_desc})"),
            });
        }

        Ok(ServiceRuntimeState::from_sv_status(&stdout))
    }

    fn read_description(&self, definition_path: &Path) -> Option<String> {
        let candidates = ["description", "README", "README.md"];
        for candidate in candidates {
            let file = definition_path.join(candidate);
            if let Ok(contents) = std::fs::read_to_string(&file) {
                let trimmed = contents.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.lines().next().unwrap_or(trimmed).to_string());
                }
            }
        }
        None
    }

    pub fn validate_service_name(&self, service: &str) -> Result<()> {
        let valid = !service.is_empty()
            && service
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
        if valid {
            Ok(())
        } else {
            Err(ServiceError::InvalidServiceName(service.to_string()))
        }
    }
}
