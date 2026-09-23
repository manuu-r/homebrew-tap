//! Activity: what needs you, how much each agent used, and on which days.
//!
//! One period is shown at a time. The numbers answer a question in words first
//! ("313.7M tokens, mostly re-read from cache") and only then as a chart.

use std::cell::Cell;
use std::collections::BTreeMap;

use chrono::{Datelike, Duration, Local, NaiveDate};
use gauge::activity::{compact, Monitoring, Period};
use objc2::{define_class, msg_send, rc::Retained, runtime::AnyObject, sel, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSColor, NSControl, NSScrollView, NSSegmentSwitchTracking,
    NSSegmentedControl, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSPoint, NSSize, NSString,
};

use crate::canvas::{self, provider_color, rect, FlippedView, MEDIUM, REGULAR, SEMIBOLD};
use crate::{post_action, AppAction};

const WIDTH: f64 = 600.0;
const HEIGHT: f64 = 680.0;
const PAD: f64 = 28.0;
const CONTENT: f64 = WIDTH - PAD * 2.0;
const SECTION_GAP: f64 = 30.0;
const CHART_DAYS: i64 = 14;
const CHART_HEIGHT: f64 = 110.0;
const PERIODS: [&str; 4] = ["Today", "This Week", "This Month", "All Time"];

thread_local! {
    /// The week is the useful default: long enough to show a pattern, short
    /// enough to act on.
    static PERIOD: Cell<usize> = const { Cell::new(1) };
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    struct ActivityTarget;

    unsafe impl NSObjectProtocol for ActivityTarget {}

    impl ActivityTarget {
        #[unsafe(method(choosePeriod:))]
        fn choose_period(&self, sender: &NSControl) {
            let selected: isize = unsafe { msg_send![sender, selectedSegment] };
            if let Ok(selected) = usize::try_from(selected) {
                PERIOD.with(|period| period.set(selected.min(PERIODS.len() - 1)));
                post_action(AppAction::Activity);
            }
        }

        #[unsafe(method(ignore:))]
        fn ignore(&self, _: &AnyObject) {}
    }
);

impl ActivityTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm);
        unsafe { msg_send![this, init] }
    }
}

pub struct ActivityWindow {
    window: Retained<NSWindow>,
    scroll: Retained<NSScrollView>,
    target: Retained<ActivityTarget>,
    rendered: String,
}

impl ActivityWindow {
    pub fn new() -> Self {
        let mtm = MainThreadMarker::new().expect("activity must be built on the main thread");
        let frame = rect(0.0, 0.0, WIDTH, HEIGHT);
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str("Activity"));
        window.center();
        let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), frame);
        scroll.setHasVerticalScroller(true);
        scroll.setDrawsBackground(false);
        window.setContentView(Some(&scroll));
        Self {
            window,
            scroll,
            target: ActivityTarget::new(mtm),
            rendered: String::new(),
        }
    }

    pub fn show(&mut self, data: &Monitoring) {
        self.render(data);
        let mtm = MainThreadMarker::new().expect("activity must be shown on the main thread");
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        self.window.makeKeyAndOrderFront(None);
        self.window.orderFrontRegardless();
    }

    pub fn render(&mut self, data: &Monitoring) {
        let period = PERIOD.with(Cell::get);
        let signature = format!(
            "{period}:{}",
            serde_json::to_string(data).unwrap_or_default()
        );
        if signature == self.rendered {
            return;
        }
        self.rendered = signature;

        let mtm = MainThreadMarker::new().expect("activity must be rendered on the main thread");
        let view = FlippedView::new(WIDTH, mtm);
        let mut y = 24.0;
        y = attention(&view, data, y);
        y = totals(&view, &self.target, data, period, y);
        if data.ready {
            y = daily_chart(&view, data, y);
            y = by_model(&view, data, period, y);
            y = footnote(&view, data, y);
        }
        view.setFrameSize(NSSize::new(WIDTH, (y + 24.0).max(HEIGHT)));

        // A window that is opening starts at the top; one already open keeps
        // the reader's place across refreshes.
        let scrolled = if self.window.isVisible() {
            self.scroll.documentVisibleRect().origin.y
        } else {
            0.0
        };
        self.scroll.setDocumentView(Some(&view));
        view.scrollPoint(NSPoint::new(0.0, scrolled));
    }
}

// ----------------------------------------------------------------- sections --

