use android_activity::AndroidApp;
use figcraft_engine::Session;
use figcraft_ui_egui::{DialogFuture, FigApp, Services};
use jni::objects::JValue;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{channel, Receiver, Sender};

const WORKSPACE_DIR: &str = "/sdcard/Download/ArtCraft";
const FONTS_DIR: &str = "/sdcard/Download/ArtCraft/Fonts";

static ANDROID_APP: Mutex<Option<AndroidApp>> = Mutex::new(None);

fn ensure_workspace_dir() -> PathBuf {
    let p = PathBuf::from(WORKSPACE_DIR);
    let _ = std::fs::create_dir_all(&p);
    let _ = std::fs::create_dir_all(PathBuf::from(FONTS_DIR));
    p
}

fn return_to_launcher() {
    log::info!("return_to_launcher: Bringing Craft Launcher (ai.storyteller.craftlauncher) to foreground");
    let guard = ANDROID_APP.lock().unwrap();
    let Some(ref app) = *guard else { return };

    let vm_ptr = app.vm_as_ptr();
    let activity_ptr = app.activity_as_ptr();
    if vm_ptr.is_null() || activity_ptr.is_null() {
        return;
    }

    unsafe {
        let jvm = match jni::JavaVM::from_raw(vm_ptr as *mut jni::sys::JavaVM) {
            Ok(jvm) => jvm,
            Err(_) => return,
        };
        let mut env = match jvm.attach_current_thread() {
            Ok(env) => env,
            Err(_) => return,
        };
        let activity = match jni::objects::JObject::from_raw(activity_ptr as jni::sys::jobject) {
            activity if !activity.as_raw().is_null() => activity,
            _ => return,
        };

        let act_class = match env.find_class("android/content/Intent") {
            Ok(c) => c,
            Err(_) => return,
        };

        let pkg_jstring = match env.new_string("ai.storyteller.craftlauncher") {
            Ok(s) => s,
            Err(_) => return,
        };
        let cls_jstring = match env.new_string("android.app.NativeActivity") {
            Ok(s) => s,
            Err(_) => return,
        };

        let intent_obj = match env.new_object(&act_class, "()V", &[]) {
            Ok(obj) => obj,
            Err(_) => return,
        };

        let comp_class = match env.find_class("android/content/ComponentName") {
            Ok(c) => c,
            Err(_) => return,
        };

        let comp_obj = match env.new_object(&comp_class, "(Ljava/lang/String;Ljava/lang/String;)V", &[JValue::Object(&pkg_jstring), JValue::Object(&cls_jstring)]) {
            Ok(obj) => obj,
            Err(_) => return,
        };

        let _ = env.call_method(&intent_obj, "setComponent", "(Landroid/content/ComponentName;)Landroid/content/Intent;", &[JValue::Object(&comp_obj)]);

        let flags: i32 = 0x10000000 | 0x00020000;
        let _ = env.call_method(&intent_obj, "setFlags", "(I)Landroid/content/Intent;", &[JValue::Int(flags)]);
        let _ = env.call_method(&activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent_obj)]);
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum DialogMode {
    Open,
    Save,
}

struct PendingModalDialog {
    mode: DialogMode,
    current_dir: PathBuf,
    selected_file: Option<PathBuf>,
    save_filename: String,
    exts: Vec<String>,
    sender: Sender<Option<PathBuf>>,
}

static PENDING_MODAL: Mutex<Option<PendingModalDialog>> = Mutex::new(None);

