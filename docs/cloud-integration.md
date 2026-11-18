# Cloud Open/Save

## The real contract

LightCraft's cloud affordances go through `craft_ui::cloud` (storytold/craft-libs#160, merged
2026-11-17). `crates/ui-egui/src/cloud.rs` implements `craft_ui::cloud::CloudApp` for
`LightcraftApp`. It states what one of its documents is. It turns bytes from the cloud into a
photo, through the desktop import queue. It turns a photo back into bytes, through
`lightcraft_engine::export`. It states the words the pages show. `craft-ui`, `craft-docpath` and
`craft-window` share one `storytold/craft-libs` revision, the #160 merge commit. `craft-cloud`,
the egui-free contract crate the pages speak through, pins that same revision.

The pages, the account badge, the share dialog and the presence bar are `craft_ui`'s own code.
This branch does not reimplement any of them. `Dialog::CloudOpen` and `Dialog::CloudSave` host
`craft_ui::cloud::open_page` and `save_page` inside LightCraft's ordinary dialog chrome
(`crates/ui-egui/src/dialogs.rs`), the same machinery every other dialog uses. That machinery
covers the backdrop, Escape, and the input barrier every other dialog already blocks through:
`shortcuts::handle`, `drop_overlay`, `lens_db`, `panels::person::handle_bulk_key`, `space_hand` and
`menus::run::view_back_command`, all gated on `app.ui.dialog.is_some()`.

The share dialog and the conflict modal manage their own open state outside `Dialog`. Both draw
themselves every frame (`self.share.show(ctx)`, `craft_ui::cloud::conflict_modal`). Neither
`app.ui.dialog.is_some()` check above sees them. `crate::cloud::blocks_input` covers that gap; the
same six call sites above also check it.

Nothing in this repository calls `CloudFiles::connect` yet. No storytold crate implements
`craft_cloud::Provider`, or an application-specific client, for a real service. `craft_ui::cloud::available`
reads `false` everywhere this app ships. Every cloud affordance stays hidden under that condition:
the File menu items, the top-bar account badge, the Open/Save dialogs, the share dialog. None of
them show up disabled. They do not show up at all.

`crates/ui-egui/tests/it/cloud_files.rs` exercises the contract against two in-process mock
backends. One answers every job. One refuses every job. Two more tests target the input-barrier
gap by name. One opens the Save dialog and presses a rating key. It checks that the photo behind
the dialog kept its rating. One opens the Open dialog and presses Escape once. It checks that the
dialog closed, and that nothing else (View ▸ Back's lights-out toggle) also fired on that press.

## Why the pin flip happened now

The branch originally built a local, hand-rolled stand-in for this contract. storytold/craft-libs
did not have a `cloud` feature yet; the only implementation was a `wdunn001/craft-libs` fork,
pinned apart from the rest of the workspace. [FredApps' review](
https://github.com/storytold/designcraft/pull/878#pullrequestreview-5764297647) on the earlier,
closed [storytold/lightcraft#2060](https://github.com/storytold/lightcraft/pull/2060) asked for
no such fork pin. It asked for no local reinvention either. craft-libs#160 then merged. It brought
the real, vendor-neutral contract onto `storytold/craft-libs` main. It brought the egui-free
`craft-cloud` crate there too, on the same `storytold/egui` revision LightCraft already pins. An
adversarial review of the local module traced most of its findings back to the local types
diverging from the real contract in ways a hand-rolled stand-in could not have caught. Building
against the real contract removed that whole category of findings at once. The local module was
due to be deleted anyway. A finding fixed one at a time, inside a module already marked for
deletion, would not have removed the category at all.

## What this branch drops entirely

**`craft-cloud-client` and `collabcraft-share`** (`wdunn001/collab-craft`). The old branch's
transport layer lived here: a worker thread, a device-flow sign-in, a token store, all built to
talk to CollabCraft specifically. No wdunn001 crate, pin or feature carries into this branch; a
search for `wdunn001` across `Cargo.toml` and the source tree finds nothing.

**The desktop worker in `apps/lightcraft/src/cloud.rs` and the browser worker in
`apps/lightcraft-web/src/cloud.rs`.** Both existed only to drive the two crates above. Neither app
needs a `cloud.rs` at all. `CloudApp` is implemented once, in `crates/ui-egui`. Both apps already
share that crate. Neither app calls `connect`. The old branch's web-side runtime loader
(`craft-runtime-loader.js`, `craft-runtime-pilot.js`, `craft-runtime-smoke.html`) does not carry
over either. Its `xtask/src/web.rs` changes do not carry over either. Both were web-worker
plumbing for the transport layer above. This branch has no transport layer for them to plumb.

## What this branch does not implement yet

**The real backend.** `craft_cloud` holds traits only (`Files`, `Sharing`, `Contacts`,
`Notifications`, `Mail`, `Accounts`, `Conversations`). Its own doc comment says so plainly:
"nothing in this crate opens a socket." No storytold crate implements `Provider` for a real
service yet. CollabCraft is not that crate either; craft-libs#123, folding collab-craft into
storytold, has not been decided. Until something does, `CloudFiles::connect` is called nowhere in
this repository.

**The Keyboard Shortcuts editor's bindable list.** It does not filter by `host_supports`.
`file.cloudOpen`, `file.cloudSave` and `file.cloudShare` are therefore bindable there even while
hidden from the File menu. `file.backupLibrary` already carries this exact gap, on the same
unfiltered list, for the same reason. This branch changes the shortcuts editor for no command,
cloud or otherwise: filtering it would change every optional-service command's bindable listing at
once, a wider change than this branch's scope.

## What remains before the PR can open

A storytold crate has to implement `craft_cloud::Provider`, or a service-specific client
implementing `CloudBackend`/`ShareBackend` directly, before anything here is reachable. Once one
exists, the apps can construct it and call `CloudFiles::connect`. `craft_ui::cloud::available` can
then stop reading `false` unconditionally, and the Open/Save dialogs, the account badge and the
share dialog can start actually showing.