fn attention(view: &NSView, data: &Monitoring, mut y: f64) -> f64 {
    let requests = &data.attention.requests;
    if requests.is_empty() {
        return y;
    }
    heading(view, "Needs you", y);
    y += 30.0;
    let now = gauge::now_seconds();
    for request in requests {
        let card = canvas::fill(
            view,
            rect(PAD, y, CONTENT, 0.0),
            10.0,
            &NSColor::systemOrangeColor().colorWithAlphaComponent(0.10),
        );
        let top = y;
        let inner = CONTENT - 32.0;
        y += 14.0;
        let project = std::path::Path::new(&request.project)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Session");
        dot(
            view,
            PAD + 16.0,
            y + 5.0,
            &provider_color(&request.provider),
        );
        canvas::text(
            view,
            &format!("{} · {project}", request.provider),
            PAD + 30.0,
            y,
            inner - 110.0,
            12.0,
            SEMIBOLD,
            &NSColor::labelColor(),
        );
        canvas::number(
            view,
            &ago(request.created_at, now),
            PAD + CONTENT - 16.0 - 100.0,
            y,
            100.0,
            11.0,
            REGULAR,
            &NSColor::secondaryLabelColor(),
        );
        y += 24.0;
        y += canvas::paragraph(
            view,
            &request.message,
            PAD + 16.0,
            y,
            inner,
            13.0,
            &NSColor::labelColor(),
        );
        y += 8.0;
        y += canvas::paragraph(
            view,
            &format!(
                "Reply in the original {} session · {}",
                request.provider, request.session
            ),
            PAD + 16.0,
            y,
            inner,
            11.0,
            &NSColor::secondaryLabelColor(),
        );
        y += 14.0;
        card.setFrame(rect(PAD, top, CONTENT, y - top));
        y += 10.0;
    }
    y + SECTION_GAP - 10.0
}

fn totals(
    view: &NSView,
    target: &ActivityTarget,
    data: &Monitoring,
    period: usize,
    mut y: f64,
) -> f64 {
    heading(view, "Tokens", y);
    let mtm = MainThreadMarker::from(view);
    let labels: Vec<Retained<NSString>> = PERIODS
        .iter()
        .map(|label| NSString::from_str(label))
        .collect();
    let control = unsafe {
        NSSegmentedControl::segmentedControlWithLabels_trackingMode_target_action(
            &NSArray::from_retained_slice(&labels),
            NSSegmentSwitchTracking::SelectOne,
            Some(target),
            Some(sel!(choosePeriod:)),
            mtm,
        )
    };
    control.setSelectedSegment(period as isize);
    control.setFrame(rect(PAD + CONTENT - 330.0, y - 3.0, 330.0, 24.0));
    view.addSubview(&control);
    y += 42.0;

    if !data.ready {
        canvas::text(
            view,
            "Reading session history…",
            PAD,
            y,
            CONTENT,
            13.0,
            REGULAR,
            &NSColor::secondaryLabelColor(),
        );
        return y + 24.0 + SECTION_GAP;
    }
    let providers = &data.tokens.providers;
    if providers.is_empty() {
        canvas::text(
            view,
            "No agents are switched on in Settings.",
            PAD,
            y,
            CONTENT,
            13.0,
            REGULAR,
            &NSColor::secondaryLabelColor(),
        );
        return y + 24.0 + SECTION_GAP;
    }

    let gap = 16.0;
    let columns = providers.len().max(1) as f64;
    let width = (CONTENT - gap * (columns - 1.0)) / columns;
    let mut tallest: f64 = 0.0;
    for (index, provider) in providers.iter().enumerate() {
        let x = PAD + index as f64 * (width + gap);
        let usage = select(provider, period);
        tallest = tallest.max(tile(view, &provider.provider, usage, x, y, width));
    }
    y + tallest + SECTION_GAP
}

