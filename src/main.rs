mod activity_window;
#[cfg(target_os = "macos")]
mod canvas;
mod login_item;
mod network;
#[cfg(target_os = "macos")]
mod popover;
#[cfg(target_os = "macos")]
mod settings_window;

use gauge::{
    calendar, config,
    dashboard::DashboardSnapshot,
    devices::{DeviceStore, PairedDevice},
    fetch_enabled, meter_groups, now_seconds,
    provisioning::{self, DiscoveredAccessory, PairingRequest},
    summary, tray_summary, MeterGroup, Usage,
};
use std::{
    env, process,
    sync::{mpsc, Arc, OnceLock},
    time::{Duration, Instant},
};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use winit::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoop},
    platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS},
};

#[cfg(target_os = "macos")]
mod mac_wifi {
    use objc2::rc::Retained;
    use objc2::{extern_class, extern_methods};
    use objc2_core_location::{CLAuthorizationStatus, CLLocationManager};
    use objc2_foundation::{NSObject, NSString};

    #[link(name = "CoreWLAN", kind = "framework")]
    unsafe extern "C" {}

    extern_class!(
        #[unsafe(super(NSObject))]
        struct CWWiFiClient;
    );

    extern_class!(
        #[unsafe(super(NSObject))]
        struct CWInterface;
    );

    impl CWWiFiClient {
        extern_methods!(
            #[unsafe(method(sharedWiFiClient))]
            #[unsafe(method_family = none)]
            unsafe fn shared() -> Retained<Self>;

            #[unsafe(method(interface))]
            #[unsafe(method_family = none)]
            unsafe fn interface(&self) -> Option<Retained<CWInterface>>;
        );
    }

    impl CWInterface {
        extern_methods!(
            #[unsafe(method(ssid))]
            #[unsafe(method_family = none)]
            unsafe fn ssid(&self) -> Option<Retained<NSString>>;
        );
    }

    thread_local! {
        static LOCATION_MANAGER: Retained<CLLocationManager> = unsafe { CLLocationManager::new() };
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum LocationAccess {
        NotDetermined,
        Authorized,
        Unavailable,
    }

    pub fn location_access() -> LocationAccess {
        if !unsafe { CLLocationManager::locationServicesEnabled_class() } {
            return LocationAccess::Unavailable;
        }
        LOCATION_MANAGER.with(|manager| {
            let status = unsafe { manager.authorizationStatus() };
            if status == CLAuthorizationStatus::NotDetermined {
                LocationAccess::NotDetermined
            } else if matches!(
                status,
                CLAuthorizationStatus::AuthorizedAlways
                    | CLAuthorizationStatus::AuthorizedWhenInUse
            ) {
                LocationAccess::Authorized
            } else {
                LocationAccess::Unavailable
            }
        })
    }

    pub fn request_location_access() {
        LOCATION_MANAGER.with(|manager| unsafe { manager.requestWhenInUseAuthorization() });
    }

    pub fn current_network_name() -> Option<String> {
        let client = unsafe { CWWiFiClient::shared() };
        let interface = unsafe { client.interface() }?;
        let ssid = unsafe { interface.ssid() }?;
        let name = ssid.to_string();
        (!name.trim().is_empty()).then(|| name.trim().to_owned())
    }
}

const HELP: &str = "Gauge\n\
    \n\
    gauge                  Show remaining agent quota\n\
    gauge --json           Print the same data as JSON\n\
    gauge --tray           Keep it in the menu bar\n\
    gauge --settings       Open the settings file in an editor\n\
    gauge --stats          Token history and pending requests as JSON\n\
    gauge --install-hooks  Enable attention monitoring for both agents\n\
    gauge --version        Print the version";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Cli,
    Json,
    Tray,
    Settings,
    Stats,
    InstallHooks,
}

