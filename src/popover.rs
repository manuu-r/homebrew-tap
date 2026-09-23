//! The menu-bar popover: what is left, what needs you, what is next.
//!
//! Content is laid out top-down in a flipped view, so the panel measures itself
//! as it is built. There is no separate height formula to drift out of sync.

use std::cell::{Cell, RefCell};

use objc2::{define_class, msg_send, rc::Retained, runtime::AnyObject, sel, MainThreadOnly};
use objc2_app_kit::{
    NSButton, NSColor, NSControl, NSFont, NSImage, NSImageSymbolConfiguration, NSLineBreakMode,
    NSPopover, NSPopoverBehavior, NSScreen, NSScrollView, NSTextAlignment, NSTextField,
    NSViewController,
};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRectEdge, NSSize, NSString,
};
use tray_icon::TrayIcon;

use gauge::{activity::compact, now_seconds, reset_label};

use crate::{
    canvas::{self, FlippedView, MEDIUM, REGULAR, SEMIBOLD},
    format_time_range, post_action, AppAction, TraySnapshot,
};

const WIDTH: f64 = 340.0;
const PAD: f64 = 16.0;
const CONTENT: f64 = WIDTH - PAD * 2.0;
const SECTION_GAP: f64 = 14.0;
const METER_LABEL: f64 = 58.0;
const METER_PERCENT: f64 = 42.0;
const METER_RESET: f64 = 72.0;
const METER_BAR: f64 = CONTENT - METER_LABEL - METER_PERCENT - METER_RESET - 24.0;
const TOKEN_COLUMN: f64 = 60.0;
const EDIT_TAG_BASE: isize = -1_000;
const DELETE_TAG_BASE: isize = -2_000;
const MAX_REQUESTS: usize = 3;
define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    struct PopoverTarget;

    unsafe impl NSObjectProtocol for PopoverTarget {}

    impl PopoverTarget {
        #[unsafe(method(activity:))]
        fn activity(&self, _: &AnyObject) {
            post_action(AppAction::Activity);
        }

        #[unsafe(method(installHooks:))]
        fn install_hooks(&self, _: &AnyObject) {
            post_action(AppAction::InstallHooks);
        }

        #[unsafe(method(refresh:))]
        fn refresh(&self, _: &AnyObject) {
            post_action(AppAction::Refresh);
        }

        #[unsafe(method(settings:))]
        fn settings(&self, _: &AnyObject) {
            post_action(AppAction::Settings);
        }

        #[unsafe(method(pairAccessory:))]
        fn pair_accessory(&self, _: &AnyObject) {
            post_action(AppAction::PairAccessory);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _: &AnyObject) {
            post_action(AppAction::Quit);
        }

        #[unsafe(method(todo:))]
        fn todo(&self, sender: &NSControl) {
            if let Ok(index) = usize::try_from(sender.tag()) {
                post_action(AppAction::ToggleTodo(index));
            }
        }

        #[unsafe(method(editTodo:))]
        fn edit_todo(&self, sender: &NSControl) {
            if let Some(index) = task_index(sender.tag(), EDIT_TAG_BASE) {
                post_action(AppAction::EditTodo(index));
            }
        }

        #[unsafe(method(deleteTodo:))]
        fn delete_todo(&self, sender: &NSControl) {
            if let Some(index) = task_index(sender.tag(), DELETE_TAG_BASE) {
                post_action(AppAction::DeleteTodo(index));
            }
        }

        #[unsafe(method(addTodo:))]
        fn add_todo(&self, _: &AnyObject) {
            post_action(AppAction::AddTodo);
        }
    }
);

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
    scroll: RefCell<Option<Retained<NSScrollView>>>,
    // NSControl keeps its target weak, so own it alongside the popover.
    target: Retained<PopoverTarget>,
}

impl PopoverUi {
    pub fn new(snapshot: &TraySnapshot) -> Self {
        let mtm = MainThreadMarker::new().expect("popover must be built on the main thread");
        let popover = NSPopover::new(mtm);
        // This lets a click update the content without AppKit dismissing the
        // dashboard, while the menu-bar icon remains the explicit toggle.
        popover.setBehavior(NSPopoverBehavior::ApplicationDefined);
        let ui = Self {
            popover,
            scroll: RefCell::new(None),
            target: PopoverTarget::new(mtm),
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
            // `close` normally animates. The external native editor must not
            // be created until the popover has left the screen.
            self.popover.setAnimates(false);
            self.popover.close();
            self.popover.setAnimates(true);
        }
    }