/// One agent's total, then what that total is made of, in plain words.
fn tile(view: &NSView, name: &str, usage: &Period, x: f64, top: f64, width: f64) -> f64 {
    let card = canvas::fill(
        view,
        rect(x, top, width, 0.0),
        10.0,
        &NSColor::labelColor().colorWithAlphaComponent(0.05),
    );
    let inner = width - 32.0;
    let left = x + 16.0;
    let mut y = top + 14.0;

    dot(view, left, y + 5.0, &provider_color(name));
    canvas::text(
        view,
        name,
        left + 14.0,
        y,
        inner - 14.0,
        13.0,
        MEDIUM,
        &NSColor::secondaryLabelColor(),
    );
    y += 22.0;
    let tokens = usage.tokens;
    let total = canvas::text(
        view,
        &compact(tokens.total()),
        left,
        y,
        inner,
        30.0,
        SEMIBOLD,
        &NSColor::labelColor(),
    );
    total.setFont(Some(
        &objc2_app_kit::NSFont::monospacedDigitSystemFontOfSize_weight(30.0, SEMIBOLD),
    ));
    total.setToolTip(Some(&NSString::from_str(&format!(
        "{} tokens",
        group_digits(tokens.total())
    ))));
    y += 40.0;
    let requests = match usage.requests {
        1 => "1 response".to_string(),
        count => format!("{} responses", group_digits(count)),
    };
    canvas::text(
        view,
        &format!("tokens · {requests}"),
        left,
        y,
        inner,
        12.0,
        REGULAR,
        &NSColor::secondaryLabelColor(),
    );
    y += 28.0;

    for (label, value) in [("Input", tokens.input), ("Output", tokens.output)] {
        canvas::text(
            view,
            label,
            left,
            y,
            inner - 80.0,
            12.0,
            REGULAR,
            &NSColor::secondaryLabelColor(),
        );
        canvas::number(
            view,
            &compact(value),
            left + inner - 80.0,
            y,
            80.0,
            12.0,
            MEDIUM,
            &NSColor::labelColor(),
        )
        .setToolTip(Some(&NSString::from_str(&group_digits(value))));
        y += 20.0;
    }
    y += 14.0;
    card.setFrame(rect(x, top, width, y - top));
    y - top
}

fn daily_chart(view: &NSView, data: &Monitoring, mut y: f64) -> f64 {
    heading(view, "Last 14 days", y);
    let providers: Vec<&str> = data
        .tokens
        .providers
        .iter()
        .map(|p| p.provider.as_str())
        .collect();
    // Legend: identity is never carried by colour alone.
    let mut legend_x = PAD + CONTENT;
    for name in providers.iter().rev() {
        legend_x -= 70.0;
        dot(view, legend_x, y + 6.0, &provider_color(name));
        canvas::text(
            view,
            name,
            legend_x + 12.0,
            y + 1.0,
            56.0,
            12.0,
            REGULAR,
            &NSColor::secondaryLabelColor(),
        );
    }
    y += 34.0;

    let today = Local::now().date_naive();
    let days = daily_series(data, today);
    let peak = days
        .iter()
        .map(|(_, values)| values.values().sum::<u64>())
        .max()
        .unwrap_or(0);

    let label_width = 60.0;
    let plot_x = PAD + label_width;
    let plot_width = CONTENT - label_width;
    canvas::number(
        view,
        &compact(peak),
        PAD,
        y - 7.0,
        label_width - 8.0,
        11.0,
        REGULAR,
        &NSColor::tertiaryLabelColor(),
    );
    canvas::fill(
        view,
        rect(plot_x, y, plot_width, 1.0),
        0.0,
        &NSColor::quaternaryLabelColor(),
    );
    canvas::fill(
        view,
        rect(plot_x, y + CHART_HEIGHT, plot_width, 1.0),
        0.0,
        &NSColor::separatorColor(),
    );

    let slot = plot_width / CHART_DAYS as f64;
    let bar = (slot * 0.56).min(24.0);
    for (index, (date, values)) in days.iter().enumerate() {
        let x = plot_x + index as f64 * slot + (slot - bar) / 2.0;
        let mut top = y + CHART_HEIGHT;
        let mut tip = vec![date.format("%a, %b %-d").to_string()];
        for name in &providers {
            let value = values.get(*name).copied().unwrap_or(0);
            tip.push(format!("{name}  {}", compact(value)));
            if value == 0 || peak == 0 {
                continue;
            }
            let height = (CHART_HEIGHT * value as f64 / peak as f64).max(2.0);
            // A 2pt surface gap separates stacked segments.
            let gap = if top < y + CHART_HEIGHT { 2.0 } else { 0.0 };
            top -= height;
            canvas::fill(
                view,
                rect(x, top, bar, (height - gap).max(1.0)),
                3.0,
                &provider_color(name),
            );
        }
        canvas::hover(
            view,
            rect(plot_x + index as f64 * slot, y, slot, CHART_HEIGHT + 20.0),
            &tip.join("\n"),
        );
    }

    let label_y = y + CHART_HEIGHT + 6.0;
    let first = days
        .first()
        .map(|(date, _)| date.format("%b %-d").to_string())
        .unwrap_or_default();
    canvas::text(
        view,
        &first,
        plot_x,
        label_y,
        80.0,
        11.0,
        REGULAR,
        &NSColor::tertiaryLabelColor(),
    );
    canvas::number(
        view,
        "Today",
        plot_x + plot_width - 80.0,
        label_y,
        80.0,
        11.0,
        REGULAR,
        &NSColor::tertiaryLabelColor(),
    );
    y + CHART_HEIGHT + 24.0 + SECTION_GAP
}