fn main() {
    if let Err(error) = run(env::args().skip(1)) {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn run(args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut args = args.peekable();
    if args.peek().is_some_and(|arg| arg == "--hook") {
        args.next();
        let provider = match args.next().as_deref() {
            Some("codex") => "Codex",
            Some("claude") => "Claude",
            _ => return Err("expected --hook codex|claude".into()),
        };
        // Hook failures must never stop agent work or produce approval output.
        if let Err(e) = gauge::attention::receive(provider) {
            eprintln!("Gauge hook: {e}");
        }
        return Ok(());
    }
    let mut mode = if args.peek().is_none() && launched_from_app_bundle() {
        Mode::Tray
    } else {
        Mode::Cli
    };

    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("gauge {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--stats" => set_mode(&mut mode, Mode::Stats)?,
            "--install-hooks" => set_mode(&mut mode, Mode::InstallHooks)?,
            "--json" => set_mode(&mut mode, Mode::Json)?,
            "--tray" => set_mode(&mut mode, Mode::Tray)?,
            "--settings" => set_mode(&mut mode, Mode::Settings)?,
            _ => return Err(format!("unknown argument: {arg}\n\n{HELP}")),
        }
    }

    match mode {
        Mode::Tray => {
            run_tray();
            Ok(())
        }
        Mode::Settings => open_configuration_file(),
        Mode::InstallHooks => {
            gauge::attention::install_hooks()?;
            println!("{}", gauge::attention::HOOK_SETUP_MESSAGE);
            Ok(())
        }
        Mode::Stats => {
            let enabled = providers();
            let data = gauge::activity::Monitoring {
                tokens: gauge::activity::UsageReader::default().collect(&enabled),
                attention: gauge::attention::collect(&enabled),
                ready: true,
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        Mode::Json => {
            let (usages, errors) = fetch_enabled(&providers());
            println!("{}", quota_json(&usages, &errors));
            Ok(())
        }
        Mode::Cli => {
            let (usages, errors) = fetch_enabled(&providers());

            if usages.is_empty() {
                return Err(format!(
                    "failed to fetch any provider data:\n- {}",
                    errors.join("\n- ")
                ));
            }

            println!("{}", summary(&usages));
            for error in errors {
                eprintln!("warning: {error}");
            }
            Ok(())
        }
    }
}

/// The command-line paths honour the same provider switches as the menu bar,
/// falling back to both providers when settings cannot be read.
fn providers() -> config::ProviderConfig {
    config::load_or_create()
        .map(|config| config.providers)
        .unwrap_or_default()
}

fn launched_from_app_bundle() -> bool {
    // Use argv[0], not current_exe(): Homebrew exposes a CLI symlink to the
    // executable inside Gauge.app, and resolving that symlink would make a
    // plain `gauge` command look like a LaunchServices app launch.
    env::args_os()
        .next()
        .and_then(|path| path.to_str().map(str::to_owned))
        .is_some_and(|path| path.contains(".app/Contents/MacOS/"))
}

fn set_mode(mode: &mut Mode, requested: Mode) -> Result<(), String> {
    if *mode != Mode::Cli && *mode != requested {
        return Err(
            "choose only one mode: --json, --tray, --settings, --stats, or --install-hooks".into(),
        );
    }
    *mode = requested;
    Ok(())
}

fn quota_json(usages: &[Usage], errors: &[String]) -> String {
    let providers: Vec<_> = usages
        .iter()
        .map(|usage| {
            serde_json::json!({
                "name": usage.name,
                "remaining_percent": usage.remaining_percent(),
                "limits": usage.limits.iter().map(|limit| serde_json::json!({
                    "label": limit.label,
                    "used_percent": limit.used_percent,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    serde_json::json!({
        "generated_at": now_seconds(),
        "providers": providers,
        "errors": errors,
    })
    .to_string()
}

/// Keep the menu-bar title compact and show the dashboard in an anchored
/// popover. The quota detail grid is always visible, so the panel keeps one
/// stable layout instead of resizing in response to an expand/collapse action.
#[allow(deprecated)]
fn run_tray() {
    let device_store = match DeviceStore::open() {
        Ok(devices) => Some(Arc::new(devices)),
        Err(error) => {
            eprintln!("accessory pairing unavailable: {error}");
            None
        }
    };
    let dashboard = DashboardSnapshot::collect();
    let mut snapshot = TraySnapshot::from_dashboard(&dashboard, &paired(device_store.as_deref()));
    let mut title = snapshot.title.clone();
    let mut next_refresh = Instant::now() + refresh_interval(snapshot.refresh_seconds);
    let mut tray: Option<TrayIcon> = None;
    let events = TrayIconEvent::receiver();
    let (actions_tx, actions_rx) = mpsc::channel();
    let _ = ACTIONS.set(actions_tx);
    let mut popover: Option<PopoverUi> = None;
    let mut settings: Option<SettingsWindow> = None;
    let mut activity_window: Option<activity_window::ActivityWindow> = None;
    start_activity_monitor();
    let mut pairing_in_progress = false;
    let mut accessory_server = if dashboard.config.accessories.enabled {
        device_store.as_ref().and_then(|devices| {
            match start_accessory_server(&dashboard, Arc::clone(devices)) {
                Ok(server) => Some(server),
                Err(error) => {
                    eprintln!("accessory sharing unavailable: {error}");
                    None
                }
            }
        })
    } else {
        None
    };

    // Accessory keeps Gauge out of the Dock and the app switcher, which is what
    // a menu bar utility should do.
    let event_loop = EventLoop::builder()
        .with_activation_policy(ActivationPolicy::Accessory)
        .build()
        .expect("failed to start event loop");
    let _ = EVENT_LOOP_PROXY.set(event_loop.create_proxy());
    let _ = event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::WaitUntil(next_refresh));

        if let Event::NewEvents(StartCause::Init) = event {
            tray = Some(
                TrayIconBuilder::new()
                    .with_title(&title)
                    .with_tooltip("Gauge")
                    .build()
                    .expect("failed to build tray icon"),
            );
            popover = Some(PopoverUi::new(&snapshot));
        }

        while let Ok(event) = events.try_recv() {
            // macOS emits one event for mouse-down and another for mouse-up.
            // Acting on both immediately opened and then closed the popover.
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Down,
                    ..
                }
            ) {
                if let (Some(tray), Some(popover)) = (&tray, &popover) {
                    popover.toggle(tray, &snapshot);
                }
            }
        }

        while let Ok(action) = actions_rx.try_recv() {
            let mut should_refresh = false;
            match action {
                AppAction::TokensUpdated(tokens) => {
                    snapshot.monitoring.tokens = tokens;
                    snapshot.monitoring.retain_enabled(&providers());
                    snapshot.monitoring.ready = true;
                    if let Some(popover) = &popover {
                        popover.render(&snapshot);
                    }
                    if let Some(window) = &mut activity_window {
                        window.render(&snapshot.monitoring);
                    }
                }
                AppAction::AttentionUpdated(attention) => {
                    snapshot.monitoring.attention = attention;
                    title = snapshot.monitoring.title(&snapshot.title);
                    if let Some(tray) = &tray {
                        tray.set_title(Some(&title));
                    }
                    if let Some(popover) = &popover {
                        popover.render(&snapshot);
                    }
                    if let Some(window) = &mut activity_window {
                        window.render(&snapshot.monitoring);
                    }
                }
                AppAction::Activity => {
                    if let Some(popover) = &popover {
                        popover.dismiss();
                    }
                    activity_window
                        .get_or_insert_with(activity_window::ActivityWindow::new)
                        .show(&snapshot.monitoring);
                }
                AppAction::InstallHooks => {
                    if let Some(popover) = &popover {
                        popover.dismiss();
                    }
                    match gauge::attention::install_hooks() {
                        Ok(()) => {
                            snapshot.hooks_installed = true;
                            show_message(
                                "Agent hooks installed",
                                gauge::attention::HOOK_SETUP_MESSAGE,
                            );
                        }
                        Err(e) => show_message("Could not enable monitoring", &e),
                    }
                }
                AppAction::Quit => process::exit(0),
                AppAction::Settings => {
                    if let Some(popover) = &popover {
                        popover.dismiss();
                    }
                    let current = settings_snapshot(device_store.as_deref());
                    match &settings {
                        Some(window) => window.show(&current),
                        None => {
                            let window = SettingsWindow::new(&current);
                            window.show(&current);
                            settings = Some(window);
                        }
                    }
                }
                AppAction::OpenConfigurationFile => {
                    if let Err(error) = open_configuration_file() {
                        show_message("Could Not Open Settings", &error);
                    }
                }
                AppAction::SetSwitch(switch, on) => {
                    let applied = match switch {
                        Switch::Codex => config::update(|c| c.providers.codex = on).map(|_| ()),
                        Switch::Claude => config::update(|c| c.providers.claude = on).map(|_| ()),
                        Switch::Cursor => config::update(|c| c.providers.cursor = on).map(|_| ()),
                        Switch::Calendar => config::update(|c| c.calendar.enabled = on).map(|_| ()),
                        Switch::Tasks => config::update(|c| c.tasks.enabled = on).map(|_| ()),
                        Switch::OpenAtLogin => login_item::set_enabled(on),
                        Switch::Accessories => config::set_accessories_enabled(on).map(|_| ()),
                    };
                    match applied {
                        Ok(()) => {
                            if switch == Switch::Accessories {
                                apply_sharing(on, device_store.as_ref(), &mut accessory_server);
                            }
                            should_refresh = true;
                        }
                        Err(error) => show_message("Could Not Save", &error),
                    }
                }
                AppAction::SetRefreshSeconds(seconds) => {
                    match config::update(|config| config.refresh_seconds = seconds) {
                        Ok(_) => should_refresh = true,
                        Err(error) => show_message("Could Not Save", &error),
                    }
                }
                AppAction::ForgetDevice(index) => {
                    let device = settings
                        .as_ref()
                        .and_then(|window| window.device_id(index))
                        .zip(device_store.as_ref());
                    if let Some((device_id, devices)) = device {
                        let name = devices
                            .devices()
                            .into_iter()
                            .find(|device| device.id == device_id)
                            .map(|device| device.name)
                            .unwrap_or_else(|| device_id.clone());
                        if confirm(
                            &format!("Forget {name}?"),
                            "Gauge will stop sharing this dashboard with it. To finish, also \
                             remove it under System Settings › Bluetooth.",
                            "Forget",
                        ) {
                            match devices.revoke(&device_id) {
                                Ok(_) => should_refresh = true,
                                Err(error) => show_message("Could Not Forget", &error),
                            }
                        }
                    }
                }
                AppAction::ChooseAccessory(found, reply) => {
                    if let Some(popover) = &popover {
                        popover.dismiss();
                    }
                    let _ = reply.send(accessory_picker(&found));
                }
                AppAction::PairAccessory => {
                    if pairing_in_progress {
                        continue;
                    }
                    let Some(devices) = device_store.as_ref() else {
                        show_message(
                            "Accessories Unavailable",
                            "Gauge could not open its device registry or macOS Keychain.",
                        );
                        continue;
                    };
                    #[cfg(target_os = "macos")]
                    if mac_wifi::location_access() == mac_wifi::LocationAccess::NotDetermined {
                        mac_wifi::request_location_access();
                        continue;
                    }
                    if let Some(popover) = &popover {
                        popover.dismiss();
                    }
                    match pairing_editor() {
                        Ok(Some(request)) => {
                            match begin_pairing(Arc::clone(devices), &mut accessory_server, request)
                            {
                                Ok(()) => pairing_in_progress = true,
                                Err(error) => show_message("Could Not Pair", &error),
                            }
                        }
                        Ok(None) => {}
                        Err(error) => show_message("Could Not Pair", &error),
                    }
                }
                AppAction::PairingFinished(result) => {
                    pairing_in_progress = false;
                    match result {
                        Ok(name) => eprintln!("{name} paired and connected to Wi-Fi"),
                        Err(error) if error == provisioning::CANCELLED => {}
                        Err(error) => show_message("Pairing Failed", &error),
                    }
                    should_refresh = true;
                }
                AppAction::Refresh => should_refresh = true,
                AppAction::ToggleTodo(index) => match config::toggle_todo(index) {
                    Ok(()) => should_refresh = true,
                    Err(error) => eprintln!("warning: {error}"),
                },
                AppAction::EditTodo(index) => {
                    if let Some(popover) = &popover {
                        popover.dismiss();
                    }
                    match edit_todo_from_editor(index) {
                        Ok(changed) => should_refresh = changed,
                        Err(error) => eprintln!("warning: {error}"),
                    }
                }
                AppAction::DeleteTodo(index) => match config::delete_todo(index) {
                    Ok(()) => should_refresh = true,
                    Err(error) => eprintln!("warning: {error}"),
                },
                AppAction::AddTodo => {
                    if let Some(popover) = &popover {
                        popover.dismiss();
                    }
                    match add_todo_from_editor() {
                        Ok(created) => should_refresh = created,
                        Err(error) => eprintln!("warning: {error}"),
                    }
                }
            }
            if should_refresh {
                refresh_everything(
                    &mut snapshot,
                    &mut title,
                    &mut next_refresh,
                    tray.as_ref(),
                    popover.as_ref(),
                    settings.as_ref(),
                    device_store.as_deref(),
                    accessory_server.as_ref(),
                );
            }
        }

        if matches!(
            event,
            Event::NewEvents(StartCause::ResumeTimeReached { .. })
        ) {
            refresh_everything(
                &mut snapshot,
                &mut title,
                &mut next_refresh,
                tray.as_ref(),
                popover.as_ref(),
                settings.as_ref(),
                device_store.as_deref(),
                accessory_server.as_ref(),
            );
            target.set_control_flow(ControlFlow::WaitUntil(next_refresh));
        }
    });
}

/// One collection feeds the menu bar, the popover, the settings window, and
/// every paired accessory, so they can never show different numbers.
#[allow(clippy::too_many_arguments)]
fn refresh_everything(
    snapshot: &mut TraySnapshot,
    title: &mut String,
    next_refresh: &mut Instant,
    tray: Option<&TrayIcon>,
    popover: Option<&PopoverUi>,
    settings: Option<&SettingsWindow>,
    device_store: Option<&DeviceStore>,
    accessory_server: Option<&network::AccessoryServer>,
) {
    let dashboard = DashboardSnapshot::collect();
    let monitoring = snapshot.monitoring.clone();
    *snapshot = TraySnapshot::from_dashboard(&dashboard, &paired(device_store));
    snapshot.monitoring = monitoring;
    snapshot
        .monitoring
        .retain_enabled(&dashboard.config.providers);
    *title = snapshot.monitoring.title(&snapshot.title);
    *next_refresh = Instant::now() + refresh_interval(snapshot.refresh_seconds);
    if let Some(tray) = tray {
        let _: () = tray.set_title(Some(title));
    }
    if let Some(popover) = popover {
        popover.render(snapshot);
    }
    if let Some(settings) = settings.filter(|window| window.is_visible()) {
        settings.render(&settings_snapshot(device_store));
    }
    if let Some(server) = accessory_server {
        server.update_dashboard(dashboard.json_string());
    }
}

fn paired(device_store: Option<&DeviceStore>) -> Vec<PairedDevice> {
    device_store.map(DeviceStore::devices).unwrap_or_default()
}

fn settings_snapshot(device_store: Option<&DeviceStore>) -> SettingsSnapshot {
    SettingsSnapshot {
        config: config::load_or_create().unwrap_or_default(),
        devices: paired(device_store),
        open_at_login: login_item::is_enabled(),
    }
}

/// Sharing is a switch, not a restart: turning it on publishes the service
/// immediately and turning it off withdraws it from the network at once.
fn apply_sharing(
    enabled: bool,
    device_store: Option<&Arc<DeviceStore>>,
    accessory_server: &mut Option<network::AccessoryServer>,
) {
    if !enabled {
        *accessory_server = None;
        return;
    }
    if accessory_server.is_some() {
        return;
    }
    let Some(devices) = device_store else {
        show_message(
            "Accessories Unavailable",
            "Gauge could not open its device registry or macOS Keychain.",
        );
        return;
    };
    let current = DashboardSnapshot::collect();
    match start_accessory_server(&current, Arc::clone(devices)) {
        Ok(server) => *accessory_server = Some(server),
        Err(error) => show_message("Could Not Share", &error),
    }
}

fn start_accessory_server(
    dashboard: &DashboardSnapshot,
    devices: Arc<DeviceStore>,
) -> Result<network::AccessoryServer, String> {
    let server = network::AccessoryServer::start(
        dashboard.config.accessories.port,
        &dashboard.config.accessories.display_name,
        devices,
        dashboard.json_string(),
    )?;
    eprintln!(
        "accessory sharing available on port {}",
        server.address().port()
    );
    Ok(server)
}

fn begin_pairing(
    devices: Arc<DeviceStore>,
    accessory_server: &mut Option<network::AccessoryServer>,
    request: PairingRequest,
) -> Result<(), String> {
    let config = config::set_accessories_enabled(true)?;
    if accessory_server.is_none() {
        let current = DashboardSnapshot::collect();
        *accessory_server = Some(start_accessory_server(&current, Arc::clone(&devices))?);
    }
    let port = config.accessories.port;
    std::thread::Builder::new()
        .name("gauge-pairing".into())
        .spawn(move || {
            let result = provisioning::pair_accessory(devices, request, port, Box::new(ask_which))
                .map(|device| device.name);
            post_action(AppAction::PairingFinished(result));
        })
        .map_err(|error| format!("could not start Bluetooth pairing: {error}"))?;
    Ok(())
}

/// Hands the discovered accessories to the main thread and waits for the
/// answer. This runs on the pairing worker, so blocking here is what keeps the
/// Bluetooth session alive while the user decides.
fn ask_which(found: Vec<DiscoveredAccessory>) -> Result<Option<String>, String> {
    let (reply, answer) = mpsc::channel();
    post_action(AppAction::ChooseAccessory(found, reply));
    answer
        .recv_timeout(Duration::from_secs(120))
        .map_err(|_| provisioning::CANCELLED.to_string())
}

struct TraySnapshot {
    monitoring: gauge::activity::Monitoring,
    title: String,
    meters: Vec<MeterGroup>,
    updated_at: u64,
    hooks_installed: bool,
    calendar_events: Vec<calendar::CalendarEvent>,
    calendar_error: Option<String>,
    calendar_enabled: bool,
    tasks_enabled: bool,
    todos: Vec<(usize, String, bool)>,
    /// One line naming what Gauge is sharing with, so a paired accessory is
    /// visible from the menu bar instead of only inside Settings.
    accessory_line: String,
    accessory_paired: bool,
    /// Any paired accessory read the dashboard within the last few minutes.
    accessory_connected: bool,
    refresh_seconds: u64,
}

impl TraySnapshot {
    fn from_dashboard(dashboard: &DashboardSnapshot, devices: &[PairedDevice]) -> Self {
        let todos = dashboard
            .config
            .todos
            .iter()
            .enumerate()
            .map(|(index, todo)| (index, todo.title.clone(), todo.completed))
            .take(5)
            .collect();
        Self {
            monitoring: gauge::activity::Monitoring::default(),
            title: tray_summary(&dashboard.usages),
            meters: meter_groups(&dashboard.usages),
            updated_at: dashboard.generated_at,
            hooks_installed: gauge::attention::hooks_installed(),
            calendar_events: dashboard.calendar_events.clone(),
            calendar_error: dashboard
                .settings_error
                .clone()
                .or_else(|| dashboard.calendar_error.clone()),
            calendar_enabled: dashboard.config.calendar.enabled,
            tasks_enabled: dashboard.config.tasks.enabled,
            todos,
            accessory_line: accessory_line(devices),
            accessory_paired: !devices.is_empty(),
            accessory_connected: devices.iter().any(|device| {
                device
                    .last_seen_at
                    .is_some_and(|seen| now_seconds().saturating_sub(seen) <= 180)
            }),
            refresh_seconds: dashboard.config.refresh_seconds,
        }
    }
}

/// Paired accessories are named here rather than counted: the point of the line
/// is recognising your own device, not knowing how many there are.
fn accessory_line(devices: &[PairedDevice]) -> String {
    match devices {
        [] => "Pair an accessory…".into(),
        [device] => truncate(&device.name, 26),
        devices => format!("{} accessories", devices.len()),
    }
}

/// The single switch vocabulary shared by the settings window and the store.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Switch {
    Codex,
    Claude,
    Cursor,
    Calendar,
    Tasks,
    Accessories,
    OpenAtLogin,
}

impl Switch {
    const ALL: [Switch; 7] = [
        Switch::Codex,
        Switch::Claude,
        Switch::Cursor,
        Switch::Calendar,
        Switch::Tasks,
        Switch::Accessories,
        Switch::OpenAtLogin,
    ];

    fn tag(self) -> isize {
        Self::ALL
            .iter()
            .position(|switch| *switch == self)
            .expect("every switch is listed in ALL") as isize
    }

    fn from_tag(tag: isize) -> Option<Self> {
        usize::try_from(tag)
            .ok()
            .and_then(|tag| Self::ALL.get(tag))
            .copied()
    }
}

enum AppAction {
    Activity,
    InstallHooks,
    TokensUpdated(gauge::activity::TokenStats),
    AttentionUpdated(gauge::attention::Attention),
    Refresh,
    Settings,
    OpenConfigurationFile,
    SetSwitch(Switch, bool),
    SetRefreshSeconds(u64),
    ForgetDevice(usize),
    PairAccessory,
    ChooseAccessory(Vec<DiscoveredAccessory>, mpsc::Sender<Option<String>>),
    Quit,
    ToggleTodo(usize),
    EditTodo(usize),
    DeleteTodo(usize),
    AddTodo,
    PairingFinished(Result<String, String>),
}

static ACTIONS: OnceLock<mpsc::Sender<AppAction>> = OnceLock::new();
static EVENT_LOOP_PROXY: OnceLock<winit::event_loop::EventLoopProxy<()>> = OnceLock::new();

fn post_action(action: AppAction) {
    let sent = ACTIONS
        .get()
        .is_some_and(|sender| sender.send(action).is_ok());
    if sent {
        if let Some(proxy) = EVENT_LOOP_PROXY.get() {
            let _ = proxy.send_event(());
        }
    }
}

#[cfg(target_os = "macos")]
use popover::PopoverUi;
#[cfg(target_os = "macos")]
use settings_window::{SettingsSnapshot, SettingsWindow};

fn truncate(text: &str, max_chars: usize) -> String {
    let mut characters = text.chars();
    let preview: String = characters.by_ref().take(max_chars).collect();
    if characters.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

fn format_time_range(starts_at: f64, ends_at: f64) -> String {
    let format = |timestamp| {
        chrono::DateTime::from_timestamp(timestamp as i64, 0)
            .map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| "—".into())
    };
    format!("{}–{}", format(starts_at), format(ends_at))
}

fn refresh_interval(seconds: u64) -> Duration {
    Duration::from_secs(seconds.clamp(30, 3_600))
}

/// The window covers everything Gauge exposes; this is the escape hatch for
/// the fields it deliberately does not, such as which calendars to include.
fn open_configuration_file() -> Result<(), String> {
    let path = config::path()?;
    if !path.exists() {
        let _ = config::load_or_create()?;
    }
    std::process::Command::new("open")
        .args(["-a", "TextEdit"])
        .arg(&path)
        .spawn()
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    Ok(())
}

/// A focused one-field editor keeps creating a task lighter than opening the
/// full settings file. It is native to Gauge, so it reliably appears above the
/// app's popover rather than behind it.
fn add_todo_from_editor() -> Result<bool, String> {
    let Some(title) = todo_editor("New To-do", "What needs doing?", "", "Add")? else {
        return Ok(false);
    };
    config::add_todo(title).map(|()| true)
}

/// Editing uses the same focused native editor as creation, prefilled with the
/// task's current title. It is kept outside the popover so the field always
/// receives keyboard focus.
fn edit_todo_from_editor(index: usize) -> Result<bool, String> {
    let config = config::load_or_create()?;
    let current_title = config
        .todos
        .get(index)
        .ok_or("that to-do no longer exists")?
        .title
        .clone();
    let Some(title) = todo_editor("Edit To-do", "Edit task", &current_title, "Save")? else {
        return Ok(false);
    };
    if title == current_title {
        return Ok(false);
    }
    config::update_todo(index, title).map(|()| true)
}

#[cfg(target_os = "macos")]
fn todo_editor(
    title: &str,
    message: &str,
    current_value: &str,
    confirm: &str,
) -> Result<Option<String>, String> {
    use objc2::MainThreadOnly;
    use objc2_app_kit::{NSAlert, NSTextField};
    use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

    let mtm = MainThreadMarker::new().ok_or("to-do editor must run on the macOS main thread")?;
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));
    alert.addButtonWithTitle(&NSString::from_str(confirm));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    let field = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(300.0, 24.0)),
    );
    field.setStringValue(&NSString::from_str(current_value));
    alert.setAccessoryView(Some(&field));
    if alert.runModal() != 1000 {
        return Ok(None);
    }
    let value = field.stringValue().to_string();
    let value = value.trim().to_owned();
    Ok((!value.is_empty()).then_some(value))
}

