mod network;

use gauge::{fetch_all, now_seconds, summary, Usage};
use std::{env, process};
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
    gauge                  Show remaining agent quota\n\
    gauge --json           Print the same data as JSON\n\
    gauge --tray           Keep it in the menu bar\n\
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
        return Err("choose only one of --json, --tray, --wifi, or --ble".into());
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
                process::exit(0);
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
