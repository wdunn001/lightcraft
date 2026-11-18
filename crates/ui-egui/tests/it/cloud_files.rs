//! The real `craft_ui::cloud` contract (storytold/craft-libs#160) and the gating that keeps every
//! cloud affordance (File menu items, the top-bar account badge, the Open/Save dialogs) out of the
//! application until a backend is attached.
//!
//! Nothing in this repository calls `craft_ui::cloud::CloudFiles::connect` yet (no storytold crate
//! implements `craft_cloud::Provider`, or an application-specific client, for a real service), so
//! the "no backend" tests below describe the application as it actually ships; the mock-backed
//! tests exercise the contract a real backend will eventually drive.

use std::time::Duration;

use craft_cloud::{Capability, Entry, EntryKind};
use craft_ui::cloud::{self as cloud, Account, CloudBackend, Event, Fault, Job, Post};
use lightcraft_catalog::{Op, Photo, PhotoId, Source};
use lightcraft_ui_egui::headless::Headless;
use lightcraft_ui_egui::menus::menubar::{MenuNode, menu_bar};
use lightcraft_ui_egui::menus::ui_enabled;
use lightcraft_ui_egui::state::Dialog;
use lightcraft_ui_egui::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn entry(id: &str, name: &str, kind: EntryKind, writable: bool) -> Entry {
    Entry {
        id: id.to_owned(),
        room: id.to_owned(),
        name: name.to_owned(),
        path: if id.is_empty() { String::new() } else { format!("/{name}") },
        kind,
        size: Some(1234),
        modified: None,
        mime: None,
        etag: Some("v1".into()),
        writable,
        shared_by: None,
    }
}

/// Answers every job synchronously and in-process: no thread, no socket, no provider.
struct MockBackend;

impl CloudBackend for MockBackend {
    fn submit(&self, job: Job, post: Post) {
        match job {
            Job::Account => {
                let account =
                    Account { display: "mock@cloud.test".into(), capabilities: vec![Capability::Files, Capability::Etags], provider_linked: true };
                post.send(Event::Account(Ok(account)));
            }
            Job::List(path) => {
                let entries = vec![entry("folder-1", "Photos", EntryKind::Folder, true), entry("a.jpg", "a.jpg", EntryKind::File, true)];
                post.send(Event::Listed { path, entries: Ok(entries) });
            }
            Job::SharedWithMe => post.send(Event::Shared(Ok(Vec::new()))),
            Job::Read { entry, into } => post.send(Event::Read { entry, into, bytes: Ok(vec![0xFF, 0xD8, 0xFF, 0xE0]) }),
            Job::Write { uid, entry, bytes } => {
                let mut saved = entry;
                saved.size = Some(bytes.len() as u64);
                post.send(Event::Saved { uid, entry: Ok(saved) });
            }
            Job::Create { uid, folder, name, bytes } => {
                let mut saved = entry(&format!("{folder}/{name}"), &name, EntryKind::File, true);
                saved.size = Some(bytes.len() as u64);
                post.send(Event::Saved { uid, entry: Ok(saved) });
            }
            Job::SignIn | Job::LinkStart => {}
            Job::LinkPoll(_) => post.send(Event::Linked(Ok(true))),
            Job::SignOut => {}
            Job::Conversation(_) => post.send(Event::Conversation(Err(Fault::Refused("no conversation in the mock".into())))),
        }
    }
}

/// A backend that always refuses, so the fault-handling path has something to show.
struct RefusingBackend;