#[cfg(not(target_os = "macos"))]
fn todo_editor(_: &str, _: &str, _: &str, _: &str) -> Result<Option<String>, String> {
    Err("the to-do editor is only available on macOS".into())
}

#[cfg(target_os = "macos")]
fn pairing_editor() -> Result<Option<PairingRequest>, String> {
    use objc2::MainThreadOnly;
    use objc2_app_kit::{NSAlert, NSView};
    use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

    let mtm = MainThreadMarker::new().ok_or("pairing must run on the macOS main thread")?;
    let location_available = mac_wifi::location_access() == mac_wifi::LocationAccess::Authorized;
    let wifi_ssid = current_wifi_name().unwrap_or_default();
    let system_wifi_password = current_wifi_password(&wifi_ssid);
    if !wifi_ssid.is_empty() {
        if let Some(wifi_password) = system_wifi_password {
            return Ok(Some(PairingRequest {
                wifi_ssid,
                wifi_password,
            }));
        }
    }

    // Usually this editor is skipped completely. It is only the fallback when
    // CoreWLAN or Keychain cannot provide the current network configuration.
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("Wi-Fi for Accessory"));
    let information = if !location_available {
        "Gauge could not read this Mac's current network. Enter the Wi-Fi the accessory should use."
    } else if wifi_ssid.is_empty() {
        "This Mac is not connected over Wi-Fi. Enter the network the accessory should use."
    } else {
        "Gauge found the network name but could not read its password from Keychain."
    };
    alert.setInformativeText(&NSString::from_str(information));
    alert.addButtonWithTitle(&NSString::from_str("Continue"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));

    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(360.0, 102.0)),
    );
    let ssid_field = form_field(
        &view,
        "Wi-Fi network",
        "Network name",
        &wifi_ssid,
        65.0,
        mtm,
    );
    let password_field = secure_form_field(&view, "Wi-Fi password", "Password", "", 13.0, mtm);
    alert.setAccessoryView(Some(&view));

    if alert.runModal() != 1000 {
        return Ok(None);
    }
    Ok(Some(PairingRequest {
        wifi_ssid: ssid_field.stringValue().to_string().trim().to_owned(),
        wifi_password: password_field.stringValue().to_string(),
    }))
}

