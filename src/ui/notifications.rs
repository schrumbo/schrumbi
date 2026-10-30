use gpui_kit::{App, AppContext};

use super::state::{self, act};
use crate::core::library::Reminder;
use crate::core::model::Status;

/// What the system reported about reminders.
pub enum Event {
    /// The system allows notifications, so it can schedule them itself.
    Allowed,
    /// The user clicked a reminder or its "uploaded" action.
    Response { assignment_id: String, uploaded: bool },
}

/// Starts listening for clicks on reminders.
pub fn init(cx: &mut App) {
    let (tx, rx) = async_channel::unbounded::<Event>();
    platform::init(tx);
    cx.spawn(async move |cx| {
        while let Ok(event) = rx.recv().await {
            cx.update(|cx| match event {
                Event::Allowed => state::reschedule_reminders(cx),
                Event::Response { assignment_id, uploaded: true } => act(cx, |l| l.set_status(Some(Status::Uploaded), &assignment_id, true)),
                Event::Response { assignment_id, uploaded: false } => {
                    act(cx, |l| l.select(Some(assignment_id.clone())));
                    super::main_window::show(cx);
                }
            });
        }
    })
    .detach();
}

/// Replaces all pending reminders. Returns the ones the app has to fire itself while running.
pub fn schedule(reminders: &[Reminder]) -> Vec<Reminder> {
    if platform::schedule(reminders) { Vec::new() } else { reminders.to_vec() }
}

/// Shows a reminder now, off the main thread.
pub fn show(reminder: Reminder, cx: &mut App) {
    cx.background_spawn(async move { platform::show(reminder) }).detach();
}

#[cfg(target_os = "macos")]
mod platform {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};

    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AllocAnyThread, define_class, msg_send};
    use objc2_foundation::{NSArray, NSBundle, NSError, NSSet, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction, UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions, UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse, UNNotificationSound, UNTimeIntervalNotificationTrigger, UNUserNotificationCenter,
        UNUserNotificationCenterDelegate,
    };

    use super::Event;
    use crate::core::i18n::tr;
    use crate::core::library::Reminder;

    const CATEGORY: &str = "deadline";
    const UPLOADED: &str = "uploaded";

    static RESPONSES: OnceLock<async_channel::Sender<Event>> = OnceLock::new();
    static ALLOWED: AtomicBool = AtomicBool::new(false);

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "SchrumbiNotificationDelegate"]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(&self, _: &UNUserNotificationCenter, _: &UNNotification, handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>) {
                handler.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::Sound,));
            }

            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(&self, _: &UNUserNotificationCenter, response: &UNNotificationResponse, handler: &DynBlock<dyn Fn()>) {
                let assignment_id = response.notification().request().content().threadIdentifier().to_string();
                let uploaded = response.actionIdentifier().to_string() == UPLOADED;
                if let Some(tx) = RESPONSES.get() {
                    let _ = tx.try_send(Event::Response { assignment_id, uploaded });
                }
                handler.call(());
            }
        }
    );

    /// The notification center aborts outside an app bundle, so `cargo run` falls back to `notify-rust`.
    fn center() -> Option<Retained<UNUserNotificationCenter>> {
        NSBundle::mainBundle().bundleIdentifier()?;
        Some(UNUserNotificationCenter::currentNotificationCenter())
    }

    pub fn init(tx: async_channel::Sender<Event>) {
        let _ = RESPONSES.set(tx);
        let Some(center) = center() else { return };
        let delegate: Retained<Delegate> = unsafe { msg_send![super(Delegate::alloc().set_ivars(())), init] };
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        std::mem::forget(delegate);
        let action = UNNotificationAction::actionWithIdentifier_title_options(&NSString::from_str(UPLOADED), &NSString::from_str(&tr("Als hochgeladen markieren")), UNNotificationActionOptions::empty());
        let category = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(&NSString::from_str(CATEGORY), &NSArray::from_retained_slice(&[action]), &NSArray::new(), UNNotificationCategoryOptions::empty());
        center.setNotificationCategories(&NSSet::from_retained_slice(&[category]));
        let done = RcBlock::new(|allowed: objc2::runtime::Bool, _: *mut NSError| {
            if allowed.as_bool() {
                ALLOWED.store(true, Ordering::SeqCst);
                if let Some(tx) = RESPONSES.get() {
                    let _ = tx.try_send(Event::Allowed);
                }
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge, &done);
    }

    fn content(r: &Reminder) -> Retained<UNMutableNotificationContent> {
        let c = UNMutableNotificationContent::new();
        c.setTitle(&NSString::from_str(&r.title));
        c.setBody(&NSString::from_str(&r.body));
        c.setThreadIdentifier(&NSString::from_str(&r.assignment_id));
        c.setSound(Some(&UNNotificationSound::defaultSound()));
        if r.upload {
            c.setCategoryIdentifier(&NSString::from_str(CATEGORY));
        }
        c
    }

    /// Hands every reminder to the system, so they also fire while the app is closed. False until the system allows it.
    pub fn schedule(reminders: &[Reminder]) -> bool {
        if !ALLOWED.load(Ordering::SeqCst) {
            return false;
        }
        let Some(center) = center() else { return false };
        center.removeAllPendingNotificationRequests();
        for r in reminders {
            let seconds = (r.fire - chrono::Local::now()).num_seconds();
            if seconds < 1 {
                continue;
            }
            let trigger = UNTimeIntervalNotificationTrigger::triggerWithTimeInterval_repeats(seconds as f64, false);
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&r.id), &content(r), Some(&trigger));
            center.addNotificationRequest_withCompletionHandler(&request, None);
        }
        true
    }

    pub fn show(r: Reminder) {
        let _ = notify_rust::Notification::new().appname("Schrumbi").summary(&r.title).body(&r.body).show();
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use std::sync::OnceLock;

    use super::Event;
    use crate::core::i18n::tr;
    use crate::core::library::Reminder;

    static RESPONSES: OnceLock<async_channel::Sender<Event>> = OnceLock::new();

    pub fn init(tx: async_channel::Sender<Event>) {
        let _ = RESPONSES.set(tx);
    }

    pub fn schedule(_: &[Reminder]) -> bool {
        false
    }

    pub fn show(r: Reminder) {
        let mut n = notify_rust::Notification::new();
        n.appname("Schrumbi").summary(&r.title).body(&r.body).action("default", &tr("Öffnen"));
        if r.upload {
            n.action("uploaded", &tr("Als hochgeladen markieren"));
        }
        let Ok(handle) = n.show() else { return };
        handle.wait_for_action(|action| {
            let uploaded = action == "uploaded";
            if (uploaded || action == "default")
                && let Some(tx) = RESPONSES.get()
            {
                let _ = tx.try_send(Event::Response { assignment_id: r.assignment_id.clone(), uploaded });
            }
        });
    }
}

#[cfg(windows)]
mod platform {
    use super::Event;
    use crate::core::library::Reminder;

    pub fn init(_: async_channel::Sender<Event>) {}

    pub fn schedule(_: &[Reminder]) -> bool {
        false
    }

    pub fn show(r: Reminder) {
        let _ = notify_rust::Notification::new().appname("Schrumbi").summary(&r.title).body(&r.body).show();
    }
}
