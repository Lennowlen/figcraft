use android_activity::AndroidApp;
use figcraft_engine::Session;
use figcraft_ui_egui::{DialogFuture, FigApp, Services};
use jni::objects::JValue;
use std::path::{Path, PathBuf};

const WORKSPACE_DIR: &str = "/sdcard/Download/ArtCraft";

fn ensure_workspace_dir() -> PathBuf {
    let p = PathBuf::from(WORKSPACE_DIR);
    let _ = std::fs::create_dir_all(&p);
    p
}

struct AndroidServices;

impl Services for AndroidServices {
    fn pick_open(&mut self, _title: &str, exts: &[&str]) -> DialogFuture {
        let base_dir = ensure_workspace_dir();
        let mut candidates = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&base_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                if path.is_file() && (exts.is_empty() || exts.iter().any(|&e| e.eq_ignore_ascii_case(&ext))) {
                    candidates.push(path);
                }
            }
        }
        candidates.sort();
        let picked = candidates.first().cloned();
        Box::pin(std::future::ready(picked))
    }

    fn pick_save(&mut self, _title: &str, name: &str, exts: &[&str]) -> DialogFuture {
        let base_dir = ensure_workspace_dir();
        let path = Path::new(name);
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string());
        let mut target = base_dir.join(file_name);
        if target.extension().is_none() && !exts.is_empty() {
            target.set_extension(exts[0]);
        }
        Box::pin(std::future::ready(Some(target)))
    }

    fn pick_folder(&mut self, _title: &str) -> DialogFuture {
        Box::pin(std::future::ready(Some(ensure_workspace_dir())))
    }

    fn clipboard_set_text(&mut self, _s: &str) {}
    fn clipboard_get_text(&mut self) -> Option<String> { None }
    fn clipboard_get_html(&mut self) -> Option<String> { None }
    fn clipboard_set_png(&mut self, _w: u32, _h: u32, _rgba: &[u8]) {}
    fn clipboard_get_image(&mut self) -> Option<(u32, u32, Vec<u8>)> { None }
    fn open_url(&mut self, _url: &str) {}
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
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.logic(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
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

    apply_immersive_sticky(&app);

    let mut options = eframe::NativeOptions::default();
    options.android_app = Some(app);

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