    pub fn render(&self, snapshot: &TraySnapshot) {
        let mtm = MainThreadMarker::new().expect("popover must be rendered on the main thread");
        // Replacing visible popover content normally cross-fades the old and
        // new layouts, which looked like a flickering expand effect.
        let suppress_transition = self.popover.isShown();
        if suppress_transition {
            self.popover.setAnimates(false);
        }

        let panel = Panel::new(&self.target, mtm);
        panel.build(snapshot);
        let height = panel.finish();

        let visible_height = height.min(
            NSScreen::mainScreen(mtm)
                .map(|screen| screen.visibleFrame().size.height - 80.0)
                .unwrap_or(640.0),
        );
        // In a flipped document the scroll origin is the distance from the top,
        // so a refresh keeps the reader where they were.
        let scrolled = self
            .scroll
            .borrow()
            .as_ref()
            .map(|old| old.documentVisibleRect().origin.y)
            .unwrap_or(0.0);
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, visible_height)),
        );
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(height > visible_height);
        scroll.setDocumentView(Some(&panel.view));
        panel.view.scrollPoint(NSPoint::new(
            0.0,
            scrolled.min(height - visible_height).max(0.0),
        ));

        let controller = NSViewController::new(mtm);
        controller.setView(&scroll);
        *self.scroll.borrow_mut() = Some(scroll);
        self.popover.setContentViewController(Some(&controller));
        // Setting a view controller can reset the popover to its previous
        // content size, so apply the final size afterwards.
        self.popover
            .setContentSize(NSSize::new(WIDTH, visible_height));
        if suppress_transition {
            self.popover.setAnimates(true);
        }
    }
}

/// A single top-down pass over the content. `y` is the distance from the top
/// of the panel to the next free line.
struct Panel<'a> {
    view: Retained<FlippedView>,
    target: &'a PopoverTarget,
    y: Cell<f64>,
    mtm: MainThreadMarker,
}

impl<'a> Panel<'a> {
    fn new(target: &'a PopoverTarget, mtm: MainThreadMarker) -> Self {
        Self {
            view: FlippedView::new(WIDTH, mtm),
            target,
            y: Cell::new(14.0),
            mtm,
        }
    }

    fn finish(&self) -> f64 {
        let height = self.y.get() + 12.0;
        self.view.setFrameSize(NSSize::new(WIDTH, height));
        height
    }

    fn advance(&self, by: f64) {
        self.y.set(self.y.get() + by);
    }

    fn build(&self, snapshot: &TraySnapshot) {
        self.header(snapshot);
        self.attention(snapshot);
        self.quota(snapshot);
        self.tokens(snapshot);
        if snapshot.calendar_enabled {
            self.separator();
            self.calendar(snapshot);
        }
        if snapshot.tasks_enabled {
            self.separator();
            self.tasks(snapshot);
        }
        self.separator();
        self.accessory(snapshot);
    }

    // ------------------------------------------------------------ sections --

    fn header(&self, snapshot: &TraySnapshot) {
        let y = self.y.get();
        self.text(
            "Gauge",
            PAD,
            y,
            120.0,
            13.0,
            SEMIBOLD,
            &NSColor::labelColor(),
        );
        self.text(
            &updated_label(snapshot.updated_at, now_seconds()),
            PAD,
            y + 17.0,
            180.0,
            11.0,
            REGULAR,
            &NSColor::secondaryLabelColor(),
        );
        let icons = [
            ("power", "Quit Gauge", sel!(quit:)),
            ("gearshape", "Settings", sel!(settings:)),
            ("arrow.clockwise", "Refresh now", sel!(refresh:)),
        ];
        for (index, (symbol, tip, action)) in icons.into_iter().enumerate() {
            let x = WIDTH - PAD - 22.0 - index as f64 * 28.0;
            self.icon(
                symbol,
                tip,
                x,
                y + 4.0,
                22.0,
                14.0,
                action,
                -1,
                &NSColor::secondaryLabelColor(),
            );
        }
        self.advance(34.0 + SECTION_GAP);
    }

