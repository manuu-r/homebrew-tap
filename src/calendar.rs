#[cfg(target_os = "macos")]
mod macos {
    use crate::config::CalendarConfig;
    use block2::RcBlock;
    use objc2::{runtime::Bool, sel};
    use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore};
    use objc2_foundation::{NSDate, NSError, NSObjectProtocol};

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
                request_full_access();
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

    fn request_full_access() {
        let store = unsafe { EKEventStore::new() };
        // Keep the event store alive until EventKit invokes the copied block.
        let retained_store = store.clone();
        let completion: RcBlock<dyn Fn(Bool, *mut NSError)> = RcBlock::new(move |_, _| {
            let _keep_alive = &retained_store;
        });

        if store.respondsToSelector(sel!(requestFullAccessToEventsWithCompletion:)) {
            unsafe {
                store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&completion));
            }
        } else {
            // macOS 13 uses the earlier request API. Both paths ask for read
            // access to events; macOS 14+ names that level Full Access.
            #[allow(deprecated)]
            unsafe {
                store.requestAccessToEntityType_completion(
                    EKEntityType::Event,
                    RcBlock::as_ptr(&completion),
                );
            }
        }
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
