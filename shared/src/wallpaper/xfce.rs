//! Xfdesktop's current connector/workspace settings, including implicit defaults.
//!
//! Listing xfconf alone is insufficient: a stock desktop may contain only obsolete numeric
//! monitor keys. Xfdesktop 4.20 uses RandR connector names and its selected workspace policy.
use std::{collections::BTreeSet, env, path::Path};

use anyhow::{Context, Result, bail};
use x11rb::{
    connection::Connection,
    protocol::{
        randr::{self, ConnectionExt as _},
        xproto::{AtomEnum, ConnectionExt as _},
    },
};

use super::{run_cmd, stdout_of, stdout_of_raw};

const SINGLE_MODE: &str = "/backdrop/single-workspace-mode";
const SINGLE_NUMBER: &str = "/backdrop/single-workspace-number";
type Entries = Vec<(String, Option<String>)>;

trait Settings {
    fn list(&self) -> Result<BTreeSet<String>>;
    fn get(&self, property: &str) -> Result<String>;
    fn put(&self, property: &str, value: &str) -> Result<()>;
    fn reset(&self, property: &str) -> Result<()>;
}

struct Xfconf;
impl Settings for Xfconf {
    fn list(&self) -> Result<BTreeSet<String>> {
        Ok(stdout_of("xfconf-query", &["-c", "xfce4-desktop", "-l"])?
            .lines()
            .map(str::to_owned)
            .collect())
    }
    fn get(&self, property: &str) -> Result<String> {
        let out = stdout_of_raw("xfconf-query", &["-c", "xfce4-desktop", "-p", property])?;
        // Remove the CLI's one terminating newline, preserving whitespace in filenames.
        Ok(out.strip_suffix('\n').unwrap_or(&out).to_owned())
    }
    fn put(&self, property: &str, value: &str) -> Result<()> {
        let kind = property_type(property)?;
        if kind == "int" {
            value.parse::<i32>().context("invalid Xfce image style")?;
        }
        run_cmd(
            "xfconf-query",
            &[
                "-c",
                "xfce4-desktop",
                "-p",
                property,
                "--create",
                "--type",
                kind,
                "--set",
                value,
            ],
        )
    }
    fn reset(&self, property: &str) -> Result<()> {
        run_cmd(
            "xfconf-query",
            &["-c", "xfce4-desktop", "-p", property, "--reset"],
        )
    }
}

fn property_type(property: &str) -> Result<&'static str> {
    if property.ends_with("/last-image") {
        Ok("string")
    } else if property.ends_with("/image-style") {
        Ok("int")
    } else {
        bail!("not an Xfce wallpaper property: {property}")
    }
}

fn plan(screens: &[(usize, Vec<String>, u32)], single: Option<u32>) -> Result<Vec<String>> {
    let mut properties = BTreeSet::new();
    for (screen, connectors, count) in screens {
        if connectors.is_empty() {
            continue;
        }
        if *count == 0 || *count > 1024 {
            bail!("invalid Xfce workspace count: {count}");
        }
        if single.is_some_and(|workspace| workspace >= *count) {
            bail!("Xfce shared wallpaper workspace is outside the workspace list");
        }
        for connector in connectors {
            if connector.is_empty() || connector.contains('/') {
                bail!("invalid Xfce connector name");
            }
            for workspace in single.map(|w| w..w + 1).unwrap_or(0..*count) {
                let prefix =
                    format!("/backdrop/screen{screen}/monitor{connector}/workspace{workspace}");
                properties.insert(format!("{prefix}/last-image"));
                properties.insert(format!("{prefix}/image-style"));
            }
        }
    }
    if properties.is_empty() {
        bail!("no connected Xfce desktop outputs");
    }
    Ok(properties.into_iter().collect())
}

