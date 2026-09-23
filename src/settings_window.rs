//! Gauge's settings, as one window with no tabs and nothing hidden.
//!
//! Every control is a direct switch on a single visible fact, so the window is
//! also the honest inventory of what Gauge does. There is no Apply button: a
//! change is the act itself, written the moment it is made.

use std::cell::RefCell;

use objc2::{
    define_class, msg_send, rc::Retained, runtime::AnyObject, sel, MainThreadOnly, Message,
};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSButton, NSColor, NSControl, NSFont, NSPopUpButton,
    NSScreen, NSSwitch, NSTextAlignment, NSTextField, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use gauge::{
    config::{Config, REFRESH_CHOICES},
    devices::PairedDevice,
    now_seconds,
};

use crate::{post_action, AppAction, Switch};

const WIDTH: f64 = 400.0;
const LEFT: f64 = 22.0;
const CONTENT: f64 = WIDTH - (LEFT * 2.0);
const TOP_INSET: f64 = 20.0;
const BOTTOM_INSET: f64 = 18.0;
const HEADER_HEIGHT: f64 = 27.0;
const ROW_HEIGHT: f64 = 32.0;
const DEVICE_HEIGHT: f64 = 46.0;
const EMPTY_HEIGHT: f64 = 28.0;
const PAIR_HEIGHT: f64 = 42.0;
const SECTION_GAP: f64 = 16.0;
const FOOTER_HEIGHT: f64 = 28.0;
const CONTROL_WIDTH: f64 = 160.0;
const FORGET_TAG_BASE: isize = 100;

/// What the window is showing. Collected once per render so the switches, the
/// device list, and the menu bar can never disagree about the same instant.
pub struct SettingsSnapshot {
    pub config: Config,
    pub devices: Vec<PairedDevice>,
    pub open_at_login: bool,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    struct SettingsTarget;

    unsafe impl NSObjectProtocol for SettingsTarget {}

    impl SettingsTarget {
        #[unsafe(method(toggle:))]
        fn toggle(&self, sender: &NSControl) {
            let Some(switch) = Switch::from_tag(sender.tag()) else {
                return;
            };
            let state: isize = unsafe { msg_send![sender, state] };
            post_action(AppAction::SetSwitch(switch, state != 0));
        }

        #[unsafe(method(chooseRefresh:))]
        fn choose_refresh(&self, sender: &NSControl) {
            let index: isize = unsafe { msg_send![sender, indexOfSelectedItem] };
            if let Some(seconds) = usize::try_from(index).ok().and_then(|index| REFRESH_CHOICES.get(index)) {
                post_action(AppAction::SetRefreshSeconds(*seconds));
            }
        }

        #[unsafe(method(pair:))]
        fn pair(&self, _: &AnyObject) {
            post_action(AppAction::PairAccessory);
        }

        #[unsafe(method(forget:))]
        fn forget(&self, sender: &NSControl) {
            let index = sender.tag() - FORGET_TAG_BASE;
            if let Ok(index) = usize::try_from(index) {
                post_action(AppAction::ForgetDevice(index));
            }
        }

        #[unsafe(method(openConfigurationFile:))]
        fn open_configuration_file(&self, _: &AnyObject) {
            post_action(AppAction::OpenConfigurationFile);
        }
    }
);

impl SettingsTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm);
        unsafe { msg_send![this, init] }
    }
}

pub struct SettingsWindow {
    window: Retained<NSWindow>,
    // NSControl keeps its target weak, so the window owns it.
    target: Retained<SettingsTarget>,
    // The rows the user is looking at, so a Forget button always refers to the
    // device printed beside it rather than to a position in a stale list.
    device_ids: RefCell<Vec<String>>,
}