#[cfg(target_os = "macos")]
fn form_field(
    view: &objc2_app_kit::NSView,
    title: &str,
    placeholder: &str,
    value: &str,
    y: f64,
    mtm: objc2_foundation::MainThreadMarker,
) -> objc2::rc::Retained<objc2_app_kit::NSTextField> {
    use objc2::MainThreadOnly;
    use objc2_app_kit::NSTextField;
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    label.setFrame(NSRect::new(
        NSPoint::new(0.0, y + 23.0),
        NSSize::new(150.0, 17.0),
    ));
    view.addSubview(&label);
    let field = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, y), NSSize::new(360.0, 24.0)),
    );
    field.setPlaceholderString(Some(&NSString::from_str(placeholder)));
    field.setStringValue(&NSString::from_str(value));
    view.addSubview(&field);
    field
}

#[cfg(target_os = "macos")]
fn secure_form_field(
    view: &objc2_app_kit::NSView,
    title: &str,
    placeholder: &str,
    value: &str,
    y: f64,
    mtm: objc2_foundation::MainThreadMarker,
) -> objc2::rc::Retained<objc2_app_kit::NSSecureTextField> {
    use objc2::MainThreadOnly;
    use objc2_app_kit::{NSSecureTextField, NSTextField};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    label.setFrame(NSRect::new(
        NSPoint::new(0.0, y + 23.0),
        NSSize::new(150.0, 17.0),
    ));
    view.addSubview(&label);
    let field = NSSecureTextField::initWithFrame(
        NSSecureTextField::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, y), NSSize::new(360.0, 24.0)),
    );
    field.setPlaceholderString(Some(&NSString::from_str(placeholder)));
    field.setStringValue(&NSString::from_str(value));
    view.addSubview(&field);
    field
}