    /// Only speaks when something needs a person. An agent waiting on you is
    /// the one thing here worth colour.
    fn attention(&self, snapshot: &TraySnapshot) {
        let requests = &snapshot.monitoring.attention.requests;
        if requests.is_empty() {
            if !snapshot.hooks_installed {
                self.link(
                    "bell",
                    "Tell me when an agent needs input…",
                    sel!(installHooks:),
                );
                self.advance(SECTION_GAP);
            }
            return;
        }

        let shown = requests.len().min(MAX_REQUESTS);
        let top = self.y.get();
        let height = 30.0 + shown as f64 * 36.0 + if requests.len() > shown { 18.0 } else { 0.0 };
        let orange = NSColor::systemOrangeColor();
        self.fill(
            PAD - 6.0,
            top,
            CONTENT + 12.0,
            height,
            9.0,
            &orange.colorWithAlphaComponent(0.13),
        );

        self.dot(PAD + 2.0, top + 11.0, &orange);
        let title = match requests.len() {
            1 => "An agent needs you".to_string(),
            count => format!("{count} agents need you"),
        };
        self.text(
            &title,
            PAD + 14.0,
            top + 6.0,
            CONTENT - 60.0,
            12.0,
            SEMIBOLD,
            &NSColor::labelColor(),
        );
        self.text(
            "Open ›",
            WIDTH - PAD - 60.0,
            top + 7.0,
            58.0,
            11.0,
            MEDIUM,
            &orange,
        )
        .setAlignment(NSTextAlignment::Right);

        let mut y = top + 28.0;
        for request in requests.iter().take(shown) {
            let project = std::path::Path::new(&request.project)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Session");
            self.text(
                &format!("{} · {project}", request.provider),
                PAD + 14.0,
                y,
                CONTENT - 14.0,
                11.0,
                REGULAR,
                &NSColor::secondaryLabelColor(),
            );
            self.text(
                &request.message,
                PAD + 14.0,
                y + 15.0,
                CONTENT - 14.0,
                12.0,
                REGULAR,
                &NSColor::labelColor(),
            );
            y += 36.0;
        }
        if requests.len() > shown {
            self.text(
                &format!("and {} more", requests.len() - shown),
                PAD + 14.0,
                y - 2.0,
                CONTENT,
                11.0,
                REGULAR,
                &NSColor::secondaryLabelColor(),
            );
        }
        // The whole card is the target; a person should not have to aim.
        self.overlay(
            PAD - 6.0,
            top,
            CONTENT + 12.0,
            height,
            sel!(activity:),
            "Show everything that needs you",
        );
        self.advance(height + SECTION_GAP);
    }

    fn quota(&self, snapshot: &TraySnapshot) {
        if snapshot.meters.is_empty() {
            self.text(
                "Quota unavailable",
                PAD,
                self.y.get(),
                CONTENT,
                12.0,
                REGULAR,
                &NSColor::secondaryLabelColor(),
            );
            self.advance(20.0 + SECTION_GAP);
            return;
        }
        let now = now_seconds();
        for group in &snapshot.meters {
            self.section_title(group.provider);
            for meter in &group.meters {
                self.meter(
                    &meter.label,
                    meter.remaining_percent,
                    &reset_label(meter.resets_at, now),
                );
            }
            self.advance(8.0);
        }
        self.advance(SECTION_GAP - 8.0);
    }

