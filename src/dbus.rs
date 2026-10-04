//! D-Bus control surface via [`ime_control`] (`org.inputmethod.Control1`).

use cosmic::iced::futures::Stream;
use ime_control::{self, BusIdentity, ControlEvent};

use crate::Message;

pub const IDENTITY: BusIdentity = BusIdentity {
    well_known_name: "com.chewingwl.InputMethod1",
    object_path: "/com/chewingwl/InputMethod1",
};

/// Subscription that registers D-Bus and forwards control events as [`Message`]s.
pub fn dbus_subscription() -> impl Stream<Item = Message> {
    cosmic::iced::stream::channel(10, async |mut sender| {
        use cosmic::iced::futures::SinkExt;

        let mut rx = ime_control::serve(
            IDENTITY,
            "中",
            ime_control::menu_cn_en_half_full_settings(false),
            "chewingwl --settings",
        )
        .await;

        while let Some(ev) = rx.recv().await {
            let msg = match ev {
                ControlEvent::Activate(id) => match id.as_str() {
                    ime_control::action::TOGGLE_MODE => Message::DbusToggleMode,
                    ime_control::action::HALF_FULL => Message::DbusToggleHalfFullWidth,
                    ime_control::action::SETTINGS => Message::DbusOpenSettings,
                    other => {
                        log::debug!("chewingwl: ignore unknown Activate({other})");
                        continue;
                    }
                },
                ControlEvent::SetPassthrough(p) => Message::DbusSetPassthrough(p),
            };
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    })
}