#[cfg(not(target_os = "macos"))]
fn pairing_editor() -> Result<Option<PairingRequest>, String> {
    Err("accessory pairing is only available on macOS".into())
}

/// Name what you are about to trust. Gauge lists every accessory it can hear
/// and connects to exactly the one that is chosen — never to whatever happened
/// to answer first.
#[cfg(target_os = "macos")]
fn accessory_picker(found: &[DiscoveredAccessory]) -> Option<String> {
    use objc2::MainThreadOnly;
    use objc2_app_kit::{NSAlert, NSButton, NSButtonType, NSFont, NSView};
    use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

    const ROW: f64 = 24.0;
    const WIDTH: f64 = 320.0;

    let mtm = MainThreadMarker::new()?;
    let found: Vec<_> = found.iter().take(6).collect();
    if found.is_empty() {
        return None;
    }

    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("Pair an Accessory"));
    alert.setInformativeText(&NSString::from_str(if found.len() == 1 {
        "Gauge found one accessory in pairing mode."
    } else {
        "Choose the accessory to pair with this Mac."
    }));
    alert.addButtonWithTitle(&NSString::from_str("Pair"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));

    let height = found.len() as f64 * ROW;
    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, height)),
    );
    let mut choices = Vec::new();
    for (index, accessory) in found.iter().enumerate() {
        let button = NSButton::initWithFrame(
            NSButton::alloc(mtm),
            NSRect::new(
                NSPoint::new(0.0, height - (index as f64 + 1.0) * ROW),
                NSSize::new(WIDTH, ROW),
            ),
        );
        button.setButtonType(NSButtonType::Radio);
        button.setTitle(&NSString::from_str(&accessory.name));
        button.setFont(Some(&NSFont::systemFontOfSize_weight(13.0, 0.0)));
        button.setState(if index == 0 { 1 } else { 0 });
        view.addSubview(&button);
        choices.push(button);
    }
    alert.setAccessoryView(Some(&view));

    if alert.runModal() != 1000 {
        return None;
    }
    choices
        .iter()
        .position(|button| button.state() != 0)
        .and_then(|index| found.get(index))
        .map(|accessory| accessory.id.clone())
}