    fn tokens(&self, snapshot: &TraySnapshot) {
        let monitoring = &snapshot.monitoring;
        let top = self.y.get();
        self.section_title("Tokens");
        self.text(
            "History ›",
            WIDTH - PAD - 80.0,
            top,
            80.0,
            11.0,
            MEDIUM,
            &NSColor::controlAccentColor(),
        )
        .setAlignment(NSTextAlignment::Right);
        self.overlay(
            WIDTH - PAD - 80.0,
            top - 2.0,
            80.0,
            18.0,
            sel!(activity:),
            "Usage history and breakdown",
        );

        if !monitoring.ready || monitoring.tokens.providers.is_empty() {
            let message = if monitoring.ready {
                "No sessions yet"
            } else {
                "Reading session history…"
            };
            self.text(
                message,
                PAD,
                self.y.get(),
                CONTENT,
                12.0,
                REGULAR,
                &NSColor::secondaryLabelColor(),
            );
            self.advance(20.0 + SECTION_GAP);
            return;
        }

        let columns = ["Today", "Week", "Month"];
        let y = self.y.get();
        for (index, column) in columns.iter().enumerate() {
            self.number(
                column,
                token_column_x(index),
                y,
                11.0,
                REGULAR,
                &NSColor::tertiaryLabelColor(),
            );
        }
        self.advance(17.0);
        for provider in monitoring
            .tokens
            .providers
            .iter()
            .filter(|provider| provider.provider != "Cursor")
        {
            let y = self.y.get();
            self.text(
                &provider.provider,
                PAD,
                y,
                90.0,
                12.0,
                REGULAR,
                &NSColor::labelColor(),
            );
            let values = [&provider.today, &provider.week, &provider.month];
            for (index, period) in values.iter().enumerate() {
                self.number(
                    &compact(period.tokens.total()),
                    token_column_x(index),
                    y,
                    12.0,
                    MEDIUM,
                    &NSColor::labelColor(),
                );
            }
            self.advance(19.0);
        }
        self.advance(SECTION_GAP - 4.0);
    }

    fn calendar(&self, snapshot: &TraySnapshot) {
        let y = self.y.get();
        self.symbol(
            "calendar",
            PAD,
            y - 1.0,
            16.0,
            &NSColor::secondaryLabelColor(),
        );
        let (title, when, muted) = if let Some(error) = &snapshot.calendar_error {
            (error.clone(), String::new(), true)
        } else if let Some(event) = snapshot.calendar_events.first() {
            let when = if event.all_day {
                "All day".into()
            } else {
                format_time_range(event.starts_at, event.ends_at)
            };
            (event.title.clone(), when, false)
        } else {
            ("No upcoming events".into(), String::new(), true)
        };
        let color = if muted {
            NSColor::secondaryLabelColor()
        } else {
            NSColor::labelColor()
        };
        let when_width = if when.is_empty() { 0.0 } else { 90.0 };
        self.text(
            &title,
            PAD + 24.0,
            y,
            CONTENT - 24.0 - when_width,
            12.0,
            REGULAR,
            &color,
        );
        if !when.is_empty() {
            self.number(
                &when,
                WIDTH - PAD - when_width,
                y,
                12.0,
                REGULAR,
                &NSColor::secondaryLabelColor(),
            )
            .setFrameSize(NSSize::new(when_width, 16.0));
        }
        self.advance(20.0 + SECTION_GAP - 4.0);
    }

    fn tasks(&self, snapshot: &TraySnapshot) {
        let top = self.y.get();
        self.section_title("Today");
        self.icon(
            "plus",
            "Add a to-do",
            WIDTH - PAD - 20.0,
            top - 3.0,
            20.0,
            13.0,
            sel!(addTodo:),
            -1,
            &NSColor::secondaryLabelColor(),
        );

        if snapshot.todos.is_empty() {
            self.plain_button(
                "Add a to-do…",
                PAD,
                self.y.get(),
                CONTENT,
                sel!(addTodo:),
                -1,
                &NSColor::secondaryLabelColor(),
            );
            self.advance(22.0 + SECTION_GAP - 4.0);
            return;
        }

        for (index, title, completed) in &snapshot.todos {
            let y = self.y.get();
            let tag = *index as isize;
            let (symbol, tint) = if *completed {
                ("checkmark.circle.fill", NSColor::controlAccentColor())
            } else {
                ("circle", NSColor::tertiaryLabelColor())
            };
            self.icon(
                symbol,
                "Mark done",
                PAD - 2.0,
                y - 1.0,
                20.0,
                15.0,
                sel!(todo:),
                tag,
                &tint,
            );
            let text_color = if *completed {
                NSColor::tertiaryLabelColor()
            } else {
                NSColor::labelColor()
            };
            self.plain_button(
                title,
                PAD + 24.0,
                y - 1.0,
                CONTENT - 24.0 - 50.0,
                sel!(todo:),
                tag,
                &text_color,
            );
            self.icon(
                "pencil",
                "Edit",
                WIDTH - PAD - 44.0,
                y - 1.0,
                20.0,
                11.0,
                sel!(editTodo:),
                EDIT_TAG_BASE - tag,
                &NSColor::tertiaryLabelColor(),
            );
            self.icon(
                "xmark",
                "Delete",
                WIDTH - PAD - 20.0,
                y - 1.0,
                20.0,
                10.0,
                sel!(deleteTodo:),
                DELETE_TAG_BASE - tag,
                &NSColor::tertiaryLabelColor(),
            );
            self.advance(24.0);
        }
        self.advance(SECTION_GAP - 8.0);
    }