pub(super) fn targets() -> Result<Vec<String>> {
    if env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland") {
        bail!("Xfce Wayland output discovery is not supported");
    }
    let existing = Xfconf.list()?;
    let single = if existing.contains(SINGLE_MODE) {
        match Xfconf.get(SINGLE_MODE)?.as_str() {
            "true" => Some(if existing.contains(SINGLE_NUMBER) {
                Xfconf
                    .get(SINGLE_NUMBER)?
                    .parse()
                    .context("invalid Xfce shared workspace")?
            } else {
                0
            }),
            "false" => None,
            _ => bail!("invalid Xfce single-workspace-mode"),
        }
    } else {
        None
    };
    let (conn, _) = x11rb::connect(None).context("could not inspect Xfce's X11 desktop")?;
    let atom = conn
        .intern_atom(false, b"_NET_NUMBER_OF_DESKTOPS")?
        .reply()?
        .atom;
    let mut screens = Vec::new();
    for (index, screen) in conn.setup().roots.iter().enumerate() {
        let resources = conn
            .randr_get_screen_resources_current(screen.root)?
            .reply()?;
        let mut connectors = Vec::new();
        for output in resources.outputs {
            let info = conn
                .randr_get_output_info(output, resources.config_timestamp)?
                .reply()?;
            if info.connection == randr::Connection::CONNECTED && info.crtc != x11rb::NONE {
                connectors
                    .push(String::from_utf8(info.name).context("invalid Xfce connector encoding")?);
            }
        }
        if connectors.is_empty() {
            continue;
        }
        let reply = conn
            .get_property(false, screen.root, atom, AtomEnum::CARDINAL, 0, 1)?
            .reply()?;
        let count = reply
            .value32()
            .and_then(|mut values| values.next())
            .context("Xfce's window manager did not report its workspace count")?;
        screens.push((index, connectors, count));
    }
    plan(&screens, single)
}

fn capture(store: &impl Settings, targets: Vec<String>) -> Result<Entries> {
    let existing = store.list()?;
    targets
        .into_iter()
        .map(|property| {
            let value = if existing.contains(&property) {
                Some(store.get(&property)?)
            } else {
                None
            };
            if property_type(&property)? == "int"
                && let Some(value) = &value
            {
                value.parse::<i32>().context("invalid Xfce image style")?;
            }
            Ok((property, value))
        })
        .collect()
}

pub(super) fn snapshot() -> Result<Entries> {
    capture(&Xfconf, targets()?)
}

fn apply<'a>(
    store: &impl Settings,
    path: &str,
    properties: impl Iterator<Item = &'a str>,
) -> Result<()> {
    let properties = properties.collect::<Vec<_>>();
    if properties.is_empty() {
        bail!("no captured Xfce wallpaper targets");
    }
    for property in properties {
        let value = if property_type(property)? == "string" {
            path
        } else {
            "5"
        };
        store.put(property, value)?;
        if store.get(property)? != value {
            bail!("Xfce did not accept wallpaper property {property}");
        }
    }
    Ok(())
}

pub(super) fn set<'a>(path: &Path, properties: impl Iterator<Item = &'a str>) -> Result<()> {
    apply(
        &Xfconf,
        path.to_str().context("wallpaper path is not valid UTF-8")?,
        properties,
    )
}

