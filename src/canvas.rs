//! The few drawing primitives Gauge's popover and windows share.
//!
//! Everything is placed in a flipped view, top-down, so a panel measures itself
//! as it is built.

use objc2::{define_class, msg_send, rc::Retained, MainThreadOnly};
use objc2_app_kit::{
    NSBox, NSBoxType, NSColor, NSFont, NSLineBreakMode, NSTextAlignment, NSTextField,
    NSTitlePosition, NSView,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

// AppKit's NSFontWeight constants are extern statics; these are their values.
pub const REGULAR: f64 = 0.0;
pub const MEDIUM: f64 = 0.23;
pub const SEMIBOLD: f64 = 0.3;

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    pub struct FlippedView;

    impl FlippedView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl FlippedView {
    pub fn new(width: f64, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm);
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, 0.0));
        unsafe { msg_send![this, initWithFrame: frame] }
    }
}

pub fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

/// One line of text that truncates rather than wraps.
#[allow(clippy::too_many_arguments)]
pub fn text(
    view: &NSView,
    text: &str,
    x: f64,
    y: f64,
    width: f64,
    size: f64,
    weight: f64,
    color: &NSColor,
) -> Retained<NSTextField> {
    let mtm = MainThreadMarker::from(view);
    let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.setFrame(rect(x, y, width, size + 5.0));
    field.setFont(Some(&NSFont::systemFontOfSize_weight(size, weight)));
    field.setTextColor(Some(color));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.setToolTip(Some(&NSString::from_str(text)));
    view.addSubview(&field);
    field
}

/// Right-aligned tabular digits, so a column of numbers lines up by place.
#[allow(clippy::too_many_arguments)]
pub fn number(
    view: &NSView,
    value: &str,
    x: f64,
    y: f64,
    width: f64,
    size: f64,
    weight: f64,
    color: &NSColor,
) -> Retained<NSTextField> {
    let field = self::text(view, value, x, y, width, size, weight, color);
    field.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(
        size, weight,
    )));
    field.setAlignment(NSTextAlignment::Right);
    field
}

/// Text that wraps to `width`. Returns the height it took.
#[allow(clippy::too_many_arguments)]
pub fn paragraph(
    view: &NSView,
    text: &str,
    x: f64,
    y: f64,
    width: f64,
    size: f64,
    color: &NSColor,
) -> f64 {
    let mtm = MainThreadMarker::from(view);
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(&NSFont::systemFontOfSize(size)));
    field.setTextColor(Some(color));
    field.setSelectable(true);
    field.setPreferredMaxLayoutWidth(width);
    let fitting: NSSize = unsafe { msg_send![&field, fittingSize] };
    let height = fitting.height.ceil();
    field.setFrame(rect(x, y, width, height));
    view.addSubview(&field);
    height
}

pub fn fill(view: &NSView, frame: NSRect, radius: f64, color: &NSColor) -> Retained<NSBox> {
    let mtm = MainThreadMarker::from(view);
    let shape = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
    shape.setBoxType(NSBoxType::Custom);
    shape.setTitlePosition(NSTitlePosition::NoTitle);
    shape.setBorderWidth(0.0);
    shape.setCornerRadius(radius);
    shape.setFillColor(color);
    shape.setContentViewMargins(NSSize::new(0.0, 0.0));
    view.addSubview(&shape);
    shape
}

pub fn separator(view: &NSView, x: f64, y: f64, width: f64) {
    let mtm = MainThreadMarker::from(view);
    let line = NSBox::initWithFrame(NSBox::alloc(mtm), rect(x, y, width, 1.0));
    line.setBoxType(NSBoxType::Separator);
    view.addSubview(&line);
}

/// An empty view that only carries a tooltip, laid over a mark so the hover
/// target is larger than the mark itself.
pub fn hover(view: &NSView, frame: NSRect, tip: &str) {
    let mtm = MainThreadMarker::from(view);
    let area = NSView::initWithFrame(NSView::alloc(mtm), frame);
    area.setToolTip(Some(&NSString::from_str(tip)));
    view.addSubview(&area);
}

/// Each agent keeps one colour everywhere Gauge draws it. The pair is checked
/// for colour-blind separation and contrast in both light and dark mode.
pub fn provider_color(provider: &str) -> Retained<NSColor> {
    let (red, green, blue) = match provider {
        "Claude" => (217.0, 119.0, 6.0),
        "Codex" => (59.0, 130.0, 246.0),
        "Cursor" => (5.0, 150.0, 105.0),
        _ => return NSColor::secondaryLabelColor(),
    };
    NSColor::colorWithSRGBRed_green_blue_alpha(red / 255.0, green / 255.0, blue / 255.0, 1.0)
}