#[cfg(not(target_os = "macos"))]
fn accessory_picker(_: &[DiscoveredAccessory]) -> Option<String> {
    None
}

/// Used only where the next step removes something the user set up.
#[cfg(target_os = "macos")]
fn confirm(title: &str, message: &str, action: &str) -> bool {
    use objc2_app_kit::NSAlert;
    use objc2_foundation::{MainThreadMarker, NSString};

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));
    alert.addButtonWithTitle(&NSString::from_str(action));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    alert.runModal() == 1000
}

#[cfg(not(target_os = "macos"))]
fn confirm(_: &str, _: &str, _: &str) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn show_message(title: &str, message: &str) {
    use objc2_app_kit::NSAlert;
    use objc2_foundation::{MainThreadMarker, NSString};

    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("{title}: {message}");
        return;
    };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));
    alert.addButtonWithTitle(&NSString::from_str("OK"));
    alert.runModal();
}

#[cfg(not(target_os = "macos"))]
fn show_message(title: &str, message: &str) {
    eprintln!("{title}: {message}");
}

#[cfg(target_os = "macos")]
fn current_wifi_name() -> Option<String> {
    mac_wifi::current_network_name().or_else(current_wifi_name_from_networksetup)
}

#[cfg(target_os = "macos")]
fn current_wifi_name_from_networksetup() -> Option<String> {
    let hardware = std::process::Command::new("/usr/sbin/networksetup")
        .arg("-listallhardwareports")
        .output()
        .ok()?;
    let body = String::from_utf8(hardware.stdout).ok()?;
    let mut wifi_port = false;
    let mut interface = None;
    for line in body.lines() {
        if let Some(port) = line.strip_prefix("Hardware Port: ") {
            wifi_port = matches!(port, "Wi-Fi" | "AirPort");
        } else if wifi_port {
            if let Some(device) = line.strip_prefix("Device: ") {
                interface = Some(device.trim().to_owned());
                break;
            }
        }
    }
    let interface = interface?;
    let current = std::process::Command::new("/usr/sbin/networksetup")
        .args(["-getairportnetwork", &interface])
        .output()
        .ok()?;
    let body = String::from_utf8(current.stdout).ok()?;
    let (_, name) = body.trim().split_once(": ")?;
    let name = name.trim();
    (!name.is_empty()
        && !name.eq_ignore_ascii_case("You are not associated with an AirPort network."))
    .then(|| name.to_owned())
}