impl CloudBackend for RefusingBackend {
    fn submit(&self, job: Job, post: Post) {
        let fault = || Fault::Unreachable("mock service down".into());
        match job {
            Job::Account => post.send(Event::Account(Err(fault()))),
            Job::List(path) => post.send(Event::Listed { path, entries: Err(fault()) }),
            Job::SharedWithMe => post.send(Event::Shared(Err(fault()))),
            Job::Read { entry, into } => post.send(Event::Read { entry, into, bytes: Err(fault()) }),
            Job::Write { uid, .. } => post.send(Event::Saved { uid, entry: Err(fault()) }),
            Job::Create { uid, .. } => post.send(Event::Saved { uid, entry: Err(fault()) }),
            Job::SignIn | Job::LinkStart | Job::LinkPoll(_) => post.send(Event::Linked(Err(fault()))),
            Job::SignOut => {}
            Job::Conversation(_) => post.send(Event::Conversation(Err(fault()))),
        }
    }
}

/// One photo, rating 0, active: a session this suite's tests fully control rather than the
/// procedural demo's unpredictable ratings.
fn app() -> LightcraftApp {
    let mut s = lightcraft_engine::Session::new();
    let photo = Photo::new(PhotoId(1), Source::File { path: "/lightcraft-cloud-files-test/1.jpg".into() }, "1.jpg", "JPEG", 40, 30, "2026-11-18");
    s.catalog.apply(Op::AddPhoto { photo: Box::new(photo) }).expect("a fresh catalog accepts one photo");
    s.execute("library.select", &serde_json::json!({"ids": [1], "active": 1})).expect("selecting the one photo");
    LightcraftApp::new(s, Services { png: None, ..Default::default() })
}

/// A session with real, renderable pixels (the procedural demo), for the one test that needs
/// `CloudApp::encode` to produce real export bytes rather than a placeholder.
fn demo_app() -> LightcraftApp {
    LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() })
}

/// Every id in a menu bar, submenus included.
fn all_ids(bar: &[(String, Vec<MenuNode>)]) -> Vec<String> {
    fn walk(nodes: &[MenuNode], out: &mut Vec<String>) {
        for node in nodes {
            match node {
                MenuNode::Item { id, .. } => out.push(id.clone()),
                MenuNode::Submenu { children, .. } => walk(children, out),
                MenuNode::Separator => {}
            }
        }
    }
    let mut out = Vec::new();
    for (_, items) in bar {
        walk(items, &mut out);
    }
    out
}

#[test]
fn cloud_affordances_are_hidden_without_a_backend() {
    let mut app = self::app();
    assert!(!cloud::available(&app), "no backend is attached in a freshly built app");
    assert!(!ui_enabled(&app, "file.cloudOpen"));
    assert!(!ui_enabled(&app, "file.cloudSave"));
    assert!(!ui_enabled(&app, "file.cloudShare"));

    let ids = all_ids(&menu_bar(&app));
    for id in ["file.cloudOpen", "file.cloudSave", "file.cloudShare"] {
        assert!(!ids.iter().any(|i| i == id), "{id} must not appear in File without a backend");
    }

    let err = app.run("file.cloudOpen", serde_json::json!({})).expect_err("no backend is attached");
    assert!(!err.is_empty());
    assert_eq!(app.ui.dialog, None, "a refused command must not open the dialog");
}

#[test]
fn attaching_a_backend_reveals_the_cloud_menu_items() {
    let mut app = self::app();
    app.cloud.connect(MockBackend);
    // `connect` only attaches the backend; it asks nobody anything. `start` is what a page calls
    // on open to learn who is signed in (and to list the root folder), same as it would here.
    cloud::start(&mut app);
    cloud::pump(&mut app, &egui::Context::default());

    assert!(cloud::available(&app));
    assert!(ui_enabled(&app, "file.cloudOpen"));
    assert!(ui_enabled(&app, "file.cloudSave"), "a photo is active");
    assert_eq!(app.cloud.account.as_ref().map(|a| a.display.as_str()), Some("mock@cloud.test"));

    let ids = all_ids(&menu_bar(&app));
    for id in ["file.cloudOpen", "file.cloudSave", "file.cloudShare"] {
        assert!(ids.iter().any(|i| i == id), "{id} must appear in File once a backend is attached");
    }
}

