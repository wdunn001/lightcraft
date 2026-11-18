//! LightCraft's half of `craft_ui::cloud`: the vendor-neutral Open/Save contract every Craft app
//! shares (storytold/craft-libs#160). The pages, the account badge and the share/presence surfaces
//! are `craft_ui`'s own; what is here is what only LightCraft can answer: what one of its documents
//! is, how bytes become a photo and back, and the words it shows them in.
//!
//! LightCraft holds no provider credential and speaks no provider protocol. Nothing in this crate,
//! or in `apps/lightcraft` / `apps/lightcraft-web`, attaches a [`CloudFiles::connect`] backend yet:
//! no storytold crate implements `craft_cloud::Provider` (or an application-specific client) for a
//! real service. Until one does, [`craft_ui::cloud::available`] reads `false` everywhere, and every
//! cloud affordance (the File menu items, the top-bar account badge, the Open/Save pages, the
//! share dialog) stays hidden rather than shown disabled.

use craft_cloud::EntryKind;
use craft_ui::cloud::{CloudApp, CloudFiles, CloudStyle, Destination};
use egui::{Color32, Painter, Rect, Ui};
use lightcraft_catalog::PhotoId;

use crate::LightcraftApp;
use crate::theme::Tokens;

/// Extensions LightCraft opens from cloud storage, the same set the desktop file picker offers.
const OPENS: [&str; 20] =
    ["jpg", "jpeg", "png", "tif", "tiff", "webp", "dng", "cr2", "cr3", "nef", "arw", "rw2", "raf", "orf", "pef", "srw", "gpr", "bmp", "heic", "jxl"];

/// Content types LightCraft opens, matched against the start of what the provider said. Read
/// beside the extension because neither is reliable alone: a WebDAV server hands back
/// `application/octet-stream` for a perfectly good raw file.
const MIMES: [&str; 5] = ["image/jpeg", "image/png", "image/tiff", "image/webp", "image/"];

/// Extensions CollabCraft-shaped Save can actually encode (developed output, not RAW originals).
const WRITABLE: &[&str] = &["jpg", "jpeg", "png", "tif", "tiff", "webp"];

/// What one LightCraft photo is, as a cloud file list needs to know.
pub const PHOTO: craft_ui::cloud::DocumentKind =
    craft_ui::cloud::DocumentKind { extensions: &OPENS, mimes: &MIMES, noun: "photo", default_extension: "jpg" };

/// The export format to write for a given cloud destination.
///
/// RAW and other open-only extensions are refused rather than writing JPEG bytes under a `.cr3` /
/// `.dng` name: a save must never silently mislabel what it wrote.
fn export_format_for(into: Destination<'_>) -> Result<lightcraft_engine::export::ExportFormat, String> {
    use lightcraft_engine::export::ExportFormat;
    let ext = into.extension();
    let mime = into.mime.unwrap_or("");
    match ext.as_str() {
        "jpg" | "jpeg" => Ok(ExportFormat::Jpeg),
        "png" => Ok(ExportFormat::Png),
        "tif" | "tiff" => Ok(ExportFormat::Tiff),
        "webp" => Ok(ExportFormat::Webp),
        _ if mime.starts_with("image/jpeg") => Ok(ExportFormat::Jpeg),
        _ if mime.starts_with("image/png") => Ok(ExportFormat::Png),
        _ if mime.starts_with("image/tiff") => Ok(ExportFormat::Tiff),
        _ if mime.starts_with("image/webp") => Ok(ExportFormat::Webp),
        _ => Err(format!(
            "cloud save writes JPEG, PNG, TIFF, or WebP; rename to {}.jpg (or .png / .tif / .webp)",
            std::path::Path::new(into.name).file_stem().and_then(|s| s.to_str()).unwrap_or("photo")
        )),
    }
}

/// Suggested cloud Save name: keep a writable extension, otherwise use `.jpg`.
fn cloud_save_name(file_name: &str) -> String {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name).trim();
    if base.is_empty() {
        return String::new();
    }
    let ext = std::path::Path::new(base).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if WRITABLE.iter().any(|w| *w == ext) {
        return base.to_owned();
    }
    let stem = std::path::Path::new(base).file_stem().and_then(|s| s.to_str()).unwrap_or(base);
    format!("{stem}.jpg")
}

/// Durable path for a cloud Open on desktop: under the library's `Originals/Cloud/` when the
/// library is on disk, otherwise a process-unique folder under the OS temp directory.
fn durable_cloud_path(app: &LightcraftApp, name: &str) -> Result<std::path::PathBuf, String> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let file_name = format!("{stamp}-{base}");
    let dir = app
        .session
        .library
        .as_ref()
        .filter(|l| l.on_disk)
        .map(|l| l.dir.join("Originals").join("Cloud"))
        .unwrap_or_else(|| std::env::temp_dir().join(format!("lightcraft-cloud-{}", std::process::id())));
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create cloud import folder: {e}"))?;
    Ok(dir.join(file_name))
}

impl CloudApp for LightcraftApp {
    fn cloud(&self) -> &CloudFiles {
        &self.cloud
    }

    fn cloud_mut(&mut self) -> &mut CloudFiles {
        &mut self.cloud
    }

