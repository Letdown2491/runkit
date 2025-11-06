use runkit_core::{DesiredState, ServiceInfo, ServiceRuntimeState};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::Type;

const BUS_NAME: &str = "tech.geektoshi.Runkit1";
const OBJECT_PATH: &str = "/tech/geektoshi/Runkit1";
const INTERFACE: &str = "tech.geektoshi.Runkit1.Controller";

#[derive(Clone)]
pub struct ActionDispatcher {
    connection: Connection,
}

impl Default for ActionDispatcher {
    fn default() -> Self {
        let connection =
            Connection::system().expect("Failed to connect to the system bus for runkitd");
        ActionDispatcher { connection }
    }
}

impl ActionDispatcher {
    fn proxy(&self) -> Result<Proxy<'_>, String> {
        Proxy::new(&self.connection, BUS_NAME, OBJECT_PATH, INTERFACE)
            .map_err(|err| format!("Failed to connect to runkitd: {err}"))
    }

    fn call_helper<T>(&self, method: &str, body: &T) -> Result<DaemonProcessResponse, String>
    where
        T: serde::ser::Serialize + Type,
    {
        let proxy = self.proxy()?;
        let reply: String = proxy
            .call(method, body)
            .map_err(|err| format!("runkitd call {method} failed: {err}"))?;
        serde_json::from_str(&reply)
            .map_err(|err| format!("Failed to decode runkitd response for {method}: {err}"))
    }

    pub fn run(
        &self,
        action: &str,
        service: &str,
        allow_cached_authorization: bool,
    ) -> Result<String, String> {
        let response = self.call_helper(
            "PerformAction",
            &(action, service, allow_cached_authorization),
        )?;
        match response.status.as_str() {
            "ok" => Ok(response
                .message
                .unwrap_or_else(|| format!("{action} command completed for {service}"))),
            _ => Err(response
                .message
                .unwrap_or_else(|| format!("runkitd reported failure for {service}"))),
        }
    }

    pub fn fetch_services(&self) -> Result<Vec<ServiceInfo>, String> {
        let response = self.call_helper::<()>("ListServices", &())?;
        if response.status.as_str() != "ok" {
            return Err(response
                .message
                .unwrap_or_else(|| "runkitd failed to enumerate services".to_string()));
        }

        let data = response
            .data
            .ok_or_else(|| "runkitd returned no service data".to_string())?;

        let snapshots: Vec<ServiceSnapshot> = serde_json::from_value(data)
            .map_err(|err| format!("Failed to decode runkitd response: {err}"))?;

        Ok(snapshots.into_iter().map(ServiceInfo::from).collect())
    }

    pub fn fetch_logs(&self, service: &str, lines: usize) -> Result<Vec<LogEntry>, String> {
        let line_cap = lines.max(1).min(u32::MAX as usize) as u32;
        let response = self.call_helper("FetchLogs", &(service, line_cap))?;

        if response.status.as_str() != "ok" {
            return Err(response
                .message
                .unwrap_or_else(|| format!("runkitd failed to stream logs for {service}")));
        }

        let data = response
            .data
            .ok_or_else(|| "runkitd returned no log data".to_string())?;

        let entries: Vec<LogEntrySnapshot> = serde_json::from_value(data)
            .map_err(|err| format!("Failed to decode runkitd logs response: {err}"))?;

        Ok(entries.into_iter().map(LogEntry::from).collect())
    }

    pub fn fetch_description(&self, service: &str) -> Result<Option<String>, String> {
        let response = self.call_helper("FetchDescription", &(service,))?;

        if response.status.as_str() != "ok" {
            return Err(response
                .message
                .unwrap_or_else(|| format!("runkitd failed to describe {service}")));
        }

        let data = response
            .data
            .ok_or_else(|| "runkitd returned no description data".to_string())?;

        let snapshot: DescriptionSnapshot = serde_json::from_value(data)
            .map_err(|err| format!("Failed to decode runkitd description response: {err}"))?;

        Ok(snapshot.description)
    }
}

#[derive(Debug, Deserialize)]
struct DaemonProcessResponse {
    status: String,
    message: Option<String>,
    data: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ServiceSnapshot {
    name: String,
    definition_path: String,
    enabled: bool,
    desired_state: SnapshotDesiredState,
    runtime_state: SnapshotRuntimeState,
    description: Option<String>,
}

impl From<ServiceSnapshot> for ServiceInfo {
    fn from(snapshot: ServiceSnapshot) -> Self {
        ServiceInfo {
            name: snapshot.name,
            definition_path: snapshot.definition_path.into(),
            enabled: snapshot.enabled,
            desired_state: DesiredState::from(snapshot.desired_state),
            runtime_state: ServiceRuntimeState::from(snapshot.runtime_state),
            description: snapshot.description,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SnapshotDesiredState {
    AutoStart,
    Manual,
}

impl From<SnapshotDesiredState> for DesiredState {
    fn from(value: SnapshotDesiredState) -> Self {
        match value {
            SnapshotDesiredState::AutoStart => DesiredState::AutoStart,
            SnapshotDesiredState::Manual => DesiredState::Manual,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum SnapshotRuntimeState {
    Running {
        pid: u32,
        uptime_seconds: u64,
    },
    Down {
        since_seconds: u64,
        normally_up: bool,
    },
    Failed {
        pid: u32,
        uptime_seconds: u64,
        exit_code: i32,
    },
    Unknown {
        raw: String,
    },
}

impl From<SnapshotRuntimeState> for ServiceRuntimeState {
    fn from(value: SnapshotRuntimeState) -> Self {
        match value {
            SnapshotRuntimeState::Running {
                pid,
                uptime_seconds,
            } => ServiceRuntimeState::Running {
                pid,
                uptime: Duration::from_secs(uptime_seconds),
            },
            SnapshotRuntimeState::Down {
                since_seconds,
                normally_up,
            } => ServiceRuntimeState::Down {
                since: Duration::from_secs(since_seconds),
                normally_up,
            },
            SnapshotRuntimeState::Failed {
                pid,
                uptime_seconds,
                exit_code,
            } => ServiceRuntimeState::Failed {
                pid,
                uptime: Duration::from_secs(uptime_seconds),
                exit_code,
            },
            SnapshotRuntimeState::Unknown { raw } => ServiceRuntimeState::Unknown { raw },
        }
    }
}

#[derive(Debug, Deserialize)]
struct LogEntrySnapshot {
    unix_seconds: Option<i64>,
    nanos: Option<u32>,
    raw: Option<String>,
    message: String,
}

impl From<LogEntrySnapshot> for LogEntry {
    fn from(snapshot: LogEntrySnapshot) -> Self {
        LogEntry {
            unix_seconds: snapshot.unix_seconds,
            nanos: snapshot.nanos,
            raw: snapshot.raw,
            message: snapshot.message,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub unix_seconds: Option<i64>,
    pub nanos: Option<u32>,
    pub raw: Option<String>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct DescriptionSnapshot {
    #[allow(dead_code)]
    service: String,
    description: Option<String>,
}
