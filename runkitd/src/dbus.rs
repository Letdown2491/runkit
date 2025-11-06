use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use zbus::MessageHeader;
use zbus::blocking::{Connection, ConnectionBuilder};
use zbus::fdo;
use zbus_polkit::policykit1::{AuthorityProxyBlocking, CheckAuthorizationFlags, Subject};

use crate::{ActionKind, CommandOutcome, HelperContext, HelperError, HelperResponse};

const BUS_NAME: &str = "tech.geektoshi.Runkit1";
const OBJECT_PATH: &str = "/tech/geektoshi/Runkit1";
const POLKIT_ACTION_REQUIRE_PASSWORD: &str = "tech.geektoshi.Runkit.require_password";
const POLKIT_ACTION_ALLOW_CACHE: &str = "tech.geektoshi.Runkit.cached";

pub fn run_dbus_service() -> Result<(), Box<dyn std::error::Error>> {
    let service = RunkitService {
        context: HelperContext::default(),
    };

    let _connection = ConnectionBuilder::system()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, service)?
        .build()?;

    // Keep the process alive while zbus' internal executor services requests.
    loop {
        thread::park_timeout(Duration::from_secs(60));
    }

    #[allow(unreachable_code)]
    Ok(())
}

struct RunkitService {
    context: HelperContext,
}

#[zbus::dbus_interface(name = "tech.geektoshi.Runkit1.Controller")]
impl RunkitService {
    fn perform_action(
        &self,
        #[zbus(header)] header: MessageHeader<'_>,
        action: &str,
        service: &str,
        allow_cached_authorization: bool,
    ) -> fdo::Result<String> {
        let Some(kind) = ActionKind::parse(action) else {
            return serialize_response(Err(HelperError::Other(format!(
                "Unsupported action '{action}'"
            ))));
        };

        let action_id = if allow_cached_authorization {
            POLKIT_ACTION_ALLOW_CACHE
        } else {
            POLKIT_ACTION_REQUIRE_PASSWORD
        };

        let mut details = HashMap::new();
        details.insert("service", service);
        details.insert("operation", kind.as_str());

        if let Err(message) = authorize(&header, action_id, details) {
            return serialize_response(Err(HelperError::Other(message)));
        }

        serialize_response(self.context.perform_action(kind, service))
    }

    fn list_services(&self) -> fdo::Result<String> {
        serialize_response(self.context.list())
    }

    fn fetch_logs(&self, service: &str, lines: u32) -> fdo::Result<String> {
        serialize_response(self.context.logs(service, lines as usize))
    }

    fn fetch_description(&self, service: &str) -> fdo::Result<String> {
        serialize_response(self.context.describe(service))
    }
}

fn authorize(
    header: &MessageHeader<'_>,
    action_id: &str,
    details: HashMap<&str, &str>,
) -> Result<(), String> {
    let connection =
        Connection::system().map_err(|err| format!("polkit connection error: {err}"))?;
    let proxy = AuthorityProxyBlocking::new(&connection)
        .map_err(|err| format!("polkit proxy error: {err}"))?;
    let subject = Subject::new_for_message_header(header)
        .map_err(|err| format!("polkit subject error: {err}"))?;
    let flags = CheckAuthorizationFlags::AllowUserInteraction.into();
    let result = proxy
        .check_authorization(&subject, action_id, &details, flags, "")
        .map_err(|err| format!("polkit check failed: {err}"))?;

    if result.is_authorized {
        Ok(())
    } else if result.is_challenge {
        Err("Authentication was dismissed or failed".to_string())
    } else {
        Err("Authorization denied".to_string())
    }
}

fn serialize_response(result: Result<CommandOutcome, HelperError>) -> fdo::Result<String> {
    let response = match result {
        Ok(outcome) => HelperResponse::ok_with(outcome),
        Err(err) => HelperResponse::error(err.to_string()),
    };
    serde_json::to_string(&response).map_err(|err| fdo::Error::Failed(err.to_string()))
}
