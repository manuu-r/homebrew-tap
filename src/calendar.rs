#[cfg(target_os = "macos")]
mod macos {
    use crate::config::CalendarConfig;
    use objc2::AnyThread;
    use objc2_event_kit::{EKAuthorizationStatus, EKEntityMask, EKEntityType, EKEventStore};
    use objc2_foundation::NSDate;

    #[derive(Clone, Debug)]
    pub struct CalendarEvent {
        pub title: String,
        pub starts_at: f64,
        pub ends_at: f64,
        pub all_day: bool,
    }

    pub fn fetch(config: &CalendarConfig) -> Result<Vec<CalendarEvent>, String> {
        if !config.enabled {
            return Ok(Vec::new());
        }

        let status = unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) };
        if status != EKAuthorizationStatus::FullAccess {
            if status == EKAuthorizationStatus::NotDetermined {
                // This legacy initializer is the only EventKit API that starts
                // the macOS permission prompt without requiring a callback.
                #[allow(deprecated)]
                unsafe {
                    let _ = EKEventStore::initWithAccessToEntityTypes(
                        EKEventStore::alloc(),
                        EKEntityMask::Event,
                    );
                }
                return Err("Full Calendar access requested; allow Gauge in the macOS prompt, then it will refresh automatically".into());
            }
            return Err("Calendar access is not allowed; enable it in System Settings > Privacy & Security > Calendars".into());
        }

        let store = unsafe { EKEventStore::new() };
        let start = NSDate::date();
        let end = NSDate::dateWithTimeIntervalSinceNow((config.look_ahead_hours * 3600) as f64);
        let predicate =
            unsafe { store.predicateForEventsWithStartDate_endDate_calendars(&start, &end, None) };
        let events = unsafe { store.eventsMatchingPredicate(&predicate) };
        let mut result = Vec::new();

        for event in events.iter() {
            let calendar_name = unsafe {
                event
                    .calendar()
                    .map(|calendar| calendar.title().to_string())
                    .unwrap_or_default()
            };
            if !config.calendar_names.is_empty()
                && !config
                    .calendar_names
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&calendar_name))
            {
                continue;
            }
            result.push(CalendarEvent {
                title: unsafe { event.title().to_string() },
                starts_at: unsafe { event.startDate().timeIntervalSince1970() },
                ends_at: unsafe { event.endDate().timeIntervalSince1970() },
                all_day: unsafe { event.isAllDay() },
            });
        }
        result.sort_by(|left, right| left.starts_at.total_cmp(&right.starts_at));
        result.truncate(config.max_events);
        Ok(result)
    }
}

#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(not(target_os = "macos"))]
#[derive(Clone, Debug)]
pub struct CalendarEvent {
    pub title: String,
    pub starts_at: f64,
    pub ends_at: f64,
    pub all_day: bool,
}

#[cfg(not(target_os = "macos"))]
pub fn fetch(_: &crate::config::CalendarConfig) -> Result<Vec<CalendarEvent>, String> {
    Err("Calendar integration is available on macOS only".into())
}