impl SettingsWindow {
    pub fn new(snapshot: &SettingsSnapshot) -> Self {
        let mtm = MainThreadMarker::new().expect("settings must be built on the main thread");
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, 100.0)),
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(&NSString::from_str("Gauge"));
        // The tray owns this window for the life of the process.
        unsafe { window.setReleasedWhenClosed(false) };
        let this = Self {
            window,
            target: SettingsTarget::new(mtm),
            device_ids: RefCell::new(Vec::new()),
        };
        this.render(snapshot);
        this
    }

    /// `center()` uses whichever screen happens to hold the key window, which
    /// for a menu-bar utility is nothing in particular. Settings belongs on the
    /// display the menu bar is on — the one the user just clicked.
    fn place_on_menu_bar_screen(&self, mtm: MainThreadMarker) {
        let Some(screen) = NSScreen::screens(mtm).firstObject() else {
            self.window.center();
            return;
        };
        let visible = screen.visibleFrame();
        let size = self.window.frame().size;
        let origin = NSPoint::new(
            visible.origin.x + (visible.size.width - size.width) / 2.0,
            visible.origin.y + (visible.size.height - size.height) * 0.66,
        );
        self.window.setFrameOrigin(origin);
    }

    /// Rebuild the whole window from the snapshot. Settings has a handful of
    /// controls, so replacing the content is simpler than tracking each one and
    /// guarantees the window agrees with what was saved.
    pub fn render(&self, snapshot: &SettingsSnapshot) {
        let mtm = MainThreadMarker::new().expect("settings must be rendered on the main thread");
        *self.device_ids.borrow_mut() = snapshot
            .devices
            .iter()
            .map(|device| device.id.clone())
            .collect();

        let height = window_height(snapshot);
        let view = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, height)),
        );
        populate(&view, &self.target, snapshot, height, mtm);
        self.window.setContentSize(NSSize::new(WIDTH, height));
        self.window.setContentView(Some(&view));
    }

    #[allow(deprecated)]
    pub fn show(&self, snapshot: &SettingsSnapshot) {
        let mtm = MainThreadMarker::new().expect("settings must be shown on the main thread");
        let first_time = !self.window.isVisible();
        self.render(snapshot);
        if first_time {
            self.place_on_menu_bar_screen(mtm);
        }
        // A menu-bar utility is not the active app when it is asked for its
        // settings, and macOS may refuse the activation request outright.
        // Ordering the window front regardless is what guarantees that asking
        // for Settings actually shows Settings.
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        self.window.makeKeyAndOrderFront(None);
        self.window.orderFrontRegardless();
    }

    pub fn is_visible(&self) -> bool {
        self.window.isVisible()
    }

    pub fn device_id(&self, index: usize) -> Option<String> {
        self.device_ids.borrow().get(index).cloned()
    }
}

/// Mirrors `populate` in exact pixel units. Keep the two adjacent: a settings
/// row that is laid out but not counted here falls off the bottom of the window.
fn window_height(snapshot: &SettingsSnapshot) -> f64 {
    let devices = if snapshot.devices.is_empty() {
        EMPTY_HEIGHT
    } else {
        snapshot.devices.len() as f64 * DEVICE_HEIGHT
    };
    TOP_INSET
        + HEADER_HEIGHT
        + 2.0 * ROW_HEIGHT
        + SECTION_GAP
        + HEADER_HEIGHT
        + 5.0 * ROW_HEIGHT
        + SECTION_GAP
        + HEADER_HEIGHT
        + ROW_HEIGHT
        + devices
        + PAIR_HEIGHT
        + SECTION_GAP
        + FOOTER_HEIGHT
        + BOTTOM_INSET
}

