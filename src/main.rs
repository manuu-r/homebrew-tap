mod network;

use gauge::{
    calendar, config,
    dashboard::DashboardSnapshot,
    devices::DeviceStore,
    fetch_all, now_seconds,
    provisioning::{self, PairingRequest},
    quota_groups, summary, tray_summary, QuotaGroup, Usage,
};
use std::{
    env, process,
    sync::{mpsc, Arc, OnceLock},
    time::{Duration, Instant},
};
use tray_icon::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
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
    gauge --settings       Open the tray settings file\n\
    gauge --version        Print the version";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Cli,
    Json,
    Tray,
    Settings,
}

fn main() {
    if let Err(error) = run(env::args().skip(1)) {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn run(args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut args = args.peekable();
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
        Mode::Settings => open_settings(),
        Mode::Json => {
            let (usages, errors) = fetch_all();
            println!("{}", quota_json(&usages, &errors));
            Ok(())
        }
        Mode::Cli => {
            let (usages, errors) = fetch_all();

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
        return Err("choose only one of --json, --tray, or --settings".into());
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
    let dashboard = DashboardSnapshot::collect();
    let mut snapshot = TraySnapshot::from_dashboard(&dashboard);
    let mut title = snapshot.title.clone();
    let mut next_refresh = Instant::now() + refresh_interval(snapshot.refresh_seconds);
    let mut tray: Option<TrayIcon> = None;
    let events = TrayIconEvent::receiver();
    let (actions_tx, actions_rx) = mpsc::channel();
    let _ = POPOVER_ACTIONS.set(actions_tx);
    let mut popover: Option<PopoverUi> = None;
    let mut pairing_in_progress = false;
    let device_store = match DeviceStore::open() {
        Ok(devices) => Some(Arc::new(devices)),
        Err(error) => {
            eprintln!("accessory pairing unavailable: {error}");
            None
        }
    };
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
                PopoverAction::Quit => process::exit(0),
                PopoverAction::Settings => {
                    if let Err(error) = open_settings() {
                        eprintln!("warning: {error}");
                    }
                }
                PopoverAction::PairAccessory => {
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
                PopoverAction::PairingFinished(result) => {
                    pairing_in_progress = false;
                    match result {
                        Ok(name) => eprintln!("{name} paired and connected to Wi-Fi"),
                        Err(error) => show_message("Pairing Failed", &error),
                    }
                    should_refresh = true;
                }
                PopoverAction::Refresh => should_refresh = true,
                PopoverAction::ToggleTodo(index) => match config::toggle_todo(index) {
                    Ok(()) => should_refresh = true,
                    Err(error) => eprintln!("warning: {error}"),
                },
                PopoverAction::EditTodo(index) => {
                    if let Some(popover) = &popover {
                        popover.dismiss();
                    }
                    match edit_todo_from_editor(index) {
                        Ok(changed) => should_refresh = changed,
                        Err(error) => eprintln!("warning: {error}"),
                    }
                }
                PopoverAction::DeleteTodo(index) => match config::delete_todo(index) {
                    Ok(()) => should_refresh = true,
                    Err(error) => eprintln!("warning: {error}"),
                },
                PopoverAction::AddTodo => {
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
                refresh_popover(
                    &mut snapshot,
                    &mut title,
                    &mut next_refresh,
                    tray.as_ref(),
                    popover.as_ref(),
                    accessory_server.as_ref(),
                );
            }
        }

        if matches!(
            event,
            Event::NewEvents(StartCause::ResumeTimeReached { .. })
        ) {
            refresh_popover(
                &mut snapshot,
                &mut title,
                &mut next_refresh,
                tray.as_ref(),
                popover.as_ref(),
                accessory_server.as_ref(),
            );
            target.set_control_flow(ControlFlow::WaitUntil(next_refresh));
        }
    });
}

fn refresh_popover(
    snapshot: &mut TraySnapshot,
    title: &mut String,
    next_refresh: &mut Instant,
    tray: Option<&TrayIcon>,
    popover: Option<&PopoverUi>,
    accessory_server: Option<&network::AccessoryServer>,
) {
    let dashboard = DashboardSnapshot::collect();
    *snapshot = TraySnapshot::from_dashboard(&dashboard);
    *title = snapshot.title.clone();
    *next_refresh = Instant::now() + refresh_interval(snapshot.refresh_seconds);
    if let Some(tray) = tray {
        let _: () = tray.set_title(Some(title));
    }
    if let Some(popover) = popover {
        popover.render(snapshot);
    }
    if let Some(server) = accessory_server {
        server.update_dashboard(dashboard.json_string());
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
            let result =
                provisioning::pair_accessory(devices, request, port).map(|device| device.name);
            post_action(PopoverAction::PairingFinished(result));
        })
        .map_err(|error| format!("could not start Bluetooth pairing: {error}"))?;
    Ok(())
}