#[test]
fn the_file_menu_commands_open_the_right_dialog() {
    let mut app = self::app();
    app.cloud.connect(MockBackend);
    cloud::pump(&mut app, &egui::Context::default());

    app.run("file.cloudOpen", serde_json::json!({})).expect("a backend is attached");
    assert_eq!(app.ui.dialog, Some(Dialog::CloudOpen));

    app.run("file.cloudSave", serde_json::json!({})).expect("a backend is attached and a photo is active");
    assert_eq!(app.ui.dialog, Some(Dialog::CloudSave));
}

/// Finding 11: `ui_enabled` and `run` must agree on `file.cloudSave` requiring an active photo.
#[test]
fn cloud_save_is_refused_without_an_active_photo_agreeing_with_ui_enabled() {
    let mut app = self::app();
    app.cloud.connect(MockBackend);
    cloud::start(&mut app);
    cloud::pump(&mut app, &egui::Context::default());
    // `library.selectNone` ("Deselect All") deliberately keeps the active photo active, the same
    // as Lightroom's own Deselect All; it is not a way to clear `active()`. Nothing in the command
    // registry is, so this test clears the selection directly.
    app.session.selection = lightcraft_engine::Selection::default();

    assert!(!ui_enabled(&app, "file.cloudSave"), "ui_enabled must refuse Save with no active photo");
    let err = app.run("file.cloudSave", serde_json::json!({})).expect_err("run must refuse it too");
    assert!(!err.is_empty());
    assert_eq!(app.ui.dialog, None, "a refused command must not open the dialog");
}

#[test]
fn list_then_read_round_trips_through_the_mock_backend() {
    let mut app = self::app();
    app.cloud.connect(MockBackend);
    let ctx = egui::Context::default();
    cloud::pump(&mut app, &ctx);

    cloud::browse(&mut app, "");
    cloud::pump(&mut app, &ctx);
    assert_eq!(app.cloud.mine.entries.len(), 2);
    let file = app.cloud.mine.entries.iter().find(|e| e.kind == EntryKind::File).expect("the mock lists one file").clone();

    cloud::open_entry(&mut app, file, None);
    cloud::pump(&mut app, &ctx);
    assert!(app.cloud.busy.is_none(), "the read answered");
    // `CloudApp::open_bytes` on desktop queues the bytes through the import task and answers
    // `Ok(None)` (the new photo's id is not known synchronously), so `apply` never reaches the
    // status-setting branch that only fires for `Some(uid)`. The two things it does promise: no
    // error reached the user, and an import is now queued for those bytes.
    assert!(app.notices.is_empty(), "open_bytes must not have reported an error: {:?}", app.notices);
    assert!(app.import.is_some(), "the read bytes must be queued for import");
}

/// Finding 7: the real `Entry` separates `id` (opaque handle) from `path` (breadcrumb/folder
/// addressing). A mock whose id is not path-shaped would expose a regression that conflated them.
#[test]
fn folder_addressing_uses_path_not_id() {
    let mut app = self::app();
    app.cloud.connect(MockBackend);
    let ctx = egui::Context::default();
    cloud::pump(&mut app, &ctx);
    cloud::browse(&mut app, "");
    cloud::pump(&mut app, &ctx);

    let folder = app.cloud.mine.entries.iter().find(|e| e.kind == EntryKind::Folder).expect("the mock lists one folder").clone();
    assert_eq!(folder.id, "folder-1", "the mock's id is deliberately not path-shaped");
    assert_eq!(folder.path, "/Photos");

    // Walking into the folder browses its `path`, never its opaque `id`.
    cloud::browse(&mut app, &folder.path);
    cloud::pump(&mut app, &ctx);
    assert_eq!(app.cloud.mine.path, folder.path);
}