fn by_model(view: &NSView, data: &Monitoring, period: usize, mut y: f64) -> f64 {
    heading(view, "By model", y);
    canvas::number(
        view,
        PERIODS[period],
        PAD + CONTENT - 160.0,
        y + 2.0,
        160.0,
        12.0,
        REGULAR,
        &NSColor::secondaryLabelColor(),
    );
    y += 32.0;

    let rows = models_in_period(data, period, Local::now().date_naive());
    if rows.is_empty() {
        canvas::text(
            view,
            "No usage in this period.",
            PAD,
            y,
            CONTENT,
            13.0,
            REGULAR,
            &NSColor::secondaryLabelColor(),
        );
        return y + 22.0 + SECTION_GAP;
    }
    let total: u64 = rows.iter().map(|row| row.2).sum();
    let share_width = 120.0;
    for (provider, model, tokens) in rows.iter().take(12) {
        dot(view, PAD, y + 6.0, &provider_color(provider));
        canvas::text(
            view,
            model,
            PAD + 14.0,
            y,
            CONTENT - 330.0,
            13.0,
            REGULAR,
            &NSColor::labelColor(),
        );
        canvas::text(
            view,
            provider,
            PAD + CONTENT - 316.0,
            y + 1.0,
            70.0,
            12.0,
            REGULAR,
            &NSColor::secondaryLabelColor(),
        );
        let share = if total == 0 {
            0.0
        } else {
            *tokens as f64 / total as f64
        };
        let bar_x = PAD + CONTENT - 236.0;
        canvas::fill(
            view,
            rect(bar_x, y + 6.0, share_width, 5.0),
            2.5,
            &NSColor::quaternaryLabelColor(),
        );
        if share > 0.0 {
            canvas::fill(
                view,
                rect(bar_x, y + 6.0, (share_width * share).max(4.0), 5.0),
                2.5,
                &provider_color(provider),
            );
        }
        canvas::number(
            view,
            &format!("{:.0}%", share * 100.0),
            bar_x + share_width,
            y,
            44.0,
            12.0,
            REGULAR,
            &NSColor::secondaryLabelColor(),
        );
        canvas::number(
            view,
            &compact(*tokens),
            PAD + CONTENT - 70.0,
            y,
            70.0,
            13.0,
            MEDIUM,
            &NSColor::labelColor(),
        )
        .setToolTip(Some(&NSString::from_str(&format!(
            "{} tokens",
            group_digits(*tokens)
        ))));
        y += 26.0;
    }
    y + SECTION_GAP - 8.0
}

fn footnote(view: &NSView, data: &Monitoring, mut y: f64) -> f64 {
    canvas::separator(view, PAD, y, CONTENT);
    y += 14.0;
    let mut note = format!(
        "Counted from {} session logs on this Mac. Weeks start on Monday.",
        group_digits(data.tokens.files as u64)
    );
    let problems = data.tokens.errors.len() + data.attention.errors.len();
    if problems > 0 {
        note.push_str(&format!(
            " {problems} {} could not be read.",
            if problems == 1 { "file" } else { "files" }
        ));
    }
    y + canvas::paragraph(
        view,
        &note,
        PAD,
        y,
        CONTENT,
        11.0,
        &NSColor::tertiaryLabelColor(),
    )
}

// --------------------------------------------------------------- components --

fn heading(view: &NSView, title: &str, y: f64) {
    canvas::text(
        view,
        title,
        PAD,
        y,
        240.0,
        15.0,
        SEMIBOLD,
        &NSColor::labelColor(),
    );
}

fn dot(view: &NSView, x: f64, y: f64, color: &NSColor) {
    canvas::fill(view, rect(x, y, 8.0, 8.0), 4.0, color);
}

// ------------------------------------------------------------------- shaping --

fn select(provider: &gauge::activity::ProviderStats, period: usize) -> &Period {
    match period {
        0 => &provider.today,
        1 => &provider.week,
        2 => &provider.month,
        _ => &provider.all_time,
    }
}