fn render_pending_modal_dialog(ctx: &egui::Context) {
    let mut dlg_guard = PENDING_MODAL.lock().unwrap();
    let Some(ref mut dlg) = *dlg_guard else { return };

    let is_open = dlg.mode == DialogMode::Open;
    let title = if is_open {
        "Buka Berkas FigCraft (Open Document)"
    } else {
        "Simpan Berkas FigCraft (Save Document)"
    };

    let mut send_answer: Option<Option<PathBuf>> = None;
    let mut navigate_to: Option<PathBuf> = None;

    egui::Window::new(title)
        .id(egui::Id::new("figcraft_android_file_dialog"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(540.0, 420.0))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("⬆ Atas").clicked() {
                    if let Some(parent) = dlg.current_dir.parent() {
                        navigate_to = Some(parent.to_path_buf());
                    }
                }
                if ui.button("📁 ArtCraft").clicked() {
                    navigate_to = Some(PathBuf::from(WORKSPACE_DIR));
                }
                if ui.button("🔤 Fonts").clicked() {
                    navigate_to = Some(PathBuf::from(FONTS_DIR));
                }
                ui.label(
                    egui::RichText::new(dlg.current_dir.to_string_lossy().to_string())
                        .strong()
                        .monospace(),
                );
            });

            ui.separator();

            let mut entries_list = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&dlg.current_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let is_dir = path.is_dir();
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                    if !name.starts_with('.') {
                        if is_dir {
                            entries_list.push((name, path, true));
                        } else {
                            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                            if !is_open || dlg.exts.is_empty() || dlg.exts.iter().any(|e| e.eq_ignore_ascii_case(&ext)) {
                                entries_list.push((name, path, false));
                            }
                        }
                    }
                }
            }
            entries_list.sort_by(|a, b| {
                if a.2 == b.2 {
                    a.0.to_lowercase().cmp(&b.0.to_lowercase())
                } else if a.2 {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            });

            egui::ScrollArea::vertical()
                .max_height(260.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if entries_list.is_empty() {
                        ui.label(egui::RichText::new("Direktori ini kosong.").italics());
                    }
                    for (name, path, is_dir) in entries_list {
                        let is_selected = dlg.selected_file.as_ref() == Some(&path);
                        ui.horizontal(|ui| {
                            let icon = if is_dir { "📁" } else { "📄" };
                            let label = format!("{} {}", icon, name);
                            let resp = ui.selectable_label(is_selected, label);
                            if resp.clicked() {
                                if is_dir {
                                    navigate_to = Some(path.clone());
                                } else {
                                    dlg.selected_file = Some(path.clone());
                                    if !is_open {
                                        dlg.save_filename = name.clone();
                                    }
                                }
                            }
                            if resp.double_clicked() {
                                if is_dir {
                                    navigate_to = Some(path);
                                } else if is_open {
                                    send_answer = Some(Some(path));
                                }
                            }
                        });
                    }
                });

            ui.separator();
            ui.add_space(4.0);

            if !is_open {
                ui.horizontal(|ui| {
                    ui.label("Nama Berkas:");
                    ui.text_edit_singleline(&mut dlg.save_filename);
                });
                ui.add_space(4.0);
            }

            ui.horizontal(|ui| {
                if ui.button(egui::RichText::new("Batal (Cancel)").color(egui::Color32::from_rgb(220, 80, 80))).clicked() {
                    send_answer = Some(None);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if is_open {
                        let can_open = dlg.selected_file.is_some();
                        let btn = ui.add_enabled(can_open, egui::Button::new(egui::RichText::new("Buka (Open)").strong()));
                        if btn.clicked() {
                            if let Some(ref sel) = dlg.selected_file {
                                send_answer = Some(Some(sel.clone()));
                            }
                        }
                    } else {
                        let can_save = !dlg.save_filename.trim().is_empty();
                        let btn = ui.add_enabled(can_save, egui::Button::new(egui::RichText::new("Simpan (Save)").strong()));
                        if btn.clicked() {
                            let mut target = dlg.current_dir.join(dlg.save_filename.trim());
                            if target.extension().is_none() && !dlg.exts.is_empty() {
                                target.set_extension(&dlg.exts[0]);
                            }
                            send_answer = Some(Some(target));
                        }
                    }
                });
            });
        });

    if let Some(path) = navigate_to {
        dlg.current_dir = path;
        dlg.selected_file = None;
    }

    if let Some(ans) = send_answer {
        let _ = dlg.sender.send(ans);
        *dlg_guard = None;
    }
}

struct AndroidServices;

impl Services for AndroidServices {
    fn pick_open(&mut self, _title: &str, exts: &[&str]) -> DialogFuture {
        let (tx, rx): (Sender<Option<PathBuf>>, Receiver<Option<PathBuf>>) = channel();
        let base_dir = ensure_workspace_dir();
        *PENDING_MODAL.lock().unwrap() = Some(PendingModalDialog {
            mode: DialogMode::Open,
            current_dir: base_dir,
            selected_file: None,
            save_filename: String::new(),
            exts: exts.iter().map(|s| s.to_string()).collect(),
            sender: tx,
        });

        Box::pin(async move {
            rx.recv().ok().flatten()
        })
    }