fn put_back(store: &impl Settings, entries: &Entries) -> Result<()> {
    let existing = store.list()?;
    // Attempt all captured values even if an individual restore fails; retain the snapshot on
    // any failure so recovery can retry. Never reset an entire subtree of somebody's settings.
    let mut first_error = None;
    for (property, value) in entries {
        let result = match value {
            Some(value) => store.put(property, value),
            None if existing.contains(property) => store.reset(property),
            None => Ok(()),
        };
        if let Err(error) = result {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

pub(super) fn restore(entries: &Entries) -> Result<()> {
    put_back(&Xfconf, entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::BTreeMap};
    #[derive(Default)]
    struct Memory(RefCell<BTreeMap<String, String>>);
    impl Settings for Memory {
        fn list(&self) -> Result<BTreeSet<String>> {
            Ok(self.0.borrow().keys().cloned().collect())
        }
        fn get(&self, p: &str) -> Result<String> {
            self.0.borrow().get(p).cloned().context("missing")
        }
        fn put(&self, p: &str, v: &str) -> Result<()> {
            self.0.borrow_mut().insert(p.into(), v.into());
            Ok(())
        }
        fn reset(&self, p: &str) -> Result<()> {
            self.0.borrow_mut().remove(p);
            Ok(())
        }
    }
    #[test]
    fn stock_desktop_ignores_legacy_keys_and_restores_absent_properties() {
        let store = Memory::default();
        store
            .put("/backdrop/screen0/monitor0/last-image", "/default")
            .unwrap();
        let original = store.0.borrow().clone();
        let keys = plan(&[(0, vec!["Virtual-1".into()], 4)], None).unwrap();
        assert_eq!(keys.len(), 8);
        let snapshot = capture(&store, keys).unwrap();
        assert!(snapshot.iter().all(|(_, value)| value.is_none()));
        apply(
            &store,
            "/A ' 雪.png",
            snapshot.iter().map(|(p, _)| p.as_str()),
        )
        .unwrap();
        apply(&store, "/B.png", snapshot.iter().map(|(p, _)| p.as_str())).unwrap();
        // A newly connected output must not be overwritten by restore.
        let new_output = "/backdrop/screen0/monitorHDMI-1/workspace0/last-image";
        store.put(new_output, "/new-output.png").unwrap();
        put_back(&store, &snapshot).unwrap();
        put_back(&store, &snapshot).unwrap(); // recovery is idempotent
        let mut expected = original;
        expected.insert(new_output.into(), "/new-output.png".into());
        assert_eq!(*store.0.borrow(), expected);
    }
    #[test]
    fn configured_and_missing_values_round_trip_without_losing_empty_strings() {
        let store = Memory::default();
        let keys = plan(&[(0, vec!["eDP-1".into(), "HDMI-1".into()], 4)], Some(2)).unwrap();
        assert_eq!(keys.len(), 4);
        assert!(keys.iter().all(|key| key.contains("/workspace2/")));
        store.put(&keys[0], "0").unwrap(); // image-style
        store.put(&keys[1], "").unwrap(); // explicitly empty last-image
        let original = store.0.borrow().clone();
        let snapshot = capture(&store, keys).unwrap();
        apply(&store, "/A.png", snapshot.iter().map(|(p, _)| p.as_str())).unwrap();
        put_back(&store, &snapshot).unwrap();
        assert_eq!(*store.0.borrow(), original);
    }
    #[test]
    fn rejects_unknown_topology_and_keeps_legacy_recovery_snapshots_readable() {
        assert!(plan(&[], None).is_err());
        assert!(plan(&[(0, vec!["Virtual-1".into()], 0)], None).is_err());
        assert!(plan(&[(0, vec!["Virtual-1".into()], 4)], Some(4)).is_err());
        let old = r#"{"backend":"xfce","entries":[["/backdrop/screen0/monitor0/last-image","/original.png"]]}"#;
        let super::super::Snapshot::Xfce { entries } = serde_json::from_str(old).unwrap() else {
            panic!()
        };
        assert_eq!(entries[0].1.as_deref(), Some("/original.png"));
    }

    #[test]
    fn read_errors_are_not_absent_defaults_and_ignored_writes_are_not_success() {
        struct Faulty(Memory, bool);
        impl Settings for Faulty {
            fn list(&self) -> Result<BTreeSet<String>> {
                self.0.list()
            }
            fn get(&self, property: &str) -> Result<String> {
                if self.1 {
                    bail!("settings service unavailable");
                }
                self.0.get(property)
            }
            fn put(&self, _: &str, _: &str) -> Result<()> {
                Ok(())
            }
            fn reset(&self, _: &str) -> Result<()> {
                Ok(())
            }
        }
        let mut store = Faulty(Memory::default(), true);
        let key = "/backdrop/screen0/monitorVirtual-1/workspace0/last-image";
        store.0.put(key, "/original.png").unwrap();
        assert!(capture(&store, vec![key.into()]).is_err());
        store.1 = false;
        assert!(apply(&store, "/A.png", [key].into_iter()).is_err());
    }

    #[test]
    fn failed_restore_still_restores_the_other_properties() {
        struct Faulty(Memory);
        impl Settings for Faulty {
            fn list(&self) -> Result<BTreeSet<String>> {
                self.0.list()
            }
            fn get(&self, p: &str) -> Result<String> {
                self.0.get(p)
            }
            fn put(&self, _: &str, _: &str) -> Result<()> {
                bail!("write failed")
            }
            fn reset(&self, p: &str) -> Result<()> {
                self.0.reset(p)
            }
        }
        let keys = plan(&[(0, vec!["Virtual-1".into()], 1)], None).unwrap();
        let store = Faulty(Memory::default());
        for key in &keys {
            store.0.put(key, "temporary").unwrap();
        }
        let entries = vec![(keys[0].clone(), Some("3".into())), (keys[1].clone(), None)];
        assert!(put_back(&store, &entries).is_err());
        assert!(!store.0.list().unwrap().contains(&keys[1]));
    }
}
