mod actions;
mod formatting;
mod ui;

use actions::{ActionDispatcher, LogEntry};
use gtk::glib::ControlFlow;
use gtk::glib::{self, source::SourceId};
use gtk4::{self as gtk, pango};
use libadwaita::{self as adw, Application, prelude::*};
use runkit_core::{ActivityEvent, ActivityEventType, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

fn main() -> glib::ExitCode {
    adw::init().expect("Failed to initialize libadwaita");

    let app = Application::builder()
        .application_id("tech.geektoshi.Runkit")
        .build();

    app.connect_activate(|app| {
        let controller = AppController::new(app, ActionDispatcher::default());
        controller.request_initial_load();
    });

    app.run()
}

struct AppController {
    dispatcher: ActionDispatcher,
    model: Rc<RefCell<AppModel>>,
    widgets: ui::AppWidgets,
    description_store: RefCell<DescriptionStore>,
    activity_store: RefCell<ActivityStore>,
    preferences_window: RefCell<Option<adw::PreferencesWindow>>,
    about_dialog: RefCell<Option<adw::MessageDialog>>,
    preferences: RefCell<UserPreferences>,
    refresh_source: RefCell<Option<SourceId>>,
}

#[derive(Default)]
struct AppModel {
    services: Vec<ServiceInfo>,
    filter_text: String,
    log_entries: Vec<LogEntry>,
    log_service: Option<String>,
    log_error: Option<String>,
    current_description: Option<String>,
    description_error: Option<String>,
    list_refreshing: bool,
    activity_notes: Vec<String>,
    pending_selection: Option<String>,
    previous_states: HashMap<String, (String, Option<u32>)>, // service -> (state_name, pid)
    initial_load_completed: bool,
}

struct DescriptionStore {
    path: Option<PathBuf>,
    entries: HashMap<String, Option<String>>,
}

impl DescriptionStore {
    fn load() -> Self {
        let path = description_store_path();
        let entries = path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default();
        DescriptionStore { path, entries }
    }

    fn lookup(&self, service: &str) -> Option<Option<String>> {
        self.entries.get(service).cloned()
    }

    fn ensure_present(&mut self, service: &str, description: &str) {
        let trimmed = description.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.entries.contains_key(service) {
            return;
        }
        if let Err(err) = self.store(service, Some(trimmed.to_string())) {
            eprintln!("Failed to persist description for {service}: {err}");
        }
    }

    fn store(&mut self, service: &str, description: Option<String>) -> io::Result<()> {
        let needs_write = match self.entries.get(service) {
            Some(existing) if existing == &description => false,
            _ => true,
        };
        if !needs_write {
            return Ok(());
        }

        self.entries
            .insert(service.to_string(), description.clone());
        self.save()
    }

    fn save(&self) -> io::Result<()> {
        let path = match &self.path {
            Some(path) => path,
            None => return Ok(()),
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(&self.entries)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        fs::write(path, data)
    }
}

fn description_store_path() -> Option<PathBuf> {
    let mut base = config_root()?;
    base.push("runkit");
    base.push("services.json");
    Some(base)
}

const MAX_ACTIVITY_PER_SERVICE: usize = 10;

#[derive(Serialize, Deserialize)]
struct ActivityStoreData {
    entries: HashMap<String, VecDeque<ActivityEvent>>,
    #[serde(default)]
    previous_states: HashMap<String, (String, Option<u32>)>, // service -> (state_name, pid)
}

struct ActivityStore {
    path: Option<PathBuf>,
    data: ActivityStoreData,
}

impl ActivityStore {
    fn load() -> Self {
        let path = activity_store_path();
        let data: ActivityStoreData = path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_else(|| ActivityStoreData {
                entries: HashMap::new(),
                previous_states: HashMap::new(),
            });
        ActivityStore { path, data }
    }

    fn get_activities(&self, service: &str) -> Vec<ActivityEvent> {
        self.data
            .entries
            .get(service)
            .map(|deque| deque.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn get_previous_states(&self) -> HashMap<String, (String, Option<u32>)> {
        self.data.previous_states.clone()
    }

    fn add_event(&mut self, service: &str, event: ActivityEvent) {
        let deque = self.data.entries.entry(service.to_string()).or_default();
        deque.push_back(event);

        // Keep only the last MAX_ACTIVITY_PER_SERVICE entries
        while deque.len() > MAX_ACTIVITY_PER_SERVICE {
            deque.pop_front();
        }

        if let Err(err) = self.save() {
            eprintln!("Failed to persist activity for {service}: {err}");
        }
    }

    fn update_previous_states(&mut self, previous_states: HashMap<String, (String, Option<u32>)>) {
        self.data.previous_states = previous_states;
        if let Err(err) = self.save() {
            eprintln!("Failed to persist previous states: {err}");
        }
    }

    fn save(&self) -> io::Result<()> {
        let path = match &self.path {
            Some(path) => path,
            None => return Ok(()),
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(&self.data)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        fs::write(path, data)
    }
}

fn activity_store_path() -> Option<PathBuf> {
    let mut base = config_root()?;
    base.push("runkit");
    base.push("activity.json");
    Some(base)
}

fn config_root() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("RUNKIT_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(dir));
    }
    env::var_os("HOME").map(|home| {
        let mut path = PathBuf::from(home);
        path.push(".config");
        path
    })
}

const MIN_REFRESH_INTERVAL: u32 = 5;
const MAX_REFRESH_INTERVAL: u32 = 3600;
const MIN_LOG_LINES: u32 = 10;
const MAX_LOG_LINES: u32 = 2000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum StartupBehavior {
    RememberLastService,
    ShowOverview,
}

impl Default for StartupBehavior {
    fn default() -> Self {
        StartupBehavior::ShowOverview
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct UserPreferences {
    auto_refresh: bool,
    refresh_interval_secs: u32,
    log_lines: u32,
    startup_behavior: StartupBehavior,
    show_all_services: bool,
    #[serde(default = "default_true")]
    require_password: bool,
    last_service: Option<String>,
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            auto_refresh: true,
            refresh_interval_secs: 30,
            log_lines: 200,
            startup_behavior: StartupBehavior::ShowOverview,
            show_all_services: true,
            require_password: true,
            last_service: None,
        }
    }
}

fn preferences_path() -> Option<PathBuf> {
    let mut base = config_root()?;
    base.push("runkit");
    base.push("preferences.json");
    Some(base)
}

fn load_user_preferences() -> UserPreferences {
    let mut prefs = preferences_path()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|data| serde_json::from_str::<UserPreferences>(&data).ok())
        .unwrap_or_default();
    normalize_preferences(&mut prefs);
    prefs
}

fn save_user_preferences(prefs: &UserPreferences) -> io::Result<()> {
    let Some(path) = preferences_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(prefs)
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
    fs::write(path, data)
}

fn normalize_preferences(prefs: &mut UserPreferences) {
    prefs.refresh_interval_secs = prefs
        .refresh_interval_secs
        .clamp(MIN_REFRESH_INTERVAL, MAX_REFRESH_INTERVAL);
    prefs.log_lines = prefs.log_lines.clamp(MIN_LOG_LINES, MAX_LOG_LINES);
    if prefs.startup_behavior == StartupBehavior::ShowOverview {
        prefs.last_service = None;
    }
}

impl AppController {
    fn new(app: &Application, dispatcher: ActionDispatcher) -> Rc<Self> {
        let preferences = load_user_preferences();
        let widgets = ui::AppWidgets::new(app, preferences.show_all_services);
        let description_store = DescriptionStore::load();
        let activity_store = ActivityStore::load();

        // Load previous states from activity store
        let previous_states = activity_store.get_previous_states();

        let mut model = AppModel::default();
        model.previous_states = previous_states;

        let controller = Rc::new(Self {
            dispatcher,
            model: Rc::new(RefCell::new(model)),
            widgets,
            description_store: RefCell::new(description_store),
            activity_store: RefCell::new(activity_store),
            preferences_window: RefCell::new(None),
            about_dialog: RefCell::new(None),
            preferences: RefCell::new(preferences),
            refresh_source: RefCell::new(None),
        });
        controller.setup_handlers();
        controller.configure_auto_refresh();
        controller
    }

    fn setup_handlers(self: &Rc<Self>) {
        let controller = Rc::clone(self);
        self.widgets
            .search_entry
            .connect_search_changed(move |entry| {
                controller.on_search_changed(entry.text().to_string());
            });

        {
            let controller = Rc::clone(self);
            let toggle = self.widgets.service_filter_toggle.clone();
            toggle.connect_toggled(move |button| {
                let show_all = button.is_active();
                controller
                    .widgets
                    .update_service_filter_toggle_label(show_all);
                let mut changed = false;
                {
                    let mut prefs = controller.preferences.borrow_mut();
                    if prefs.show_all_services != show_all {
                        prefs.show_all_services = show_all;
                        changed = true;
                    }
                }
                if changed {
                    controller.save_preferences();
                    controller.render_service_list();
                    controller.refresh_logs_for_selection();
                }
            });
        }

        let controller = Rc::clone(self);
        self.widgets
            .list_box
            .connect_row_selected(move |_, row| controller.on_row_selected(row));

        let register_action = |button: &gtk::Button, action: &'static str| {
            let controller = Rc::clone(self);
            button.connect_clicked(move |_| {
                controller.trigger_action(action);
            });
        };

        register_action(&self.widgets.action_start, "start");
        register_action(&self.widgets.action_stop, "stop");
        register_action(&self.widgets.action_restart, "restart");
        register_action(&self.widgets.action_reload, "reload");
        register_action(&self.widgets.action_enable, "enable");
        register_action(&self.widgets.action_disable, "disable");
        register_action(&self.widgets.action_check, "check");

        {
            let controller = Rc::clone(self);
            let popover = self.widgets.menu_popover.clone();
            self.widgets
                .preferences_action
                .connect_activate(move |_, _| {
                    popover.popdown();
                    controller.show_preferences();
                });
        }

        {
            let controller = Rc::clone(self);
            let popover = self.widgets.menu_popover.clone();
            self.widgets.about_action.connect_activate(move |_, _| {
                popover.popdown();
                controller.show_about();
            });
        }
    }

    fn request_initial_load(self: &Rc<Self>) {
        self.widgets.show_loading(true);
        let result = self.dispatcher.fetch_services();
        self.widgets.show_loading(false);
        match result {
            Ok(services) => self.update_services(services),
            Err(err) => self.widgets.show_error(&err),
        }
    }

    fn on_search_changed(self: &Rc<Self>, text: String) {
        self.model.borrow_mut().filter_text = text.clone();
        let count = self.render_service_list();
        if text.is_empty() {
            self.widgets
                .update_status_summary(&self.model.borrow().services);
        } else {
            self.widgets.update_status_summary_filtered(&text, count);
        }
    }

    fn on_row_selected(self: &Rc<Self>, row: Option<&gtk::ListBoxRow>) {
        match row.and_then(|r| self.widgets.row_service_name(r)) {
            Some(service_name) => {
                let service = {
                    self.model
                        .borrow()
                        .services
                        .iter()
                        .find(|service| service.name == service_name)
                        .cloned()
                };

                if let Some(service) = service {
                    let name = service.name.clone();

                    let service_changed = {
                        let model = self.model.borrow();
                        model.log_service.as_deref() != Some(name.as_str())
                    };
                    {
                        let mut model = self.model.borrow_mut();
                        if service_changed {
                            model.log_service = Some(name.clone());
                            model.log_entries.clear();
                            model.log_error = None;
                            model.activity_notes.clear();
                        }
                        model.current_description = service.description.clone();
                        model.description_error = None;
                    }

                    if let Some(text) = service.description.as_deref() {
                        self.description_store
                            .borrow_mut()
                            .ensure_present(&name, text);
                    }

                    self.widgets.show_service_details(&service);
                    self.widgets.action_bar_set_enabled(true, Some(&service));
                    self.ensure_service_description(&service);

                    let remember_last = {
                        let prefs = self.preferences.borrow();
                        prefs.startup_behavior == StartupBehavior::RememberLastService
                    };
                    if remember_last {
                        let mut prefs = self.preferences.borrow_mut();
                        if prefs.last_service.as_deref() != Some(name.as_str()) {
                            prefs.last_service = Some(name.clone());
                            drop(prefs);
                            self.save_preferences();
                        } else {
                            drop(prefs);
                        }
                    }

                    let (entries_snapshot, error_snapshot, notes_snapshot) = {
                        let model = self.model.borrow();
                        (
                            model.log_entries.clone(),
                            model.log_error.clone(),
                            model.activity_notes.clone(),
                        )
                    };

                    if let Some(error) = error_snapshot {
                        self.widgets.show_activity_error(&name, &error);
                    } else if !entries_snapshot.is_empty() || !notes_snapshot.is_empty() {
                        self.widgets
                            .show_activity(&name, &entries_snapshot, &notes_snapshot);
                    } else {
                        self.request_logs(name);
                    }
                }
            }
            None => {
                if self.model.borrow().list_refreshing {
                    return;
                }
                self.widgets.show_placeholder();
                self.widgets.action_bar_set_enabled(false, None);
                let mut model = self.model.borrow_mut();
                model.log_service = None;
                model.log_entries.clear();
                model.log_error = None;
                model.current_description = None;
                model.description_error = None;
                model.activity_notes.clear();
            }
        }
    }

    fn update_services(self: &Rc<Self>, services: Vec<ServiceInfo>) {
        {
            let mut store = self.description_store.borrow_mut();
            for service in &services {
                if let Some(description) = service.description.as_deref() {
                    store.ensure_present(&service.name, description);
                }
            }
        }

        // Detect state changes and record activity
        {
            let mut activity_store = self.activity_store.borrow_mut();
            let mut model = self.model.borrow_mut();
            let is_initial_load = !model.initial_load_completed;

            for service in &services {
                let current_state = service.runtime_state.state_name();
                let current_pid = service.runtime_state.pid();

                if let Some((prev_state, prev_pid)) = model.previous_states.get(&service.name) {
                    // Check if state changed
                    if prev_state != current_state {
                        let event = ActivityEvent::new(ActivityEventType::StateChange {
                            from_state: prev_state.to_string(),
                            to_state: current_state.to_string(),
                            pid: current_pid,
                        });
                        activity_store.add_event(&service.name, event);
                    }
                    // Check if PID changed while running (indicates restart)
                    else if current_state == "running" && prev_pid != &current_pid {
                        let event = ActivityEvent::new(ActivityEventType::StateChange {
                            from_state: "running".to_string(),
                            to_state: "running".to_string(),
                            pid: current_pid,
                        });
                        activity_store.add_event(&service.name, event);
                    }
                }

                // Update the previous state
                model
                    .previous_states
                    .insert(service.name.clone(), (current_state.to_string(), current_pid));
            }

            // Mark initial load as completed
            if is_initial_load {
                model.initial_load_completed = true;
            }

            // Save previous states to disk
            activity_store.update_previous_states(model.previous_states.clone());
        }

        let pending_selection = {
            let prefs = self.preferences.borrow();
            if prefs.startup_behavior == StartupBehavior::RememberLastService {
                prefs.last_service.as_ref().and_then(|name| {
                    services
                        .iter()
                        .find(|svc| svc.name == *name)
                        .and_then(|svc| {
                            if prefs.show_all_services || svc.enabled {
                                Some(name.clone())
                            } else {
                                None
                            }
                        })
                })
            } else {
                None
            }
        };
        {
            let mut model = self.model.borrow_mut();
            model.services = services;
            model.pending_selection = pending_selection;
        }
        self.widgets
            .update_status_summary(&self.model.borrow().services);
        self.render_service_list();
        self.refresh_logs_for_selection();
        self.refresh_description_for_selection();
    }

    fn render_service_list(self: &Rc<Self>) -> usize {
        let show_all = self.preferences.borrow().show_all_services;
        self.widgets.update_service_filter_toggle_label(show_all);
        let filtered = {
            let model = self.model.borrow();
            let filter = model.filter_text.to_lowercase();
            model
                .services
                .iter()
                .filter(|service| {
                    if !show_all && !service.enabled {
                        return false;
                    }
                    if filter.is_empty() {
                        return true;
                    }
                    service.name.to_lowercase().contains(&filter)
                        || service
                            .description
                            .as_ref()
                            .map(|d| d.to_lowercase().contains(&filter))
                            .unwrap_or(false)
                })
                .cloned()
                .collect::<Vec<_>>()
        };

        let count = filtered.len();
        {
            let mut model = self.model.borrow_mut();
            model.list_refreshing = true;
        }
        self.widgets.populate_list(&filtered);
        let pending = {
            let mut model = self.model.borrow_mut();
            model.list_refreshing = false;
            model.pending_selection.take()
        };
        if let Some(target) = pending {
            self.widgets.select_service(&target);
        }
        if self.widgets.current_service().is_none() {
            let mut model = self.model.borrow_mut();
            model.log_service = None;
            model.log_entries.clear();
            model.log_error = None;
            model.activity_notes.clear();
        }
        count
    }

    fn trigger_action(self: &Rc<Self>, action: &'static str) {
        if let Some(service_name) = self.widgets.current_service() {
            let allow_cached = {
                let prefs = self.preferences.borrow();
                !prefs.require_password
            };
            match self.dispatcher.run(action, &service_name, allow_cached) {
                Ok(message) => {
                    // Record successful user action
                    {
                        let mut activity_store = self.activity_store.borrow_mut();
                        let event = ActivityEvent::new(ActivityEventType::UserAction {
                            action: action.to_string(),
                            success: true,
                            error: None,
                        });
                        activity_store.add_event(&service_name, event);
                    }

                    let (entries_snapshot, notes_snapshot) = {
                        let mut model = self.model.borrow_mut();
                        if model.log_service.as_deref() != Some(service_name.as_str()) {
                            model.log_service = Some(service_name.clone());
                            model.log_entries.clear();
                            model.log_error = None;
                            model.activity_notes.clear();
                        }
                        model.log_error = None;
                        model.activity_notes.insert(0, message.clone());
                        if model.activity_notes.len() > 20 {
                            model.activity_notes.truncate(20);
                        }
                        (model.log_entries.clone(), model.activity_notes.clone())
                    };
                    self.widgets
                        .show_activity(&service_name, &entries_snapshot, &notes_snapshot);
                    self.request_refresh(true);
                }
                Err(err) => {
                    // Record failed user action
                    {
                        let mut activity_store = self.activity_store.borrow_mut();
                        let event = ActivityEvent::new(ActivityEventType::UserAction {
                            action: action.to_string(),
                            success: false,
                            error: Some(err.to_string()),
                        });
                        activity_store.add_event(&service_name, event);
                    }

                    let error_message = format!("Operation failed: {err}");
                    let (entries_snapshot, notes_snapshot) = {
                        let mut model = self.model.borrow_mut();
                        if model.log_service.as_deref() != Some(service_name.as_str()) {
                            model.log_service = Some(service_name.clone());
                            model.log_entries.clear();
                            model.log_error = None;
                            model.activity_notes.clear();
                        }
                        model.log_error = Some(error_message.clone());
                        model.activity_notes.insert(0, error_message.clone());
                        if model.activity_notes.len() > 20 {
                            model.activity_notes.truncate(20);
                        }
                        (model.log_entries.clone(), model.activity_notes.clone())
                    };
                    self.widgets
                        .show_activity(&service_name, &entries_snapshot, &notes_snapshot);
                }
            }
        }
    }

    fn request_refresh(self: &Rc<Self>, silent: bool) {
        if !silent {
            self.widgets.show_loading(true);
        }
        let result = self.dispatcher.fetch_services();
        self.widgets.show_loading(false);
        match result {
            Ok(services) => self.update_services(services),
            Err(err) => self.widgets.show_error(&err),
        }
    }

    fn request_logs(self: &Rc<Self>, service: String) {
        self.widgets.show_activity_loading(&service);
        let lines = self.preferences.borrow().log_lines.max(1) as usize;
        match self.dispatcher.fetch_logs(&service, lines) {
            Ok(entries) => {
                let mut notes = {
                    let model = self.model.borrow();
                    model.activity_notes.clone()
                };

                // Prepend activity history to notes
                let activity_history = self.format_activity_history(&service);
                if !activity_history.is_empty() {
                    // Insert activity history at the beginning
                    for (i, activity_line) in activity_history.into_iter().enumerate() {
                        notes.insert(i, activity_line);
                    }
                }

                {
                    let mut model = self.model.borrow_mut();
                    model.log_service = Some(service.clone());
                    model.log_entries = entries.clone();
                    model.log_error = None;
                }
                self.widgets.show_activity(&service, &entries, &notes);
            }
            Err(err) => {
                {
                    let mut model = self.model.borrow_mut();
                    model.log_service = Some(service.clone());
                    model.log_entries.clear();
                    model.log_error = Some(err.clone());
                }
                self.widgets.show_activity_error(&service, &err);
            }
        }
    }

    fn format_activity_history(&self, service: &str) -> Vec<String> {
        let activity_store = self.activity_store.borrow();
        let activities = activity_store.get_activities(service);

        activities
            .into_iter()
            .rev() // Show most recent first
            .map(|event| self.format_activity_event(&event))
            .collect()
    }

    fn format_activity_event(&self, event: &ActivityEvent) -> String {
        use chrono::DateTime;

        let timestamp = DateTime::parse_from_rfc3339(&event.timestamp)
            .ok()
            .and_then(|dt| {
                let local: chrono::DateTime<chrono::Local> = dt.into();
                Some(local.format("%b %d, %I:%M %p").to_string())
            })
            .unwrap_or_else(|| "Unknown time".to_string());

        match &event.event_type {
            ActivityEventType::StateChange {
                from_state,
                to_state,
                pid,
            } => {
                if from_state == to_state && to_state == "running" {
                    // This is a restart (PID change while running)
                    if let Some(pid) = pid {
                        format!("Service restarted (PID {}) - {}", pid, timestamp)
                    } else {
                        format!("Service restarted - {}", timestamp)
                    }
                } else {
                    let to_desc = match to_state.as_str() {
                        "running" => "started",
                        "down" => "stopped",
                        "failed" => "failed",
                        _ => to_state.as_str(),
                    };
                    if let Some(pid) = pid {
                        format!("Service {} (PID {}) - {}", to_desc, pid, timestamp)
                    } else {
                        format!("Service {} - {}", to_desc, timestamp)
                    }
                }
            }
            ActivityEventType::UserAction {
                action,
                success,
                error,
            } => {
                if *success {
                    format!("User action: {} - {}", action, timestamp)
                } else if let Some(err) = error {
                    format!("User action: {} failed ({}) - {}", action, err, timestamp)
                } else {
                    format!("User action: {} failed - {}", action, timestamp)
                }
            }
        }
    }

    fn refresh_logs_for_selection(self: &Rc<Self>) {
        if let Some(service_name) = self.widgets.current_service() {
            self.request_logs(service_name);
        }
    }

    fn refresh_description_for_selection(self: &Rc<Self>) {
        if let Some(service_name) = self.widgets.current_service() {
            let service = {
                self.model
                    .borrow()
                    .services
                    .iter()
                    .find(|svc| svc.name == service_name)
                    .cloned()
            };
            if let Some(service) = service {
                self.ensure_service_description(&service);
            }
        }
    }

    fn save_preferences(&self) {
        let mut snapshot = self.preferences.borrow().clone();
        normalize_preferences(&mut snapshot);
        if let Err(err) = save_user_preferences(&snapshot) {
            eprintln!("Failed to save preferences: {err}");
        } else {
            *self.preferences.borrow_mut() = snapshot;
        }
    }

    fn clear_auto_refresh(&self) {
        if let Some(source) = self.refresh_source.borrow_mut().take() {
            source.remove();
        }
    }

    fn configure_auto_refresh(self: &Rc<Self>) {
        self.clear_auto_refresh();
        let prefs = self.preferences.borrow().clone();
        if prefs.auto_refresh {
            let interval = prefs
                .refresh_interval_secs
                .clamp(MIN_REFRESH_INTERVAL, MAX_REFRESH_INTERVAL);
            let controller = Rc::downgrade(self);
            let source = glib::timeout_add_seconds_local(interval, move || {
                if let Some(controller) = controller.upgrade() {
                    controller.request_refresh(true);
                }
                ControlFlow::Continue
            });
            self.refresh_source.borrow_mut().replace(source);
        }
    }

    fn show_preferences(self: &Rc<Self>) {
        if let Some(window) = self.preferences_window.borrow().as_ref() {
            window.present();
            return;
        }

        let window = adw::PreferencesWindow::builder()
            .transient_for(&self.widgets.window)
            .modal(true)
            .title("Preferences")
            .build();
        let prefs_snapshot = self.preferences.borrow().clone();

        let page = adw::PreferencesPage::builder().title("General").build();

        let startup_group = adw::PreferencesGroup::builder().title("Startup").build();
        let startup_options =
            gtk::StringList::new(&["Remember last selected service", "Show overview"]);
        let startup_combo = adw::ComboRow::builder()
            .title("When Runkit opens")
            .model(&startup_options)
            .build();
        let selected_index = match prefs_snapshot.startup_behavior {
            StartupBehavior::RememberLastService => 0,
            StartupBehavior::ShowOverview => 1,
        };
        startup_combo.set_selected(selected_index);
        startup_group.add(&startup_combo);

        let visibility_row = adw::ActionRow::builder()
            .title("Show disabled services")
            .subtitle("Include services that are not enabled under /var/service.")
            .build();
        let show_switch = gtk::Switch::builder()
            .valign(gtk::Align::Center)
            .active(prefs_snapshot.show_all_services)
            .build();
        visibility_row.add_suffix(&show_switch);
        visibility_row.set_activatable_widget(Some(&show_switch));
        startup_group.add(&visibility_row);

        let refresh_group = adw::PreferencesGroup::builder()
            .title("Status Refresh")
            .description("Control how Runkit keeps service status up to date.")
            .build();

        let auto_row = adw::ActionRow::builder()
            .title("Refresh services automatically")
            .build();
        let auto_switch = gtk::Switch::builder()
            .valign(gtk::Align::Center)
            .active(prefs_snapshot.auto_refresh)
            .build();
        auto_row.add_suffix(&auto_switch);
        auto_row.set_activatable_widget(Some(&auto_switch));
        refresh_group.add(&auto_row);

        let auth_row = adw::ActionRow::builder()
            .title("Require password for service changes")
            .subtitle("Ask for authentication every time a privileged action runs.")
            .build();
        let auth_switch = gtk::Switch::builder()
            .valign(gtk::Align::Center)
            .active(prefs_snapshot.require_password)
            .build();
        auth_row.add_suffix(&auth_switch);
        auth_row.set_activatable_widget(Some(&auth_switch));
        refresh_group.add(&auth_row);

        let interval_adjustment = gtk::Adjustment::new(
            prefs_snapshot.refresh_interval_secs as f64,
            MIN_REFRESH_INTERVAL as f64,
            MAX_REFRESH_INTERVAL as f64,
            1.0,
            10.0,
            0.0,
        );
        let interval_spin = gtk::SpinButton::builder()
            .adjustment(&interval_adjustment)
            .digits(0)
            .valign(gtk::Align::Center)
            .build();
        interval_spin.set_numeric(true);
        interval_spin.set_sensitive(prefs_snapshot.auto_refresh);
        let interval_row = adw::ActionRow::builder()
            .title("Refresh interval (seconds)")
            .build();
        interval_row.add_suffix(&interval_spin);
        interval_row.set_activatable(false);
        refresh_group.add(&interval_row);

        let log_group = adw::PreferencesGroup::builder()
            .title("Log Fetch")
            .description("Adjust how many log entries are retrieved when viewing service activity.")
            .build();
        let log_adjustment = gtk::Adjustment::new(
            prefs_snapshot.log_lines as f64,
            MIN_LOG_LINES as f64,
            MAX_LOG_LINES as f64,
            10.0,
            50.0,
            0.0,
        );
        let log_spin = gtk::SpinButton::builder()
            .adjustment(&log_adjustment)
            .digits(0)
            .valign(gtk::Align::Center)
            .build();
        log_spin.set_numeric(true);
        let log_row = adw::ActionRow::builder()
            .title("Maximum log lines")
            .subtitle("Applies when loading service activity logs.")
            .build();
        log_row.add_suffix(&log_spin);
        log_row.set_activatable(false);
        log_group.add(&log_row);

        page.add(&startup_group);
        page.add(&refresh_group);
        page.add(&log_group);
        window.add(&page);

        let interval_spin_clone = interval_spin.clone();
        let controller_for_auto = Rc::downgrade(self);
        auto_switch.connect_state_set(move |_, state| {
            interval_spin_clone.set_sensitive(state);
            if let Some(controller) = controller_for_auto.upgrade() {
                let mut changed = false;
                {
                    let mut prefs = controller.preferences.borrow_mut();
                    if prefs.auto_refresh != state {
                        prefs.auto_refresh = state;
                        changed = true;
                    }
                }
                if changed {
                    controller.save_preferences();
                    controller.configure_auto_refresh();
                }
            }
            glib::Propagation::Proceed
        });

        let controller_for_auth = Rc::downgrade(self);
        auth_switch.connect_state_set(move |_, state| {
            if let Some(controller) = controller_for_auth.upgrade() {
                let mut changed = false;
                {
                    let mut prefs = controller.preferences.borrow_mut();
                    if prefs.require_password != state {
                        prefs.require_password = state;
                        changed = true;
                    }
                }
                if changed {
                    controller.save_preferences();
                }
            }
            glib::Propagation::Proceed
        });

        let controller_for_interval = Rc::downgrade(self);
        interval_spin.connect_value_changed(move |spin| {
            if let Some(controller) = controller_for_interval.upgrade() {
                let value = spin
                    .value()
                    .round()
                    .clamp(MIN_REFRESH_INTERVAL as f64, MAX_REFRESH_INTERVAL as f64)
                    as u32;
                let mut changed = false;
                {
                    let mut prefs = controller.preferences.borrow_mut();
                    if prefs.refresh_interval_secs != value {
                        prefs.refresh_interval_secs = value;
                        changed = true;
                    }
                }
                if changed {
                    controller.save_preferences();
                    if controller.preferences.borrow().auto_refresh {
                        controller.configure_auto_refresh();
                    }
                }
            }
        });

        let controller_for_log = Rc::downgrade(self);
        log_spin.connect_value_changed(move |spin| {
            if let Some(controller) = controller_for_log.upgrade() {
                let value = spin
                    .value()
                    .round()
                    .clamp(MIN_LOG_LINES as f64, MAX_LOG_LINES as f64)
                    as u32;
                let mut changed = false;
                {
                    let mut prefs = controller.preferences.borrow_mut();
                    if prefs.log_lines != value {
                        prefs.log_lines = value;
                        changed = true;
                    }
                }
                if changed {
                    controller.save_preferences();
                    if let Some(current) = controller.widgets.current_service() {
                        controller.request_logs(current);
                    }
                }
            }
        });

        let controller_for_startup = Rc::downgrade(self);
        startup_combo.connect_selected_notify(move |combo| {
            if let Some(controller) = controller_for_startup.upgrade() {
                let behavior = if combo.selected() == 0 {
                    StartupBehavior::RememberLastService
                } else {
                    StartupBehavior::ShowOverview
                };
                let mut pending_selection = None;
                let mut changed = false;
                {
                    let mut prefs = controller.preferences.borrow_mut();
                    if prefs.startup_behavior != behavior {
                        prefs.startup_behavior = behavior;
                        changed = true;
                        if behavior == StartupBehavior::RememberLastService {
                            let current = controller.widgets.current_service();
                            prefs.last_service = current.clone();
                            pending_selection = prefs.last_service.clone();
                        } else {
                            prefs.last_service = None;
                        }
                    }
                }
                if changed {
                    controller.save_preferences();
                    match behavior {
                        StartupBehavior::ShowOverview => {
                            controller.widgets.list_box.unselect_all();
                            controller.widgets.show_placeholder();
                        }
                        StartupBehavior::RememberLastService => {
                            if let Some(target) = pending_selection {
                                controller.model.borrow_mut().pending_selection = Some(target);
                                controller.render_service_list();
                            }
                        }
                    }
                }
            }
        });

        let controller_for_visibility = Rc::downgrade(self);
        show_switch.connect_state_set(move |_, state| {
            if let Some(controller) = controller_for_visibility.upgrade() {
                let mut changed = false;
                {
                    let mut prefs = controller.preferences.borrow_mut();
                    if prefs.show_all_services != state {
                        prefs.show_all_services = state;
                        changed = true;
                    }
                }
                if changed {
                    controller.save_preferences();
                    controller.widgets.set_service_filter_toggle(state);
                    controller.render_service_list();
                    controller.refresh_logs_for_selection();
                }
            }
            glib::Propagation::Proceed
        });

        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(controller) = weak.upgrade() {
                controller.preferences_window.borrow_mut().take();
            }
            glib::Propagation::Proceed
        });

        let weak_hide = Rc::downgrade(self);
        window.connect_hide(move |_| {
            if let Some(controller) = weak_hide.upgrade() {
                controller.preferences_window.borrow_mut().take();
            }
        });

        let window_clone = window.clone();
        self.preferences_window.borrow_mut().replace(window_clone);
        window.present();
    }

    fn show_about(self: &Rc<Self>) {
        if let Some(dialog) = self.about_dialog.borrow().as_ref() {
            dialog.present();
            return;
        }

        let dialog = adw::MessageDialog::builder()
            .transient_for(&self.widgets.window)
            .modal(true)
            .build();

        let content_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .halign(gtk::Align::Center)
            .build();
        content_box.set_margin_top(12);
        content_box.set_margin_bottom(12);

        let logo = gtk::Image::from_icon_name("runkit");
        logo.set_pixel_size(96);
        logo.set_valign(gtk::Align::Center);
        logo.set_halign(gtk::Align::Center);
        content_box.append(&logo);

        let title = gtk::Label::builder()
            .label("Runkit")
            .css_classes(["title-1"])
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .halign(gtk::Align::Center)
            .build();
        content_box.append(&title);

        let version = gtk::Label::builder()
            .label(&format!("Version {}", env!("CARGO_PKG_VERSION")))
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .css_classes(["dim-label"])
            .halign(gtk::Align::Center)
            .build();
        version.set_xalign(0.5);
        content_box.append(&version);

        let description = gtk::Label::builder()
            .label("Graphical manager for Void Linux runit services.")
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .css_classes(["dim-label"])
            .halign(gtk::Align::Center)
            .build();
        description.set_xalign(0.5);
        content_box.append(&description);

        let links_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .halign(gtk::Align::Center)
            .build();

        let make_link = |text: &str, url: &str| {
            let link = gtk::LinkButton::builder().label(text).uri(url).build();
            link.add_css_class("flat");
            link
        };

        links_row.append(&make_link(
            "Project website",
            "https://github.com/letdown2491/runkit",
        ));

        let separator = gtk::Label::builder()
            .label("/")
            .halign(gtk::Align::Center)
            .build();
        separator.add_css_class("dim-label");
        links_row.append(&separator);

        links_row.append(&make_link(
            "Report an issue",
            "https://github.com/letdown2491/runkit/issues",
        ));

        content_box.append(&links_row);

        dialog.set_extra_child(Some(&content_box));
        dialog.add_response("close", "Close");
        dialog.set_default_response(Some("close"));
        dialog.connect_response(None, |dialog: &adw::MessageDialog, _response| {
            dialog.close()
        });

        let weak = Rc::downgrade(self);
        dialog.connect_close_request(move |_| {
            if let Some(controller) = weak.upgrade() {
                controller.about_dialog.borrow_mut().take();
            }
            glib::Propagation::Proceed
        });

        let weak = Rc::downgrade(self);
        dialog.connect_hide(move |_| {
            if let Some(controller) = weak.upgrade() {
                controller.about_dialog.borrow_mut().take();
            }
        });

        let dialog_clone = dialog.clone();
        self.about_dialog.borrow_mut().replace(dialog_clone);
        dialog.present();
    }

    fn ensure_service_description(self: &Rc<Self>, service: &ServiceInfo) {
        let name = service.name.clone();

        if let Some(existing) = service.description.clone() {
            self.record_description(&name, Some(existing));
            return;
        }

        if let Some(saved) = self.description_store.borrow().lookup(&name) {
            self.record_description(&name, saved);
            return;
        }

        self.widgets.show_description_loading(&name);
        match self.dispatcher.fetch_description(&name) {
            Ok(description) => {
                if let Err(err) = self
                    .description_store
                    .borrow_mut()
                    .store(&name, description.clone())
                {
                    eprintln!("Failed to persist description for {name}: {err}");
                }
                self.record_description(&name, description);
            }
            Err(err) => {
                self.record_description_error(&name, err);
            }
        }
    }

    fn record_description(self: &Rc<Self>, service: &str, description: Option<String>) {
        let description_clone = description.clone();
        {
            let mut model = self.model.borrow_mut();
            if let Some(entry) = model.services.iter_mut().find(|info| info.name == service) {
                entry.description = description.clone();
            }
            if model.log_service.as_deref() == Some(service) {
                model.current_description = description.clone();
                model.description_error = None;
            }
        }
        self.widgets.show_description(description_clone.as_deref());
    }

    fn record_description_error(self: &Rc<Self>, service: &str, error: String) {
        {
            let mut model = self.model.borrow_mut();
            if model.log_service.as_deref() == Some(service) {
                model.current_description = None;
                model.description_error = Some(error.clone());
            }
        }
        self.widgets.show_description_error(service, &error);
    }
}