struct TraySnapshot {
    title: String,
    quota_groups: Vec<QuotaGroup>,
    calendar_events: Vec<calendar::CalendarEvent>,
    calendar_error: Option<String>,
    calendar_enabled: bool,
    todos: Vec<(usize, String, bool)>,
    refresh_seconds: u64,
}

impl TraySnapshot {
    fn from_dashboard(dashboard: &DashboardSnapshot) -> Self {
        let todos = dashboard
            .config
            .todos
            .iter()
            .enumerate()
            .map(|(index, todo)| (index, todo.title.clone(), todo.completed))
            .take(5)
            .collect();
        Self {
            title: tray_summary(&dashboard.usages),
            quota_groups: quota_groups(&dashboard.usages),
            calendar_events: dashboard.calendar_events.clone(),
            calendar_error: dashboard
                .settings_error
                .clone()
                .or_else(|| dashboard.calendar_error.clone()),
            calendar_enabled: dashboard.config.calendar.enabled,
            todos,
            refresh_seconds: dashboard.config.refresh_seconds,
        }
    }
}

enum PopoverAction {
    Refresh,
    Settings,
    PairAccessory,
    Quit,
    ToggleTodo(usize),
    EditTodo(usize),
    DeleteTodo(usize),
    AddTodo,
    PairingFinished(Result<String, String>),
}

static POPOVER_ACTIONS: OnceLock<mpsc::Sender<PopoverAction>> = OnceLock::new();
static EVENT_LOOP_PROXY: OnceLock<winit::event_loop::EventLoopProxy<()>> = OnceLock::new();

fn post_action(action: PopoverAction) {
    let sent = POPOVER_ACTIONS
        .get()
        .is_some_and(|sender| sender.send(action).is_ok());
    if sent {
        if let Some(proxy) = EVENT_LOOP_PROXY.get() {
            let _ = proxy.send_event(());
        }
    }
}

#[cfg(target_os = "macos")]
mod popover_ui {
    use super::{format_time_range, post_action, scratched, truncate, PopoverAction, TraySnapshot};
    use objc2::{define_class, msg_send, rc::Retained, runtime::AnyObject, sel, MainThreadOnly};

    use objc2_app_kit::{
        NSButton, NSColor, NSControl, NSFont, NSPopover, NSPopoverBehavior, NSTextAlignment,
        NSTextField, NSView, NSViewController,
    };
    use objc2_foundation::{
        MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRectEdge, NSSize, NSString,
    };
    use tray_icon::TrayIcon;

    const WIDTH: f64 = 330.0;
    const LEFT: f64 = 12.0;
    const CONTENT_WIDTH: f64 = WIDTH - (LEFT * 2.0);
    const DIVIDER: &str = "────────────────────────────────────────";
    const EDIT_TAG_BASE: isize = -1_000;
    const DELETE_TAG_BASE: isize = -2_000;