    fn accessory(&self, snapshot: &TraySnapshot) {
        let y = self.y.get();
        if !snapshot.accessory_paired {
            self.link(
                "plus.circle",
                &snapshot.accessory_line,
                sel!(pairAccessory:),
            );
            return;
        }
        self.symbol("display", PAD, y, 16.0, &NSColor::secondaryLabelColor());
        self.text(
            &snapshot.accessory_line,
            PAD + 24.0,
            y,
            CONTENT - 130.0,
            12.0,
            REGULAR,
            &NSColor::labelColor(),
        );
        let (status, color) = if snapshot.accessory_connected {
            ("Connected", NSColor::systemGreenColor())
        } else {
            ("Not connected", NSColor::tertiaryLabelColor())
        };
        self.dot(WIDTH - PAD - 96.0, y + 5.0, &color);
        self.text(
            status,
            WIDTH - PAD - 86.0,
            y + 1.0,
            86.0,
            11.0,
            REGULAR,
            &NSColor::secondaryLabelColor(),
        )
        .setAlignment(NSTextAlignment::Right);
        self.overlay(
            PAD - 4.0,
            y - 3.0,
            CONTENT + 8.0,
            22.0,
            sel!(settings:),
            "Accessory settings",
        );
        self.advance(20.0);
    }

    // ---------------------------------------------------------- components --

    fn section_title(&self, title: &str) {
        self.text(
            title,
            PAD,
            self.y.get(),
            CONTENT,
            12.0,
            SEMIBOLD,
            &NSColor::labelColor(),
        );
        self.advance(21.0);
    }

    /// Remaining quota as a capsule that empties. It stays the accent colour
    /// until running low actually matters.
    fn meter(&self, label: &str, remaining: u64, reset: &str) {
        let y = self.y.get();
        self.text(
            label,
            PAD,
            y,
            METER_LABEL,
            12.0,
            REGULAR,
            &NSColor::secondaryLabelColor(),
        );

        let bar_x = PAD + METER_LABEL + 8.0;
        let bar_y = y + 5.0;
        self.fill(
            bar_x,
            bar_y,
            METER_BAR,
            6.0,
            3.0,
            &NSColor::quaternaryLabelColor(),
        );
        let fraction = remaining.min(100) as f64 / 100.0;
        if fraction > 0.0 {
            let tint = match remaining {
                0..=10 => NSColor::systemRedColor(),
                11..=25 => NSColor::systemOrangeColor(),
                _ => NSColor::controlAccentColor(),
            };
            self.fill(
                bar_x,
                bar_y,
                (METER_BAR * fraction).max(6.0),
                6.0,
                3.0,
                &tint,
            );
        }

        let percent_x = bar_x + METER_BAR + 8.0;
        self.number(
            &format!("{remaining}%"),
            percent_x,
            y,
            12.0,
            MEDIUM,
            &NSColor::labelColor(),
        )
        .setFrameSize(NSSize::new(METER_PERCENT, 16.0));
        self.number(
            reset,
            percent_x + METER_PERCENT + 8.0,
            y + 1.0,
            11.0,
            REGULAR,
            &NSColor::tertiaryLabelColor(),
        )
        .setFrameSize(NSSize::new(METER_RESET, 15.0));
        self.advance(22.0);
    }

    fn separator(&self) {
        canvas::separator(&self.view, PAD, self.y.get(), CONTENT);
        self.advance(1.0 + SECTION_GAP - 2.0);
    }