/// Finding 8: with real `encode` wired, a Save carries real bytes, not an empty placeholder.
#[test]
fn save_as_encodes_real_photo_bytes() {
    let mut app = demo_app();
    app.cloud.connect(MockBackend);
    let ctx = egui::Context::default();
    cloud::pump(&mut app, &ctx);

    let uid = app.session.active().or_else(|| app.session.catalog.photos().next().map(|p| p.id)).expect("the demo has a photo");
    app.session.execute("library.select", &serde_json::json!({"ids": [uid.0], "active": uid.0})).expect("selecting the demo photo");

    let saved = cloud::save_as(&mut app, "", "roundtrip.jpg").expect("the mock backend accepts the write");
    assert_eq!(saved["saving"], "roundtrip.jpg");
    let bytes = saved["bytes"].as_u64().expect("a byte count");
    assert!(bytes > 100, "a real JPEG export is not a placeholder: got {bytes} bytes");

    cloud::pump(&mut app, &ctx);
    let status = app.cloud.status.as_deref().expect("the save answered");
    assert!(status.contains("roundtrip.jpg"), "{status}");
}

#[test]
fn a_refusing_backend_reports_a_fault_without_panicking() {
    let mut app = self::app();
    app.cloud.connect(RefusingBackend);
    let ctx = egui::Context::default();
    cloud::start(&mut app);
    cloud::pump(&mut app, &ctx);

    assert!(app.cloud.account.is_none(), "a refused Account job must not fabricate one");
    let fault = app.cloud.account_fault.as_ref().expect("the refusal is reported");
    assert_eq!(fault.code(), "unreachable");
}

/// The rating-key regression: a cloud dialog hosted as `Dialog::CloudOpen` already blocks
/// `shortcuts::handle` through the same `app.ui.dialog.is_some()` check every other dialog does.
/// Pressing the rating key must not change the photo behind it.
#[test]
fn opening_the_cloud_page_blocks_the_rating_key() {
    let mut app = self::app();
    app.cloud.connect(MockBackend);
    cloud::pump(&mut app, &egui::Context::default());
    app.run("file.cloudSave", serde_json::json!({})).expect("a backend is attached and a photo is active");
    assert_eq!(app.ui.dialog, Some(Dialog::CloudSave));
    assert_eq!(app.session.catalog.photo(PhotoId(1)).map(|p| p.rating), Some(0));

    let mut h = Headless::new(app, [1200.0, 800.0], 1.0);
    h.step();
    h.request("ui.key", serde_json::json!({"key": "3"}), T);
    h.step();
    h.step();

    assert_eq!(h.app.ui.dialog, Some(Dialog::CloudSave), "the dialog itself must not react to a rating key");
    assert_eq!(h.app.session.catalog.photo(PhotoId(1)).map(|p| p.rating), Some(0), "the rating behind the dialog must not change");
}

/// The single-Escape regression: one Escape press while `Dialog::CloudOpen` is open must close
/// that dialog and nothing else. `shortcuts::handle` (the only other place Escape could mean View
/// ▸ Back) is gated on the same `app.ui.dialog.is_some()` the dialog itself closes on, so the two
/// cannot both react to one press.
#[test]
fn escape_closes_the_cloud_dialog_and_nothing_else() {
    let mut app = self::app();
    app.cloud.connect(MockBackend);
    cloud::pump(&mut app, &egui::Context::default());
    app.run("file.cloudOpen", serde_json::json!({})).expect("a backend is attached");
    assert_eq!(app.ui.dialog, Some(Dialog::CloudOpen));
    let lights_out_before = app.ui.lights_out;

    let mut h = Headless::new(app, [1200.0, 800.0], 1.0);
    h.step();
    h.request("ui.key", serde_json::json!({"key": "escape"}), T);
    h.step();
    h.step();

    assert_eq!(h.app.ui.dialog, None, "Escape must close the cloud dialog");
    assert_eq!(h.app.ui.lights_out, lights_out_before, "View \u{25b8} Back must not also have fired on the same Escape");
}
