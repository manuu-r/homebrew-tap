use gauge::{fetch_all, now_seconds, summary, Usage};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    TrayIcon, TrayIconBuilder,
};
use winit::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoop},
    platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS},
};

const HELP: &str = "Gauge\n\
    \n\
    gauge            Show remaining agent quota\n\
    gauge --json     Print the same data as JSON\n\
    gauge --tray     Keep it in the menu bar\n\
    gauge --version  Print the version";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|arg| arg == flag);

    if has("-h") || has("--help") {
        println!("{HELP}");
        return;
    }

    if has("-V") || has("--version") {
        println!("gauge {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if has("--tray") {
        return run_tray();
    }

    let (usages, errors) = fetch_all();

    if has("--json") {
        println!("{}", json(&usages, &errors));
        return;
    }

    if usages.is_empty() {
        errors.iter().for_each(|error| eprintln!("- {error}"));
        std::process::exit(1);
    }

    println!("{}", summary(&usages));
    errors
        .iter()
        .for_each(|error| eprintln!("warning: {error}"));
}

fn json(usages: &[Usage], errors: &[String]) -> String {
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

    let payload = serde_json::json!({
        "generated_at": now_seconds(),
        "providers": providers,
        "errors": errors,
    });
    serde_json::to_string_pretty(&payload).unwrap_or_default()
}

/// The menu-bar title carries the numbers, so the menu only needs actions.
#[allow(deprecated)]
fn run_tray() {
    let refresh = MenuItem::new("Refresh", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::new();
    menu.append_items(&[&refresh, &quit])
        .expect("failed to build tray menu");

    let mut title = summary(&fetch_all().0);
    let mut tray: Option<TrayIcon> = None;
    let events = MenuEvent::receiver();

    // Accessory keeps Gauge out of the Dock and the app switcher, which is what
    // a menu bar utility should do.
    let event_loop = EventLoop::builder()
        .with_activation_policy(ActivationPolicy::Accessory)
        .build()
        .expect("failed to start event loop");
    let _ = event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::Wait);

        if let Event::NewEvents(StartCause::Init) = event {
            tray = Some(
                TrayIconBuilder::new()
                    .with_menu(Box::new(menu.clone()))
                    .with_title(&title)
                    .with_tooltip("Gauge")
                    .build()
                    .expect("failed to build tray icon"),
            );
        }

        while let Ok(event) = events.try_recv() {
            if event.id == *quit.id() {
                std::process::exit(0);
            }
            if event.id == *refresh.id() {
                title = summary(&fetch_all().0);
                if let Some(tray) = &tray {
                    let _: () = tray.set_title(Some(&title));
                }
            }
        }
    });
}