fn populate(
    view: &NSView,
    target: &SettingsTarget,
    snapshot: &SettingsSnapshot,
    height: f64,
    mtm: MainThreadMarker,
) {
    let config = &snapshot.config;
    let mut y = height - TOP_INSET - 17.0;

    header(view, "Updates", y, mtm);
    y -= HEADER_HEIGHT;
    refresh_row(view, target, config.refresh_seconds, y, mtm);
    y -= ROW_HEIGHT;
    switch_row(
        view,
        target,
        "Open at login",
        Switch::OpenAtLogin,
        snapshot.open_at_login,
        y,
        mtm,
    );
    y -= ROW_HEIGHT + SECTION_GAP;

    header(view, "Show", y, mtm);
    y -= HEADER_HEIGHT;
    for (title, switch, on) in [
        ("Codex quota", Switch::Codex, config.providers.codex),
        ("Claude quota", Switch::Claude, config.providers.claude),
        ("Cursor quota", Switch::Cursor, config.providers.cursor),
        (
            "Next calendar event",
            Switch::Calendar,
            config.calendar.enabled,
        ),
        ("Today's tasks", Switch::Tasks, config.tasks.enabled),
    ] {
        switch_row(view, target, title, switch, on, y, mtm);
        y -= ROW_HEIGHT;
    }
    y -= SECTION_GAP;

    header(view, "Accessories", y, mtm);
    y -= HEADER_HEIGHT;
    switch_row(
        view,
        target,
        "Share this dashboard",
        Switch::Accessories,
        config.accessories.enabled,
        y,
        mtm,
    );
    y -= ROW_HEIGHT;

    if snapshot.devices.is_empty() {
        secondary(view, "Nothing paired yet.", LEFT, y + 4.0, CONTENT, mtm);
        y -= EMPTY_HEIGHT;
    } else {
        for (index, device) in snapshot.devices.iter().enumerate() {
            device_row(view, target, index, device, y, mtm);
            y -= DEVICE_HEIGHT;
        }
    }

    let pair = bordered_button(
        view,
        target,
        "Pair Accessory…",
        LEFT,
        y + 6.0,
        150.0,
        sel!(pair:),
        mtm,
    );
    pair.setTag(-1);
    y -= PAIR_HEIGHT + SECTION_GAP;

    secondary(
        view,
        &format!("Gauge {}", env!("CARGO_PKG_VERSION")),
        LEFT,
        y,
        140.0,
        mtm,
    );
    let file = plain_button(
        view,
        target,
        "Configuration file…",
        LEFT + CONTENT - 150.0,
        y - 2.0,
        150.0,
        sel!(openConfigurationFile:),
        mtm,
    );
    file.setAlignment(NSTextAlignment::Right);
}

fn header(view: &NSView, title: &str, y: f64, mtm: MainThreadMarker) {
    let field = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    field.setFrame(NSRect::new(
        NSPoint::new(LEFT, y),
        NSSize::new(CONTENT, 17.0),
    ));
    field.setFont(Some(&NSFont::systemFontOfSize_weight(11.0, 0.4)));
    let color = NSColor::secondaryLabelColor();
    field.setTextColor(Some(&color));
    view.addSubview(&field);
}

fn switch_row(
    view: &NSView,
    target: &SettingsTarget,
    title: &str,
    switch: Switch,
    on: bool,
    y: f64,
    mtm: MainThreadMarker,
) {
    let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    label.setFrame(NSRect::new(
        NSPoint::new(LEFT, y),
        NSSize::new(CONTENT - 60.0, 18.0),
    ));
    label.setFont(Some(&NSFont::systemFontOfSize_weight(13.0, 0.0)));
    view.addSubview(&label);

    let control = NSSwitch::initWithFrame(
        NSSwitch::alloc(mtm),
        NSRect::new(
            NSPoint::new(LEFT + CONTENT - 38.0, y - 3.0),
            NSSize::new(38.0, 22.0),
        ),
    );
    let state: isize = if on { 1 } else { 0 };
    unsafe {
        let _: () = msg_send![&control, setState: state];
        control.setTarget(Some(target));
        control.setAction(Some(sel!(toggle:)));
    }
    control.setTag(switch.tag());
    view.addSubview(&control);
}

fn refresh_row(
    view: &NSView,
    target: &SettingsTarget,
    refresh_seconds: u64,
    y: f64,
    mtm: MainThreadMarker,
) {
    let label = NSTextField::labelWithString(&NSString::from_str("Refresh"), mtm);
    label.setFrame(NSRect::new(
        NSPoint::new(LEFT, y),
        NSSize::new(CONTENT - CONTROL_WIDTH, 18.0),
    ));
    label.setFont(Some(&NSFont::systemFontOfSize_weight(13.0, 0.0)));
    view.addSubview(&label);

    let popup = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        NSRect::new(
            NSPoint::new(LEFT + CONTENT - CONTROL_WIDTH, y - 4.0),
            NSSize::new(CONTROL_WIDTH, 25.0),
        ),
        false,
    );
    for seconds in REFRESH_CHOICES {
        popup.addItemWithTitle(&NSString::from_str(&interval_title(seconds)));
    }
    let selected = REFRESH_CHOICES
        .iter()
        .position(|seconds| *seconds >= refresh_seconds)
        .unwrap_or(REFRESH_CHOICES.len() - 1);
    popup.selectItemAtIndex(selected as isize);
    unsafe {
        popup.setTarget(Some(target));
        popup.setAction(Some(sel!(chooseRefresh:)));
    }
    view.addSubview(&popup);
}