    fn link(&self, symbol: &str, title: &str, action: objc2::runtime::Sel) {
        let y = self.y.get();
        self.symbol(symbol, PAD, y, 16.0, &NSColor::secondaryLabelColor());
        self.plain_button(
            title,
            PAD + 24.0,
            y - 1.0,
            CONTENT - 24.0,
            action,
            -1,
            &NSColor::secondaryLabelColor(),
        );
        self.advance(20.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        text: &str,
        x: f64,
        y: f64,
        width: f64,
        size: f64,
        weight: f64,
        color: &NSColor,
    ) -> Retained<NSTextField> {
        canvas::text(&self.view, text, x, y, width, size, weight, color)
    }

    fn number(
        &self,
        text: &str,
        x: f64,
        y: f64,
        size: f64,
        weight: f64,
        color: &NSColor,
    ) -> Retained<NSTextField> {
        canvas::number(&self.view, text, x, y, TOKEN_COLUMN, size, weight, color)
    }

    fn fill(&self, x: f64, y: f64, width: f64, height: f64, radius: f64, color: &NSColor) {
        canvas::fill(&self.view, canvas::rect(x, y, width, height), radius, color);
    }

    fn dot(&self, x: f64, y: f64, color: &NSColor) {
        self.fill(x, y, 7.0, 7.0, 3.5, color);
    }

    fn symbol(&self, name: &str, x: f64, y: f64, size: f64, color: &NSColor) {
        let Some(image) = symbol_image(name, 12.0) else {
            return;
        };
        let button =
            unsafe { NSButton::buttonWithImage_target_action(&image, None, None, self.mtm) };
        button.setBordered(false);
        button.setEnabled(false);
        button.setContentTintColor(Some(color));
        button.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(size, size)));
        self.view.addSubview(&button);
    }

    #[allow(clippy::too_many_arguments)]
    fn icon(
        &self,
        name: &str,
        tip: &str,
        x: f64,
        y: f64,
        size: f64,
        point_size: f64,
        action: objc2::runtime::Sel,
        tag: isize,
        color: &NSColor,
    ) {
        let Some(image) = symbol_image(name, point_size) else {
            return;
        };
        let button = unsafe {
            NSButton::buttonWithImage_target_action(
                &image,
                Some(self.target),
                Some(action),
                self.mtm,
            )
        };
        button.setBordered(false);
        button.setContentTintColor(Some(color));
        button.setTag(tag);
        button.setToolTip(Some(&NSString::from_str(tip)));
        button.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(size, size)));
        self.view.addSubview(&button);
    }

    #[allow(clippy::too_many_arguments)]
    fn plain_button(
        &self,
        title: &str,
        x: f64,
        y: f64,
        width: f64,
        action: objc2::runtime::Sel,
        tag: isize,
        color: &NSColor,
    ) {
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(title),
                Some(self.target),
                Some(action),
                self.mtm,
            )
        };
        button.setBordered(false);
        button.setAlignment(NSTextAlignment::Left);
        button.setContentTintColor(Some(color));
        button.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        button.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        button.setTag(tag);
        button.setToolTip(Some(&NSString::from_str(title)));
        button.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, 18.0)));
        self.view.addSubview(&button);
    }

    /// An invisible button over a block of content, added last so it receives
    /// the click wherever it lands.
    fn overlay(
        &self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        action: objc2::runtime::Sel,
        tip: &str,
    ) {
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(""),
                Some(self.target),
                Some(action),
                self.mtm,
            )
        };
        button.setBordered(false);
        button.setTransparent(true);
        button.setToolTip(Some(&NSString::from_str(tip)));
        button.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)));
        self.view.addSubview(&button);
    }
}

fn token_column_x(index: usize) -> f64 {
    WIDTH - PAD - (3 - index) as f64 * TOKEN_COLUMN
}

fn symbol_image(name: &str, point_size: f64) -> Option<Retained<NSImage>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        None,
    )?;
    let configuration =
        NSImageSymbolConfiguration::configurationWithPointSize_weight(point_size, REGULAR);
    image.imageWithSymbolConfiguration(&configuration)
}

fn updated_label(updated_at: u64, now: u64) -> String {
    match now.saturating_sub(updated_at) {
        0..=59 => "Updated just now".into(),
        seconds @ 60..=3_599 => format!("Updated {} min ago", seconds / 60),
        seconds => format!("Updated {} h ago", seconds / 3_600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_how_fresh_the_numbers_are() {
        assert_eq!(updated_label(100, 130), "Updated just now");
        assert_eq!(updated_label(0, 600), "Updated 10 min ago");
        assert_eq!(updated_label(0, 7_300), "Updated 2 h ago");
    }

    #[test]
    fn token_columns_end_at_the_right_margin() {
        assert_eq!(token_column_x(2) + TOKEN_COLUMN, WIDTH - PAD);
    }
}
