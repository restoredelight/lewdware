//! A small portal service on the regression runner's private D-Bus. This exercises the actual
//! ashpd request/response and signal code, without a compositor or user permission dialog.
use super::linux::run;
use crate::control::ControlMessage;
use shared::ipc::control::StopShortcutStatus as Status;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::{mpsc, watch};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

type Dict = HashMap<String, OwnedValue>;

struct MockPortal {
    bound: Arc<Mutex<Option<OwnedObjectPath>>>,
}

fn token(options: &Dict, name: &str) -> String {
    options[name].downcast_ref::<&str>().unwrap().to_string()
}

fn path(header: &zbus::message::Header<'_>, kind: &str, token: &str) -> OwnedObjectPath {
    let sender = header
        .sender()
        .unwrap()
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    format!("/org/freedesktop/portal/desktop/{kind}/{sender}/{token}")
        .try_into()
        .unwrap()
}

#[zbus::interface(name = "org.freedesktop.portal.GlobalShortcuts")]
impl MockPortal {
    #[zbus(property)]
    fn version(&self) -> u32 {
        1
    }

    async fn create_session(
        &self,
        options: Dict,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> OwnedObjectPath {
        let request = path(&header, "request", &token(&options, "handle_token"));
        let session = path(&header, "session", &token(&options, "session_handle_token"));
        let mut result = Dict::new();
        result.insert(
            "session_handle".into(),
            Value::from(session.as_str()).try_into().unwrap(),
        );
        connection
            .emit_signal(
                None::<&str>,
                &request,
                "org.freedesktop.portal.Request",
                "Response",
                &(0u32, result),
            )
            .await
            .unwrap();
        request
    }

    async fn bind_shortcuts(
        &self,
        session: OwnedObjectPath,
        shortcuts: Vec<(String, Dict)>,
        _parent: String,
        options: Dict,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> OwnedObjectPath {
        assert_eq!(shortcuts[0].0, "stop-immediately");
        assert_eq!(token(&shortcuts[0].1, "preferred_trigger"), "SHIFT+Escape");
        *self.bound.lock().unwrap() = Some(session);
        let request = path(&header, "request", &token(&options, "handle_token"));
        let mut info = Dict::new();
        info.insert(
            "description".into(),
            Value::from("Stop Lewdware").try_into().unwrap(),
        );
        info.insert(
            "trigger_description".into(),
            Value::from("Shift+Escape").try_into().unwrap(),
        );
        let mut result = Dict::new();
        result.insert(
            "shortcuts".into(),
            Value::from(vec![("stop-immediately".to_string(), info)])
                .try_into()
                .unwrap(),
        );
        connection
            .emit_signal(
                None::<&str>,
                &request,
                "org.freedesktop.portal.Request",
                "Response",
                &(0u32, result),
            )
            .await
            .unwrap();
        request
    }
}

#[tokio::test]
#[ignore = "run by isolated desktop regression runner"]
async fn isolated_regression_portal() {
    assert!(std::env::var_os("LEWDWARE_ISOLATED_TEST").is_some());
    let bound = Arc::new(Mutex::new(None));
    let service = zbus::connection::Builder::session()
        .unwrap()
        .name("org.freedesktop.portal.Desktop")
        .unwrap()
        .serve_at(
            "/org/freedesktop/portal/desktop",
            MockPortal {
                bound: bound.clone(),
            },
        )
        .unwrap()
        .build()
        .await
        .unwrap();
    let (tx, mut rx) = mpsc::channel(32);
    let (key_tx, keys) = watch::channel(shared::user_config::AppConfig::default().stop_button);
    let (_configure, configure_rx) = mpsc::channel(1);
    let task = tokio::spawn(run(tx, keys, configure_rx));
    let registered = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match rx.recv().await.unwrap() {
                ControlMessage::StopShortcutChanged(Status::Registered {
                    backend,
                    trigger,
                    managed_by_desktop,
                    ..
                }) => {
                    assert_eq!(backend, "Desktop portal");
                    assert_eq!(trigger, "Shift+Escape");
                    assert!(managed_by_desktop);
                    break;
                }
                ControlMessage::StopShortcutChanged(Status::Failed { message }) => {
                    panic!("{message}")
                }
                _ => {}
            }
        }
    })
    .await;
    registered.expect("portal registration did not complete");
    let session = bound.lock().unwrap().clone().unwrap();
    let unrelated: OwnedObjectPath = "/unrelated/session".try_into().unwrap();
    for (session, id) in [
        (&unrelated, "stop-immediately"),
        (&session, "other"),
        (&session, "stop-immediately"),
    ] {
        service
            .emit_signal(
                None::<&str>,
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.GlobalShortcuts",
                "Activated",
                &(session, id, 0u64, Dict::new()),
            )
            .await
            .unwrap();
    }
    let message = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(message, ControlMessage::StopKeyPressed));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
            .await
            .is_err(),
        "unrelated activations must be ignored"
    );
    service
        .emit_signal(
            None::<&str>,
            &session,
            "org.freedesktop.portal.Session",
            "Closed",
            &Dict::new(),
        )
        .await
        .unwrap();
    let message = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            message,
            ControlMessage::StopShortcutChanged(Status::Failed { .. })
        ),
        "session revocation must be visible"
    );
    drop(key_tx);
    task.await.unwrap();
}