    define_class!(
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        struct PopoverTarget;

        unsafe impl NSObjectProtocol for PopoverTarget {}

        impl PopoverTarget {
            #[unsafe(method(refresh:))]
            fn refresh(&self, _: &AnyObject) {
                send(PopoverAction::Refresh);
            }

            #[unsafe(method(settings:))]
            fn settings(&self, _: &AnyObject) {
                send(PopoverAction::Settings);
            }

            #[unsafe(method(pairAccessory:))]
            fn pair_accessory(&self, _: &AnyObject) {
                send(PopoverAction::PairAccessory);
            }

            #[unsafe(method(quit:))]
            fn quit(&self, _: &AnyObject) {
                send(PopoverAction::Quit);
            }

            #[unsafe(method(todo:))]
            fn todo(&self, sender: &NSControl) {
                let tag = sender.tag();
                if tag >= 0 {
                    send(PopoverAction::ToggleTodo(tag as usize));
                }
            }

            #[unsafe(method(editTodo:))]
            fn edit_todo(&self, sender: &NSControl) {
                if let Some(index) = task_index(sender.tag(), EDIT_TAG_BASE) {
                    send(PopoverAction::EditTodo(index));
                }
            }

            #[unsafe(method(deleteTodo:))]
            fn delete_todo(&self, sender: &NSControl) {
                if let Some(index) = task_index(sender.tag(), DELETE_TAG_BASE) {
                    send(PopoverAction::DeleteTodo(index));
                }
            }

            #[unsafe(method(addTodo:))]
            fn add_todo(&self, _: &AnyObject) {
                send(PopoverAction::AddTodo);
            }

        }
    );

    fn send(action: PopoverAction) {
        post_action(action);
    }