/// Read the current network's saved password from the macOS Keychain. The
/// `security` utility uses the same Keychain authorization path as other Mac
/// apps, so the user remains in control and may be asked to allow access.
/// Never log the command output: stdout contains the password on success.
fn current_wifi_password(ssid: &str) -> Option<String> {
    if ssid.is_empty() {
        return None;
    }
    let output = std::process::Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-D",
            "AirPort network password",
            "-a",
            ssid,
            "-w",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let password = String::from_utf8(output.stdout).ok()?;
    Some(password.trim_end_matches(['\r', '\n']).to_owned())
}

fn start_activity_monitor() {
    std::thread::spawn(|| {
        let mut reader = gauge::activity::UsageReader::default();
        loop {
            post_action(AppAction::TokensUpdated(reader.collect(&providers())));
            std::thread::sleep(Duration::from_secs(15));
        }
    });
    std::thread::spawn(|| {
        let mut previous = String::new();
        loop {
            let attention = gauge::attention::collect(&providers());
            let current = serde_json::to_string(&attention).unwrap_or_default();
            if current != previous {
                previous = current;
                post_action(AppAction::AttentionUpdated(attention));
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str) -> PairedDevice {
        PairedDevice {
            id: name.to_ascii_lowercase(),
            name: name.into(),
            kind: "display".into(),
            firmware_version: None,
            protocol_version: 1,
            capabilities: Vec::new(),
            paired_at: 0,
            last_seen_at: None,
        }
    }

    #[test]
    fn the_popover_names_one_accessory_and_counts_several() {
        assert_eq!(accessory_line(&[]), "Pair an accessory…");
        assert_eq!(accessory_line(&[device("Bunty-4F2A1C")]), "Bunty-4F2A1C");
        assert_eq!(
            accessory_line(&[device("Bunty-4F2A1C"), device("Desk")]),
            "2 accessories"
        );
    }

    #[test]
    fn every_switch_survives_the_round_trip_through_a_control_tag() {
        for switch in Switch::ALL {
            assert!(Switch::from_tag(switch.tag()) == Some(switch));
        }
        assert!(Switch::from_tag(-1).is_none());
        assert!(Switch::from_tag(Switch::ALL.len() as isize).is_none());
    }
}