    fn kind(&self) -> craft_ui::cloud::DocumentKind {
        PHOTO
    }

    /// The active (selected) photo's id, as the cloud pages need it. `PhotoId` wraps a `u64`, so
    /// the raw id carries through.
    fn active(&self) -> Option<u64> {
        self.session.active().map(|PhotoId(n)| n)
    }

    /// The file name of one photo, for the Save As field. RAW and other open-only names become
    /// `.jpg` so the suggested name matches what `encode` will actually write.
    fn title(&self, uid: u64) -> String {
        let raw = self.session.catalog.photo(PhotoId(uid)).map(|p| p.file_name.clone()).unwrap_or_default();
        cloud_save_name(&raw)
    }

    /// Turn bytes from cloud storage into an imported photo. The bytes are written under the
    /// library's `Originals/Cloud/` (or a process-scoped durable folder without one on disk), then
    /// imported in place so the catalog keeps a durable source path. `Ok(None)` is never returned:
    /// unlike the web host, the desktop import queue always has somewhere to put the bytes.
    fn open_bytes(&mut self, name: &str, bytes: Vec<u8>) -> Result<Option<u64>, String> {
        let path = durable_cloud_path(self, name)?;
        std::fs::write(&path, &bytes).map_err(|e| format!("could not write {name} into the library: {e}"))?;
        let path_str = path.to_string_lossy().to_string();
        if let Err(e) = crate::import::start_paths(self, vec![path_str]) {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
        Ok(None)
    }

    /// Turn the active photo into bytes to save, in the format the destination asks for. A photo
    /// opened as a raw file goes back as a developed image in one of the writable formats.
    fn encode(&mut self, uid: u64, into: Destination<'_>) -> Result<Vec<u8>, String> {
        let format = export_format_for(into)?;
        let opts = lightcraft_engine::export::ExportOptions { format, ..Default::default() };
        let item = lightcraft_engine::export::prepare_export(&mut self.session, PhotoId(uid), &opts, 1)
            .map_err(|e| format!("could not prepare photo for export: {e}"))?;
        let dest = lightcraft_engine::export::Destination { dir: String::new(), exact: Some("cloud_export".into()) };
        let mut captured = Vec::new();
        lightcraft_engine::export::run_batch(
            vec![item],
            &opts,
            &dest,
            &mut |_path, bytes| {
                captured = bytes.to_vec();
                Ok(())
            },
            &|_| false,
            true,
            &mut |_, _| true,
        )
        .map_err(|e| format!("could not export photo: {e}"))?;
        if captured.is_empty() {
            return Err("photo export produced no bytes".into());
        }
        Ok(captured)
    }

    /// Closing a cloud photo in LightCraft means nothing special: the photo stays in the catalog.
    fn close(&mut self, _uid: u64) {}

    fn sharing(&mut self) -> &mut craft_ui::share::ShareDialog {
        &mut self.share
    }

    fn presence(&mut self) -> &mut craft_ui::presence::PresenceBar {
        &mut self.presence
    }

    fn paint_icon(&self, painter: &Painter, rect: Rect, kind: EntryKind, colour: Color32) {
        let icon = if kind == EntryKind::Folder { crate::icons::Icon::Folder } else { crate::icons::Icon::GridPhoto };
        crate::icons::paint(painter, rect, icon, colour);
    }

    fn open_url(&self, url: &str) {
        // `services.open_url` is `FnMut`, which needs `&mut self`; this method only has `&self`.
        // Route through egui's output command queue instead, the same way the runner opens every
        // other browser link.
        if let Some(ctx) = &self.tasks.repaint {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }

    fn say(&mut self, text: String) {
        self.notices.push(text);
    }

    fn toast(&mut self, text: String) {
        self.ui.status = text;
    }

    fn app_name(&self) -> &str {
        "LightCraft"
    }

    fn tr(&self, text: &str) -> String {
        crate::i18n::tr(text).to_owned()
    }

    fn save_note(&self) -> Option<String> {
        Some("The photo is saved as a developed image in the format the destination expects.".into())
    }

    fn style(&self, ui: &Ui) -> CloudStyle {
        let t = Tokens::get(ui.ctx());
        CloudStyle {
            surface: t.chrome,
            border: t.divider,
            text: t.text,
            dim: t.text_dim,
            hover: t.hover,
            soft: t.cell_selected,
            family: egui::FontFamily::Proportional,
            bold: egui::FontFamily::Name("semibold".into()),
        }
    }
}

/// Whether a cloud surface is up that `app.ui.dialog` does not know about: the share dialog
/// (`craft_ui::share::ShareDialog::show`) and the conflict modal (`craft_ui::cloud::conflict_modal`)
/// both draw themselves every frame and manage their own open state outside `Dialog`, unlike the
/// Open/Save pages (hosted as `Dialog::CloudOpen` / `Dialog::CloudSave`, which already block input
/// through the ordinary `app.ui.dialog.is_some()` checks). Every place that already asks "is a
/// dialog open" to decide whether a key or a background gesture may act should ask this too.
pub fn blocks_input(app: &LightcraftApp) -> bool {
    app.share.is_open() || app.cloud.conflict.is_some()
}