fn device_row(
    view: &NSView,
    target: &SettingsTarget,
    index: usize,
    device: &PairedDevice,
    y: f64,
    mtm: MainThreadMarker,
) {
    let name = NSTextField::labelWithString(&NSString::from_str(&device.name), mtm);
    name.setFrame(NSRect::new(
        NSPoint::new(LEFT, y + 4.0),
        NSSize::new(CONTENT - 70.0, 18.0),
    ));
    name.setFont(Some(&NSFont::systemFontOfSize_weight(13.0, 0.0)));
    view.addSubview(&name);

    secondary(
        view,
        &device_detail(device),
        LEFT,
        y - 12.0,
        CONTENT - 70.0,
        mtm,
    );

    let forget = plain_button(
        view,
        target,
        "Forget",
        LEFT + CONTENT - 60.0,
        y + 2.0,
        60.0,
        sel!(forget:),
        mtm,
    );
    forget.setAlignment(NSTextAlignment::Right);
    forget.setTag(FORGET_TAG_BASE + index as isize);
}

/// Answers the question the menu bar cannot: is this thing actually talking to
/// me? A paired accessory polls continuously, so silence is meaningful.
fn device_detail(device: &PairedDevice) -> String {
    let kind = capitalized(&device.kind);
    let Some(last_seen) = device.last_seen_at else {
        return format!("{kind} · Not connected yet");
    };
    let elapsed = now_seconds().saturating_sub(last_seen);
    let when = match elapsed {
        0..=180 => "Connected".to_string(),
        181..=3_599 => format!("Last seen {} min ago", elapsed / 60),
        3_600..=86_399 => format!("Last seen {} h ago", elapsed / 3_600),
        _ => format!("Last seen {} days ago", elapsed / 86_400),
    };
    format!("{kind} · {when}")
}

fn capitalized(value: &str) -> String {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => "Accessory".into(),
    }
}

fn interval_title(seconds: u64) -> String {
    match seconds {
        60 => "Every minute".into(),
        seconds if seconds < 3_600 => format!("Every {} minutes", seconds / 60),
        _ => format!("Every {} hours", seconds / 3_600),
    }
}

fn secondary(view: &NSView, text: &str, x: f64, y: f64, width: f64, mtm: MainThreadMarker) {
    let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, 16.0)));
    field.setFont(Some(&NSFont::systemFontOfSize_weight(11.0, 0.0)));
    let color = NSColor::secondaryLabelColor();
    field.setTextColor(Some(&color));
    view.addSubview(&field);
}

#[allow(clippy::too_many_arguments)]
fn plain_button(
    view: &NSView,
    target: &SettingsTarget,
    title: &str,
    x: f64,
    y: f64,
    width: f64,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(target),
            Some(action),
            mtm,
        )
    };
    button.setBordered(false);
    button.setFont(Some(&NSFont::systemFontOfSize_weight(11.0, 0.0)));
    let color = NSColor::secondaryLabelColor();
    button.setContentTintColor(Some(&color));
    button.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, 18.0)));
    view.addSubview(&button);
    button.retain()
}

#[allow(clippy::too_many_arguments)]
fn bordered_button(
    view: &NSView,
    target: &SettingsTarget,
    title: &str,
    x: f64,
    y: f64,
    width: f64,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(target),
            Some(action),
            mtm,
        )
    };
    button.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, 28.0)));
    view.addSubview(&button);
    button.retain()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(last_seen_at: Option<u64>) -> PairedDevice {
        PairedDevice {
            id: "bunty-01".into(),
            name: "Bunty-4F2A1C".into(),
            kind: "display".into(),
            firmware_version: None,
            protocol_version: 1,
            capabilities: Vec::new(),
            paired_at: 0,
            last_seen_at,
        }
    }

    #[test]
    fn reports_whether_an_accessory_is_still_there() {
        assert_eq!(device_detail(&device(None)), "Display · Not connected yet");
        assert_eq!(
            device_detail(&device(Some(now_seconds()))),
            "Display · Connected"
        );
        assert_eq!(
            device_detail(&device(Some(now_seconds().saturating_sub(600)))),
            "Display · Last seen 10 min ago"
        );
    }

    #[test]
    fn names_every_refresh_choice_in_plain_words() {
        assert_eq!(interval_title(60), "Every minute");
        assert_eq!(interval_title(300), "Every 5 minutes");
    }
}