fn period_start(period: usize, today: NaiveDate) -> Option<NaiveDate> {
    match period {
        0 => Some(today),
        1 => Some(today - Duration::days(today.weekday().num_days_from_monday() as i64)),
        2 => today.with_day(1),
        _ => None,
    }
}

/// Every day of the last two weeks, including days with no use, so gaps in the
/// chart are real gaps rather than missing bars.
fn daily_series(data: &Monitoring, today: NaiveDate) -> Vec<(NaiveDate, BTreeMap<String, u64>)> {
    let first = today - Duration::days(CHART_DAYS - 1);
    let mut days: Vec<_> = (0..CHART_DAYS)
        .map(|offset| (first + Duration::days(offset), BTreeMap::new()))
        .collect();
    for row in &data.tokens.daily {
        let Ok(date) = NaiveDate::parse_from_str(&row.date, "%Y-%m-%d") else {
            continue;
        };
        if date < first || date > today {
            continue;
        }
        let slot = &mut days[(date - first).num_days() as usize].1;
        *slot.entry(row.provider.clone()).or_insert(0) += row.usage.tokens.total();
    }
    days
}

fn models_in_period(
    data: &Monitoring,
    period: usize,
    today: NaiveDate,
) -> Vec<(String, String, u64)> {
    let start = period_start(period, today);
    let mut totals = BTreeMap::<(String, String), u64>::new();
    for row in &data.tokens.daily {
        let Ok(date) = NaiveDate::parse_from_str(&row.date, "%Y-%m-%d") else {
            continue;
        };
        if start.is_some_and(|start| date < start) || date > today {
            continue;
        }
        *totals
            .entry((row.provider.clone(), row.model.clone()))
            .or_insert(0) += row.usage.tokens.total();
    }
    let mut rows: Vec<_> = totals
        .into_iter()
        .filter(|(_, tokens)| *tokens > 0)
        .map(|((provider, model), tokens)| (provider, model, tokens))
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.2));
    rows
}

fn ago(created_at: u64, now: u64) -> String {
    match now.saturating_sub(created_at) {
        0..=59 => "Just now".into(),
        seconds @ 60..=3_599 => format!("{}m ago", seconds / 60),
        seconds @ 3_600..=86_399 => format!("{}h ago", seconds / 3_600),
        seconds => format!("{}d ago", seconds / 86_400),
    }
}

fn group_digits(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauge::activity::Tokens;
    use gauge::activity::{DailyStats, TokenStats};

    fn day(date: &str, provider: &str, model: &str, input: u64) -> DailyStats {
        DailyStats {
            date: date.into(),
            provider: provider.into(),
            model: model.into(),
            usage: Period {
                tokens: Tokens {
                    input,
                    ..Tokens::default()
                },
                requests: 1,
            },
        }
    }

    fn data(daily: Vec<DailyStats>) -> Monitoring {
        Monitoring {
            tokens: TokenStats {
                daily,
                ..TokenStats::default()
            },
            ready: true,
            ..Monitoring::default()
        }
    }

    #[test]
    fn the_chart_has_a_bar_slot_for_every_day_even_empty_ones() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 17).unwrap();
        let days = daily_series(
            &data(vec![
                day("2026-09-17", "Claude", "opus", 10),
                day("2026-09-01", "Codex", "gpt", 5),
            ]),
            today,
        );
        assert_eq!(days.len(), 14);
        assert_eq!(days[13].1["Claude"], 10);
        assert!(days.iter().all(|(_, values)| !values.contains_key("Codex")));
    }

    #[test]
    fn models_are_limited_to_the_period_and_largest_first() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 17).unwrap(); // Thursday
        let monitoring = data(vec![
            day("2026-09-17", "Claude", "opus", 10),
            day("2026-09-14", "Codex", "gpt", 30),
            day("2026-09-13", "Codex", "gpt", 1_000),
        ]);
        let week = models_in_period(&monitoring, 1, today);
        assert_eq!(week[0], ("Codex".into(), "gpt".into(), 30));
        assert_eq!(week[1].2, 10);
        assert_eq!(models_in_period(&monitoring, 3, today)[0].2, 1_030);
    }

    #[test]
    fn large_numbers_are_grouped_for_tooltips() {
        assert_eq!(group_digits(151_764_866), "151,764,866");
        assert_eq!(group_digits(999), "999");
    }
}