    impl PopoverTarget {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm);
            unsafe { msg_send![this, init] }
        }
    }

    fn task_index(tag: isize, base: isize) -> Option<usize> {
        (tag <= base).then_some((base - tag) as usize)
    }

    pub struct PopoverUi {
        popover: Retained<NSPopover>,
        // NSControl keeps its target weak, so own it alongside the popover.
        _target: Retained<PopoverTarget>,
    }

    impl PopoverUi {
        pub fn new(snapshot: &TraySnapshot) -> Self {
            let mtm = MainThreadMarker::new().expect("popover must be built on the main thread");
            let popover = NSPopover::new(mtm);
            // This lets a click update the content without AppKit dismissing the
            // dashboard, while the menu-bar icon remains the explicit toggle.
            popover.setBehavior(NSPopoverBehavior::ApplicationDefined);
            let target = PopoverTarget::new(mtm);
            let ui = Self {
                popover,
                _target: target,
            };
            ui.render(snapshot);
            ui
        }

        pub fn toggle(&self, tray: &TrayIcon, snapshot: &TraySnapshot) {
            if self.popover.isShown() {
                self.popover.close();
                return;
            }
            self.render(snapshot);
            let mtm = MainThreadMarker::new().expect("tray clicks arrive on the main thread");
            let status_item = tray.ns_status_item().expect("macOS tray status item");
            let button = status_item.button(mtm).expect("macOS tray status button");
            self.popover.showRelativeToRect_ofView_preferredEdge(
                button.bounds(),
                &button,
                NSRectEdge::MinY,
            );
        }

        pub fn dismiss(&self) {
            if self.popover.isShown() {
                // `close` normally animates. The external native editor must
                // not be created until the popover has left the screen.
                self.popover.setAnimates(false);
                self.popover.close();
                self.popover.setAnimates(true);
            }
        }

        pub fn render(&self, snapshot: &TraySnapshot) {
            let mtm = MainThreadMarker::new().expect("popover must be rendered on the main thread");
            // Replacing visible popover content normally cross-fades the old
            // and new layouts, which looked like a flickering expand effect.
            let suppress_transition = self.popover.isShown();
            if suppress_transition {
                self.popover.setAnimates(false);
            }
            let height = panel_height(snapshot);
            let view = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, height)),
            );
            let controller = NSViewController::new(mtm);
            controller.setView(&view);
            populate(&view, &self._target, snapshot, height, mtm);
            self.popover.setContentViewController(Some(&controller));
            // Setting a view controller can reset the popover to its previous
            // content size. Apply the final size afterwards so quota rows are
            // never clipped.
            self.popover.setContentSize(NSSize::new(WIDTH, height));
            if suppress_transition {
                self.popover.setAnimates(true);
            }
        }
    }

    fn panel_height(snapshot: &TraySnapshot) -> f64 {
        let quota_lines = if snapshot.quota_groups.is_empty() {
            1
        } else {
            snapshot
                .quota_groups
                .iter()
                .map(|group| 1 + group.hourly_rows.len() + group.weekly_rows.len())
                .sum::<usize>()
        };

        // Keep this in exact pixel units matching `populate`. The previous
        // approximate row formula stopped at "+ Add to-do…", leaving the
        // pairing divider and button outside the content view.
        let top_inset = 28.0;
        let actions_and_divider = 25.0 + 21.0;
        let quota = quota_lines as f64 * 28.0;
        let calendar = if snapshot.calendar_enabled { 39.0 } else { 0.0 };
        let todos_header = 19.0 + 20.0;
        let todos = snapshot.todos.len() as f64 * 23.0;
        let add_todo_and_pairing = 23.0 + 20.0;
        let bottom_inset = 12.0;

        top_inset
            + actions_and_divider
            + quota
            + calendar
            + todos_header
            + todos
            + add_todo_and_pairing
            + bottom_inset
    }

    fn populate(
        view: &NSView,
        target: &PopoverTarget,
        snapshot: &TraySnapshot,
        height: f64,
        mtm: MainThreadMarker,
    ) {
        let mut y = height - 28.0;
        button(
            view,
            target,
            "Refresh",
            12.0,
            y,
            64.0,
            sel!(refresh:),
            -1,
            mtm,
        );
        button(
            view,
            target,
            "Settings…",
            85.0,
            y,
            78.0,
            sel!(settings:),
            -1,
            mtm,
        );
        button(
            view,
            target,
            "Quit",
            WIDTH - 56.0,
            y,
            40.0,
            sel!(quit:),
            -1,
            mtm,
        );
        y -= 25.0;

        label(view, DIVIDER, y, 10.0, mtm);
        y -= 21.0;
        if snapshot.quota_groups.is_empty() {
            quota_label(view, "Quota unavailable", y, 12.0, mtm);
            y -= 28.0;
        }
        for group in &snapshot.quota_groups {
            quota_label(view, group.provider, y, 12.0, mtm);
            y -= 28.0;
            for row in group.hourly_rows.iter().chain(group.weekly_rows.iter()) {
                quota_label(view, row, y, 11.0, mtm);
                y -= 28.0;
            }
        }

        if snapshot.calendar_enabled {
            label(view, DIVIDER, y, 10.0, mtm);
            y -= 19.0;
            let calendar_line = if let Some(error) = &snapshot.calendar_error {
                format!("Calendar: {}", truncate(error, 30))
            } else if let Some(event) = snapshot.calendar_events.first() {
                let when = if event.all_day {
                    "All day".into()
                } else {
                    format_time_range(event.starts_at, event.ends_at)
                };
                format!("Next: {}  {}", truncate(&event.title, 22), when)
            } else {
                "Next: No upcoming events".into()
            };
            label(view, &calendar_line, y, 12.0, mtm);
            y -= 20.0;
        }

        label(view, DIVIDER, y, 10.0, mtm);
        y -= 19.0;
        label(view, "Today", y, 12.0, mtm);
        y -= 20.0;
        for (index, title, completed) in &snapshot.todos {
            let text = if *completed {
                format!("☑  {}", scratched(&truncate(title, 25)))
            } else {
                format!("☐  {}", truncate(title, 25))
            };
            button(
                view,
                target,
                &text,
                LEFT,
                y,
                CONTENT_WIDTH - 78.0,
                sel!(todo:),
                *index as isize,
                mtm,
            );
            button(
                view,
                target,
                "Edit",
                LEFT + CONTENT_WIDTH - 72.0,
                y + 2.0,
                36.0,
                sel!(editTodo:),
                EDIT_TAG_BASE - *index as isize,
                mtm,
            );
            button(
                view,
                target,
                "×",
                LEFT + CONTENT_WIDTH - 28.0,
                y + 2.0,
                20.0,
                sel!(deleteTodo:),
                DELETE_TAG_BASE - *index as isize,
                mtm,
            );
            y -= 23.0;
        }
        button(
            view,
            target,
            "+ Add to-do…",
            LEFT,
            y,
            110.0,
            sel!(addTodo:),
            -1,
            mtm,
        );
        y -= 23.0;
        label(view, DIVIDER, y, 10.0, mtm);
        y -= 20.0;
        button(
            view,
            target,
            "Pair Accessory…",
            LEFT,
            y,
            120.0,
            sel!(pairAccessory:),
            -1,
            mtm,
        );
    }

    fn label(view: &NSView, text: &str, y: f64, size: f64, mtm: MainThreadMarker) {
        let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
        field.setFrame(NSRect::new(
            NSPoint::new(LEFT, y),
            NSSize::new(CONTENT_WIDTH, 17.0),
        ));
        field.setFont(Some(&NSFont::systemFontOfSize_weight(size, 0.23)));
        field.setAlignment(NSTextAlignment::Left);
        if text.starts_with('─') {
            let color = NSColor::tertiaryLabelColor();
            field.setTextColor(Some(&color));
        } else {
            let color = NSColor::labelColor();
            field.setTextColor(Some(&color));
        }
        view.addSubview(&field);
    }

    fn quota_label(view: &NSView, text: &str, y: f64, size: f64, mtm: MainThreadMarker) {
        let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
        field.setFrame(NSRect::new(
            NSPoint::new(LEFT, y),
            NSSize::new(CONTENT_WIDTH, 18.0),
        ));
        field.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(size, 0.23)));
        field.setAlignment(NSTextAlignment::Left);
        let color = NSColor::labelColor();
        field.setTextColor(Some(&color));
        view.addSubview(&field);
    }

    #[allow(clippy::too_many_arguments)]
    fn button(
        view: &NSView,
        target: &PopoverTarget,
        title: &str,
        x: f64,
        y: f64,
        width: f64,
        action: objc2::runtime::Sel,
        tag: isize,
        mtm: MainThreadMarker,
    ) {
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(title),
                Some(target),
                Some(action),
                mtm,
            )
        };
        button.setBordered(false);
        button.setAlignment(NSTextAlignment::Left);
        let color = NSColor::labelColor();
        button.setContentTintColor(Some(&color));
        button.setFont(Some(&NSFont::systemFontOfSize_weight(
            if tag >= 0 { 14.0 } else { 12.0 },
            0.23,
        )));
        button.setTag(tag);
        button.setFrame(NSRect::new(
            NSPoint::new(x, y),
            NSSize::new(width, if tag >= 0 { 22.0 } else { 18.0 }),
        ));
        view.addSubview(&button);
    }
}

