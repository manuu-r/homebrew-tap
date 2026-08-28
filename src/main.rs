mod network;

use gauge::{
    calendar, config, fetch_all, now_seconds, quota_groups,
    stocks::{self, StockQuote},
    summary, tray_summary, QuotaGroup, Usage,
};
use std::{
    env, process,
    sync::{mpsc, OnceLock},
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

const HELP: &str = "Gauge\n\
    \n\
    gauge                  Show remaining agent quota\n\
    gauge --json           Print the same data as JSON\n\
    gauge --tray           Keep it in the menu bar\n\
    gauge --settings       Open the tray settings file\n\
    gauge --wifi           Start the Wi-Fi HTTP API (default 0.0.0.0:8080)\n\
    gauge --ble            Start the BLE-style UDP API (default 0.0.0.0:8081)\n\
    gauge --bind HOST      Bind network services to host/IP\n\
    gauge --port PORT      Bind port (defaults per protocol)\n\
    gauge --token TOKEN    API token for network services\n\
    gauge --version        Print the version";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Cli,
    Json,
    Tray,
    Settings,
    Wifi,
    Ble,
}

fn main() {
    if let Err(error) = run(env::args().skip(1)) {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn run(args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut mode = Mode::Cli;
    let mut bind = None;
    let mut port = None;
    let mut token = None;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
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
            "--wifi" => set_mode(&mut mode, Mode::Wifi)?,
            "--ble" => set_mode(&mut mode, Mode::Ble)?,
            "--bind" => bind = Some(next_value(&mut args, "--bind")?),
            "--port" => {
                let value = next_value(&mut args, "--port")?;
                port = Some(
                    value
                        .parse::<u16>()
                        .map_err(|error| format!("invalid --port value '{value}': {error}"))?,
                );
            }
            "--token" | "--auth-token" => token = Some(next_value(&mut args, "--token")?),
            _ => return Err(format!("unknown argument: {arg}\n\n{HELP}")),
        }
    }

    let has_network_options = bind.is_some() || port.is_some() || token.is_some();
    if !matches!(mode, Mode::Wifi | Mode::Ble) && has_network_options {
        return Err("--bind, --port, and --token require --wifi or --ble".into());
    }

    match mode {
        Mode::Wifi | Mode::Ble => {
            let bind = bind
                .or_else(|| env::var("GAUGE_BIND").ok())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "0.0.0.0".into());
            let token = token
                .or_else(|| env::var("GAUGE_API_TOKEN").ok())
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .ok_or("network mode requires GAUGE_API_TOKEN or --token")?;

            match mode {
                Mode::Wifi => network::run_wifi_server(bind, port.unwrap_or(8080), token),
                Mode::Ble => network::run_udp_server(bind, port.unwrap_or(8081), token),
                _ => unreachable!(),
            }
        }
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

fn set_mode(mode: &mut Mode, requested: Mode) -> Result<(), String> {
    if *mode != Mode::Cli && *mode != requested {
        return Err("choose only one of --json, --tray, --settings, --wifi, or --ble".into());
    }
    *mode = requested;
    Ok(())
}

fn next_value(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    args.next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{option} requires a value"))
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
/// popover. Unlike an NSMenu, this stays open while a section expands in place.
#[allow(deprecated)]
fn run_tray() {
    let mut snapshot = tray_snapshot();
    let mut title = snapshot.title.clone();
    let mut next_refresh = Instant::now() + refresh_interval(snapshot.refresh_seconds);
    let mut tray: Option<TrayIcon> = None;
    let events = TrayIconEvent::receiver();
    let (actions_tx, actions_rx) = mpsc::channel();
    let _ = POPOVER_ACTIONS.set(actions_tx);
    let mut popover: Option<PopoverUi> = None;
    let mut quota_expanded = false;

    // Accessory keeps Gauge out of the Dock and the app switcher, which is what
    // a menu bar utility should do.
    let event_loop = EventLoop::builder()
        .with_activation_policy(ActivationPolicy::Accessory)
        .build()
        .expect("failed to start event loop");
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
                    popover.toggle(tray, &snapshot, quota_expanded);
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
                PopoverAction::Refresh => should_refresh = true,
                PopoverAction::ToggleQuota => {
                    quota_expanded = !quota_expanded;
                    if let Some(popover) = &popover {
                        popover.render(&snapshot, quota_expanded);
                    }
                }
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
                    quota_expanded,
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
                quota_expanded,
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
    quota_expanded: bool,
) {
    *snapshot = tray_snapshot();
    *title = snapshot.title.clone();
    *next_refresh = Instant::now() + refresh_interval(snapshot.refresh_seconds);
    if let Some(tray) = tray {
        let _: () = tray.set_title(Some(title));
    }
    if let Some(popover) = popover {
        popover.render(snapshot, quota_expanded);
    }
}

struct TraySnapshot {
    title: String,
    quota_groups: Vec<QuotaGroup>,
    calendar_events: Vec<calendar::CalendarEvent>,
    calendar_error: Option<String>,
    calendar_enabled: bool,
    quotes: Vec<StockQuote>,
    stock_errors: Vec<String>,
    todos: Vec<(usize, String, bool)>,
    refresh_seconds: u64,
}

fn tray_snapshot() -> TraySnapshot {
    let usages = fetch_all().0;
    let title = tray_summary(&usages);
    let config = match config::load_or_create() {
        Ok(config) => config,
        Err(error) => {
            return TraySnapshot {
                title,
                quota_groups: quota_groups(&usages),
                calendar_events: Vec::new(),
                calendar_error: Some(error),
                calendar_enabled: true,
                quotes: Vec::new(),
                stock_errors: Vec::new(),
                todos: Vec::new(),
                refresh_seconds: 120,
            };
        }
    };
    let (calendar_events, calendar_error) = match calendar::fetch(&config.calendar) {
        Ok(events) => (events, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let (quotes, stock_errors) = stocks::fetch(&config.stocks);
    let todos = config
        .todos
        .iter()
        .enumerate()
        .map(|(index, todo)| (index, todo.title.clone(), todo.completed))
        .take(5)
        .collect();

    TraySnapshot {
        title,
        quota_groups: quota_groups(&usages),
        calendar_events,
        calendar_error,
        calendar_enabled: config.calendar.enabled,
        quotes,
        stock_errors,
        todos,
        refresh_seconds: config.refresh_seconds,
    }
}

#[derive(Clone, Copy)]
enum PopoverAction {
    Refresh,
    Settings,
    Quit,
    ToggleQuota,
    ToggleTodo(usize),
    EditTodo(usize),
    DeleteTodo(usize),
    AddTodo,
}

static POPOVER_ACTIONS: OnceLock<mpsc::Sender<PopoverAction>> = OnceLock::new();

#[cfg(target_os = "macos")]
mod popover_ui {
    use super::{
        format_time_range, scratched, truncate, PopoverAction, TraySnapshot, POPOVER_ACTIONS,
    };
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

            #[unsafe(method(quit:))]
            fn quit(&self, _: &AnyObject) {
                send(PopoverAction::Quit);
            }

            #[unsafe(method(quota:))]
            fn quota(&self, _: &AnyObject) {
                send(PopoverAction::ToggleQuota);
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
        if let Some(sender) = POPOVER_ACTIONS.get() {
            let _ = sender.send(action);
        }
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
            ui.render(snapshot, false);
            ui
        }

        pub fn toggle(&self, tray: &TrayIcon, snapshot: &TraySnapshot, quota_expanded: bool) {
            if self.popover.isShown() {
                self.popover.close();
                return;
            }
            self.render(snapshot, quota_expanded);
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

        pub fn render(&self, snapshot: &TraySnapshot, quota_expanded: bool) {
            let mtm = MainThreadMarker::new().expect("popover must be rendered on the main thread");
            // Replacing visible popover content normally cross-fades the old
            // and new layouts, which looked like a flickering expand effect.
            let suppress_transition = self.popover.isShown();
            if suppress_transition {
                self.popover.setAnimates(false);
            }
            let height = panel_height(snapshot, quota_expanded);
            let view = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, height)),
            );
            let controller = NSViewController::new(mtm);
            controller.setView(&view);
            populate(&view, &self._target, snapshot, quota_expanded, height, mtm);
            self.popover.setContentViewController(Some(&controller));
            // Setting a view controller can reset the popover to its previous
            // content size. Apply the final size afterwards so expanded quota
            // rows are never clipped.
            self.popover.setContentSize(NSSize::new(WIDTH, height));
            if suppress_transition {
                self.popover.setAnimates(true);
            }
        }
    }

    fn panel_height(snapshot: &TraySnapshot, quota_expanded: bool) -> f64 {
        let mut rows = 3.0; // inline actions and a little breathing room
        rows += 1.4; // Gauge toggle
        if quota_expanded {
            rows += if snapshot.quota_groups.is_empty() {
                1.0
            } else {
                snapshot
                    .quota_groups
                    .iter()
                    .map(|group| 1 + group.hourly_rows.len() + group.weekly_rows.len())
                    .sum::<usize>() as f64
            };
            // Give the detailed quota grid more air than the compact dashboard.
            let quota_lines = snapshot
                .quota_groups
                .iter()
                .map(|group| 1 + group.hourly_rows.len() + group.weekly_rows.len())
                .sum::<usize>()
                .max(1);
            rows += quota_lines as f64 * (9.0 / 19.0);
        }
        if snapshot.calendar_enabled {
            rows += 2.0;
        }
        if !snapshot.quotes.is_empty() || !snapshot.stock_errors.is_empty() {
            rows += 2.0;
        }
        rows += 2.0 + snapshot.todos.len() as f64 + 0.8;
        rows += snapshot.todos.len() as f64 * (4.0 / 19.0);
        12.0 + rows * 19.0
    }

    fn populate(
        view: &NSView,
        target: &PopoverTarget,
        snapshot: &TraySnapshot,
        quota_expanded: bool,
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
        button(
            view,
            target,
            &format!(
                "Gauge  {}  {}",
                snapshot.title,
                if quota_expanded { "⌃" } else { "⌄" }
            ),
            LEFT,
            y,
            CONTENT_WIDTH,
            sel!(quota:),
            -1,
            mtm,
        );
        y -= 20.0;

        if quota_expanded {
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

        if !snapshot.quotes.is_empty() || !snapshot.stock_errors.is_empty() {
            label(view, DIVIDER, y, 10.0, mtm);
            y -= 19.0;
            let quote_line = snapshot
                .quotes
                .iter()
                .map(|quote| match quote.change_percent {
                    Some(change) => format!("{} {change:+.2}%", quote.label()),
                    None => quote.label().to_string(),
                })
                .collect::<Vec<_>>()
                .join("   ");
            let stock_line = if quote_line.is_empty() {
                truncate(&snapshot.stock_errors.join(" · "), 32)
            } else {
                truncate(&quote_line, 34)
            };
            label(view, &stock_line, y, 12.0, mtm);
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
    append_stocks_section(&menu, snapshot);
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
fn append_stocks_section(menu: &Menu, snapshot: &TraySnapshot) {
    if snapshot.quotes.is_empty() && snapshot.stock_errors.is_empty() {
        return;
    }
    menu.append(&PredefinedMenuItem::separator())
        .expect("failed to add stocks separator");
    if !snapshot.quotes.is_empty() {
        let quotes = snapshot
            .quotes
            .iter()
            .map(|quote| {
                let change = quote
                    .change_percent
                    .map(|value| format!(" {value:+.2}%"))
                    .unwrap_or_default();
                format!("{}{}", quote.label(), change)
            })
            .collect::<Vec<_>>()
            .join("    ");
        menu.append(&MenuItem::new(quotes, true, None))
            .expect("failed to add stock quotes");
    }
    for error in &snapshot.stock_errors {
        menu.append(&MenuItem::new(format!("  {error}"), true, None))
            .expect("failed to add stock error");
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