    fn pick_save(&mut self, _title: &str, name: &str, exts: &[&str]) -> DialogFuture {
        let (tx, rx): (Sender<Option<PathBuf>>, Receiver<Option<PathBuf>>) = channel();
        let base_dir = ensure_workspace_dir();
        let path = Path::new(name);
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string());
        *PENDING_MODAL.lock().unwrap() = Some(PendingModalDialog {
            mode: DialogMode::Save,
            current_dir: base_dir,
            selected_file: None,
            save_filename: file_name,
            exts: exts.iter().map(|s| s.to_string()).collect(),
            sender: tx,
        });

        Box::pin(async move {
            rx.recv().ok().flatten()
        })
    }

    fn pick_folder(&mut self, _title: &str) -> DialogFuture {
        Box::pin(std::future::ready(Some(ensure_workspace_dir())))
    }

    fn clipboard_set_text(&mut self, _s: &str) {}
    fn clipboard_get_text(&mut self) -> Option<String> { None }
    fn clipboard_get_html(&mut self) -> Option<String> { None }
    fn clipboard_set_png(&mut self, _w: u32, _h: u32, _rgba: &[u8]) {}
    fn clipboard_get_image(&mut self) -> Option<(u32, u32, Vec<u8>)> { None }
    fn open_url(&mut self, url: &str) {
        if url == "app://launcher" {
            return_to_launcher();
        }
    }
    fn reveal_path(&mut self, _path: &Path) {}
    fn save_prefs(&mut self, _prefs: &figcraft_engine::Prefs) {}
    fn emergency_save(&mut self, _session: &Session) {}
    fn reduce_motion(&self) -> bool { false }
}

fn apply_immersive_sticky(app: &AndroidApp) {
    let vm_ptr = app.vm_as_ptr();
    let activity_ptr = app.activity_as_ptr();
    if vm_ptr.is_null() || activity_ptr.is_null() {
        return;
    }

    unsafe {
        let jvm = match jni::JavaVM::from_raw(vm_ptr as *mut jni::sys::JavaVM) {
            Ok(jvm) => jvm,
            Err(_) => return,
        };
        let mut env = match jvm.attach_current_thread() {
            Ok(env) => env,
            Err(_) => return,
        };
        let activity = match jni::objects::JObject::from_raw(activity_ptr as jni::sys::jobject) {
            activity if !activity.as_raw().is_null() => activity,
            _ => return,
        };

        if let Ok(window) = env.call_method(&activity, "getWindow", "()Landroid/view/Window;", &[]) {
            if let Ok(window_obj) = window.l() {
                if let Ok(decor_view) = env.call_method(&window_obj, "getDecorView", "()Landroid/view/View;", &[]) {
                    if let Ok(decor_obj) = decor_view.l() {
                        let flags: i32 = 4096 | 4 | 2 | 1024 | 512 | 256;
                        let _ = env.call_method(&decor_obj, "setSystemUiVisibility", "(I)V", &[JValue::Int(flags)]);
                    }
                }
            }
        }
    }
}

struct FigAndroidApp {
    app: FigApp,
}

impl eframe::App for FigAndroidApp {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, _raw: &mut egui::RawInput) {
        // Let touch scrolling and keyboard shortcuts pass through
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.logic(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
        render_pending_modal_dialog(ui.ctx());
    }
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("FigCraft"),
    );

    log::info!("Starting FigCraft on Android ARM64 for Huawei MatePad 12X...");

    *ANDROID_APP.lock().unwrap() = Some(app.clone());

    apply_immersive_sticky(&app);
    let _ = ensure_workspace_dir();

    // Scan fonts on demand or background
    std::thread::spawn(|| {
        figcraft_text::scan_system_fonts();
    });

    let mut options = eframe::NativeOptions::default();
    options.android_app = Some(app);
    options.renderer = eframe::Renderer::Wgpu;

    let _ = eframe::run_native(
        "FigCraft",
        options,
        Box::new(|_cc| {
            let session = Session::default();
            let services = Box::new(AndroidServices);
            let fig_app = FigApp::new(session, services);
            Ok(Box::new(FigAndroidApp { app: fig_app }))
        }),
    );
}