#[cfg(target_os = "macos")]
use popover_ui::PopoverUi;

fn truncate(text: &str, max_chars: usize) -> String {
    let mut characters = text.chars();
    let preview: String = characters.by_ref().take(max_chars).collect();
    if characters.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

#[allow(dead_code)]
fn tray_menu(snapshot: &TraySnapshot) -> Menu {
    let menu = Menu::new();
    menu.append_items(&[
        &MenuItem::with_id("refresh", "Refresh now", true, None),
        &MenuItem::with_id("settings", "Settings…", true, None),
        &MenuItem::with_id("quit", "Quit", true, None),
    ])
    .expect("failed to add tray actions");
    menu.append(&PredefinedMenuItem::separator())
        .expect("failed to add menu separator");

    let quota_menu = Submenu::with_id(
        "quota_details",
        format!("Gauge                     {}", snapshot.title),
        true,
    );
    append_quota_details(&quota_menu, &snapshot.quota_groups);
    menu.append(&quota_menu)
        .expect("failed to add quota summary");
    append_calendar_section(&menu, snapshot);
    append_todos_section(&menu, snapshot);
    use_fixed_width_font(&menu);
    menu
}

/// The expanded view intentionally keeps the original compact quota grid from
/// Gauge, while leaving the default menu as an at-a-glance dashboard.
#[allow(dead_code)]
fn append_quota_details(menu: &Submenu, groups: &[QuotaGroup]) {
    if groups.is_empty() {
        menu.append(&MenuItem::new("Quota unavailable", true, None))
            .expect("failed to add quota status");
        return;
    }
    for (index, group) in groups.iter().enumerate() {
        if index > 0 {
            menu.append(&PredefinedMenuItem::separator())
                .expect("failed to separate providers");
        }
        menu.append(&MenuItem::new(group.provider, true, None))
            .expect("failed to add provider heading");
        for row in &group.hourly_rows {
            menu.append(&MenuItem::new(row, true, None))
                .expect("failed to add quota row");
        }
        if !group.hourly_rows.is_empty() && !group.weekly_rows.is_empty() {
            menu.append(&PredefinedMenuItem::separator())
                .expect("failed to separate quota windows");
        }
        for row in &group.weekly_rows {
            menu.append(&MenuItem::new(row, true, None))
                .expect("failed to add quota row");
        }
    }
}

#[allow(dead_code)]
fn append_calendar_section(menu: &Menu, snapshot: &TraySnapshot) {
    if !snapshot.calendar_enabled {
        return;
    }
    menu.append(&PredefinedMenuItem::separator())
        .expect("failed to add calendar separator");
    if let Some(error) = &snapshot.calendar_error {
        menu.append(&MenuItem::new(format!("Calendar: {error}"), true, None))
            .expect("failed to add calendar status");
    }
    if snapshot.calendar_events.is_empty() && snapshot.calendar_error.is_none() {
        menu.append(&MenuItem::new(
            "Next: No upcoming calendar events",
            true,
            None,
        ))
        .expect("failed to add empty-calendar status");
    }
    for (index, event) in snapshot.calendar_events.iter().enumerate() {
        let when = if event.all_day {
            "All day".to_string()
        } else {
            format_time_range(event.starts_at, event.ends_at)
        };
        menu.append(&MenuItem::new(
            format!(
                "{}: {:<22} {when}",
                if index == 0 { "Next" } else { "Then" },
                event.title
            ),
            true,
            None,
        ))
        .expect("failed to add calendar event");
    }
}

#[allow(dead_code)]
fn append_todos_section(menu: &Menu, snapshot: &TraySnapshot) {
    menu.append(&PredefinedMenuItem::separator())
        .expect("failed to add to-do separator");
    menu.append(&MenuItem::new("Today", true, None))
        .expect("failed to add to-do heading");
    for (index, title, completed) in &snapshot.todos {
        menu.append(&MenuItem::with_id(
            format!("todo:{index}"),
            format!(
                "{}  {}",
                if *completed { "☑" } else { "☐" },
                if *completed {
                    scratched(title)
                } else {
                    title.clone()
                }
            ),
            true,
            None,
        ))
        .expect("failed to add to-do");
    }
    menu.append(&MenuItem::with_id("add_todo", "+ Add to-do…", true, None))
        .expect("failed to add to-do action");
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

/// macOS native menus accept plain strings, not attributed text. Combining the
/// long-stroke mark gives completed tasks an unambiguous scratched-through
/// appearance while preserving a normal menu item and click target.
fn scratched(title: &str) -> String {
    title
        .chars()
        .flat_map(|character| [character, '\u{0336}'])
        .collect()
}

fn refresh_interval(seconds: u64) -> Duration {
    Duration::from_secs(seconds.clamp(30, 3_600))
}

fn open_settings() -> Result<(), String> {
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

/// A text-only native menu uses a proportional font by default. Our quota
/// rows are a grid, so use the system's monospaced face rather than trying to
/// imitate columns with special Unicode spaces.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
fn use_fixed_width_font(menu: &Menu) {
    use objc2_app_kit::{NSFont, NSMenu};
    use tray_icon::menu::ContextMenu;

    // Gauge builds its menu on the macOS main thread, as required by AppKit.
    let native_menu = unsafe { &*menu.ns_menu().cast::<NSMenu>() };
    let font = NSFont::monospacedSystemFontOfSize_weight(12.0, 0.0);
    unsafe { native_menu.setFont(Some(&font)) };
}

#[cfg(not(target_os = "macos"))]
#[allow(dead_code)]
fn use_fixed_width_font(_: &Menu) {}
