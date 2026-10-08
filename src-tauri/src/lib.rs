//! shotkey — ホットキーで撮って、指定フォルダに保存するだけのスクリーンショットアプリ。
//!
//! 撮る流れ:
//!   ホットキー → 画面全体を先に 1 枚に固める → (全体 / モニタ / ウィンドウ なら) すぐ切り出して保存
//!                                           → (範囲 なら) 固めた画面を各モニタに重ねて、その上で範囲を選ぶ
//! 先に固めるので、右クリックメニューのような「撮ろうとすると消えるもの」も撮れる。
//!
//! 録画 (v0.2.0): ホットキーで開始、もう一度押すと停止。ウィンドウ / モニタ / 範囲。システム音つき。録画中は小窓 (REC・時間・停止) を出す。 → record.rs

mod capture;
mod config;
mod record;

use config::Config;
use shotkey_core::{geom, naming, pixels::Bgra};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{
    ipc::Response,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    All,
    Monitor,
    Window,
    Region,
    TimerWindow,
    TimerMonitor,
    TimerAll,
    RecWindow,
    RecMonitor,
    RecRegion,
}

/// 範囲を選んだあと、何をするか
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    Shot,   // 静止画を撮る
    Record, // その範囲を録画する
}

/// 範囲選択のあいだだけ持つ、固めた画面
struct Frozen {
    desktop: Bgra,
    monitors: Vec<geom::Rect>,
    purpose: Purpose,
}

struct AppState {
    cfg: Mutex<Config>,
    cfg_path: PathBuf,
    frozen: Mutex<Option<Frozen>>,
    keys: Mutex<HashMap<u32, Action>>, // ホットキーの id → 何をするか
    busy: AtomicBool,                  // 撮影中 (範囲選択中も含む) は、次のホットキーを受けない
    key_errors: Mutex<Vec<String>>,    // 登録できなかったホットキーの説明
    rec: Mutex<Option<record::Session>>, // 録画中なら、その録画
}

// ---- 保存 ----

fn save_dir(app: &AppHandle, cfg: &Config) -> PathBuf {
    if !cfg.save_dir.trim().is_empty() {
        return PathBuf::from(cfg.save_dir.trim());
    }
    app.path().picture_dir().unwrap_or_else(|_| PathBuf::from(".")).join("shotkey")
}

fn tooltip(app: &AppHandle, text: &str) {
    if let Some(t) = app.tray_by_id("main") {
        let _ = t.set_tooltip(Some(text));
    }
}

/// 保存するファイルのパス。連番は、保存先にある同じ形の名前の一番大きい番号 + 1。
fn next_path(app: &AppHandle, cfg: &Config, template: &str, ext: &str) -> Result<PathBuf, String> {
    let dir = save_dir(app, cfg);
    std::fs::create_dir_all(&dir).map_err(|e| format!("保存先のフォルダを作れませんでした: {e}"))?;

    let now = chrono::Local::now();
    let (date, time) = (now.format("%Y%m%d").to_string(), now.format("%H%M%S").to_string());
    let existing: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| rd.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()).collect())
        .unwrap_or_default();
    let n = naming::next_number(template, &existing, &date, &time);
    let stem = naming::sanitize(&naming::render(template, n, cfg.digits, &date, &time));
    Ok(dir.join(format!("{stem}.{ext}")))
}

/// 撮った画像を、設定どおりに保存する。保存したファイルのパスを返す。
fn save_image(app: &AppHandle, img: &Bgra) -> Result<PathBuf, String> {
    let cfg = app.state::<AppState>().cfg.lock().unwrap().clone();
    let ext = if cfg.format == "jpg" { "jpg" } else { "png" };
    let path = next_path(app, &cfg, &cfg.template, ext)?;

    // 音は、書き出しを待たずに、撮れた瞬間に鳴らす (大きい画像は、PNG の圧縮に時間がかかる)
    if cfg.sound {
        capture::play_shutter();
    }

    let rgba = img.to_rgba();
    let (w, h) = (img.w as u32, img.h as u32);
    if ext == "jpg" {
        // JPEG はアルファを持てない
        let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), cfg.jpg_quality);
        let buf = image::RgbImage::from_raw(w, h, rgb).ok_or("画素の数が合いません")?;
        buf.write_with_encoder(enc).map_err(|e| e.to_string())?;
    } else {
        // 圧縮は軽め (Fast + Sub)。ファイルは少し大きくなるが、書き出しがずっと速い
        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let enc = image::codecs::png::PngEncoder::new_with_quality(
            std::io::BufWriter::new(file),
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Sub,
        );
        let buf = image::RgbaImage::from_raw(w, h, rgba.clone()).ok_or("画素の数が合いません")?;
        buf.write_with_encoder(enc).map_err(|e| e.to_string())?;
    }

    if cfg.clipboard {
        // 失敗しても、保存はできているので、止めない
        if let Ok(mut cb) = arboard::Clipboard::new() {
            let _ = cb.set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: rgba.into() });
        }
    }
    tooltip(app, &format!("shotkey — 保存しました: {}", path.file_name().and_then(|s| s.to_str()).unwrap_or("")));
    Ok(path)
}

// ---- 録画 ----

fn start_recording(app: &AppHandle, target: record::Target, crop: Option<(u32, u32, u32, u32)>) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.rec.lock().unwrap().is_some() {
        return Err("すでに録画中です".into());
    }
    let cfg = state.cfg.lock().unwrap().clone();
    let path = next_path(app, &cfg, &cfg.video_template, "mp4")?;
    let session = record::start(
        target,
        record::Options {
            path: path.clone(),
            fps: cfg.video_fps,
            bitrate: cfg.video_mbps * 1_000_000,
            cursor: cfg.video_cursor,
            audio: cfg.video_audio,
            crop,
        },
    )?;
    let audio_error = session.audio_error.clone();
    *state.rec.lock().unwrap() = Some(session);
    if cfg.sound {
        capture::play_shutter();
    }
    open_rec_window(app, cfg.video_limit_secs);
    if cfg.video_limit_secs > 0 {
        // 時間になったら、自動で止める (その前に手で止められていたら、何もしない)
        let app2 = app.clone();
        std::thread::spawn(move || {
            for _ in 0..cfg.video_limit_secs {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let same = app2.state::<AppState>().rec.lock().unwrap().as_ref().map(|s| s.path == path);
                if same != Some(true) {
                    return;
                }
            }
            stop_recording_blocking(&app2);
        });
    }
    match audio_error {
        Some(e) => tooltip(app, &format!("shotkey — 録画中 (音は録れていません: {e})")),
        None => tooltip(app, "shotkey — 録画中 (録画のホットキーをもう一度押すと停止)"),
    }
    Ok(())
}

/// 録画中だけ出す小窓: ● REC・経過時間・停止ボタン。録画には写らない (キャプチャ除外)。
fn open_rec_window(app: &AppHandle, limit_secs: u32) {
    let built = WebviewWindowBuilder::new(app, "rec", WebviewUrl::App(format!("rec.html?limit={limit_secs}").into()))
        .title("shotkey 録画中")
        .inner_size(210.0, 40.0)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .focused(false)
        .content_protected(true) // 録画・スクショに写さない
        .build();
    match built {
        Ok(win) => {
            // 手前のモニタの、上の真ん中あたり
            if let Ok(Some(mon)) = app.primary_monitor() {
                let scale = mon.scale_factor();
                let (mp, ms) = (mon.position(), mon.size());
                let w = (210.0 * scale) as i32;
                let x = mp.x + (ms.width as i32 - w) / 2;
                let y = mp.y + (12.0 * scale) as i32;
                let _ = win.set_position(PhysicalPosition::new(x, y));
            }
        }
        Err(e) => eprintln!("shotkey: 録画の小窓を作れませんでした: {e}"),
    }
}

fn close_rec_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("rec") {
        let _ = w.destroy();
    }
}

/// 小窓の「停止」ボタン
#[tauri::command]
fn stop_rec(app: AppHandle) {
    stop_recording(&app);
}

/// 録画を止めて、MP4 を閉じる (終わるまで待つ)。録画中でなければ、何もしない。
fn stop_recording_blocking(app: &AppHandle) {
    let session = app.state::<AppState>().rec.lock().unwrap().take();
    if session.is_some() {
        close_rec_window(app);
    }
    if let Some(s) = session {
        match s.stop() {
            Ok(p) => tooltip(app, &format!("shotkey — 録画を保存しました: {}", p.file_name().and_then(|s| s.to_str()).unwrap_or(""))),
            Err(e) => report_error(app, &format!("録画を保存できませんでした: {e}")),
        }
    }
}

fn stop_recording(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || stop_recording_blocking(&app));
}

fn report_error(app: &AppHandle, msg: &str) {
    eprintln!("shotkey: {msg}");
    tooltip(app, &format!("shotkey — 失敗しました: {msg}"));
}

// ---- 撮る ----

fn do_action(app: &AppHandle, action: Action) -> Result<(), String> {
    let state = app.state::<AppState>();
    let cfg = state.cfg.lock().unwrap().clone();

    // タイマーは、待ってから、それぞれの撮り方になる。
    // 待っているあいだに、撮りたいウィンドウを手前に出したり、メニューを開いたりする。
    let action = match action {
        Action::TimerWindow | Action::TimerMonitor | Action::TimerAll => {
            std::thread::sleep(std::time::Duration::from_secs(cfg.timer_secs as u64));
            match action {
                Action::TimerWindow => Action::Window,
                Action::TimerMonitor => Action::Monitor,
                _ => Action::All,
            }
        }
        a => a,
    };

    // 録画 (ウィンドウ / モニタ) は、画面を固めずに、すぐ始める
    match action {
        Action::RecWindow => return start_recording(app, record::Target::Window, None),
        Action::RecMonitor => {
            let (cx, cy) = capture::cursor_pos().ok_or("マウスの位置を取得できませんでした")?;
            let h = capture::monitor_handle_at(cx, cy).ok_or("マウスがどのモニタの上にもありません")?;
            return start_recording(app, record::Target::Monitor(h), None);
        }
        _ => {}
    }

    // ウィンドウは、固める前に調べる (固めたあとでは、手前のウィンドウが変わりうる)
    let fg = if action == Action::Window { capture::foreground_window() } else { None };
    let cursor = capture::cursor_pos();
    let desktop = capture::grab_desktop()?;
    let monitors = capture::monitors();

    let shot = match action {
        Action::All => desktop,
        Action::Monitor => {
            let (cx, cy) = cursor.ok_or("マウスの位置を取得できませんでした")?;
            let i = geom::monitor_at(&monitors, cx, cy).ok_or("マウスがどのモニタの上にもありません")?;
            desktop.crop(&monitors[i]).ok_or("モニタを切り出せませんでした")?
        }
        Action::Window => {
            let r = fg.ok_or("手前のウィンドウが見つかりません")?;
            desktop.crop(&r).ok_or("ウィンドウが画面の外にあります")?
        }
        Action::Region | Action::RecRegion => {
            let purpose = if action == Action::RecRegion { Purpose::Record } else { Purpose::Shot };
            *state.frozen.lock().unwrap() = Some(Frozen { desktop, monitors: monitors.clone(), purpose });
            open_overlays(app, &monitors, cursor)?;
            return Ok(()); // 保存 (録画の開始) は、選び終わったとき (finish_region)
        }
        Action::TimerWindow | Action::TimerMonitor | Action::TimerAll | Action::RecWindow | Action::RecMonitor => unreachable!(),
    };
    save_image(app, &shot)?;
    Ok(())
}

/// ホットキーが押されたとき。撮影は別のスレッドでやる (イベントの処理を止めない)
fn run_action(app: &AppHandle, action: Action) {
    let state = app.state::<AppState>();
    // 録画のホットキーは、録画中なら「停止」になる
    if matches!(action, Action::RecWindow | Action::RecMonitor | Action::RecRegion) && state.rec.lock().unwrap().is_some() {
        stop_recording(app);
        return;
    }
    if state.busy.swap(true, Ordering::SeqCst) {
        return; // 撮影中
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let result = do_action(&app, action);
        let keep_busy = matches!(action, Action::Region | Action::RecRegion) && result.is_ok(); // 範囲選択は、選び終わるまで続く
        if let Err(e) = result {
            report_error(&app, &e);
        }
        if !keep_busy {
            app.state::<AppState>().busy.store(false, Ordering::SeqCst);
        }
    });
}

// ---- 範囲選択の重ね画面 ----

fn open_overlays(app: &AppHandle, monitors: &[geom::Rect], cursor: Option<(i32, i32)>) -> Result<(), String> {
    let focus_idx = cursor.and_then(|(x, y)| geom::monitor_at(monitors, x, y)).unwrap_or(0);
    for (i, m) in monitors.iter().enumerate() {
        let win = WebviewWindowBuilder::new(app, format!("ov{i}"), WebviewUrl::App(format!("overlay.html?m={i}").into()))
            .title("shotkey overlay")
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .visible(false)
            .build()
            .map_err(|e| format!("範囲選択の画面を作れませんでした: {e}"))?;
        // 位置 → 大きさ → 位置 の順。モニタの拡大率が違うと、動かした直後に大きさが変わることがあるため
        let pos = PhysicalPosition::new(m.x, m.y);
        let size = PhysicalSize::new(m.w as u32, m.h as u32);
        let _ = win.set_position(pos);
        let _ = win.set_size(size);
        let _ = win.set_position(pos);
        let _ = win.show();
        if i == focus_idx {
            let _ = win.set_focus();
        }
    }
    Ok(())
}

fn close_overlays(app: &AppHandle) {
    for (label, win) in app.webview_windows() {
        if label.starts_with("ov") {
            let _ = win.destroy();
        }
    }
}

fn end_selection(app: &AppHandle) {
    *app.state::<AppState>().frozen.lock().unwrap() = None;
    close_overlays(app);
    app.state::<AppState>().busy.store(false, Ordering::SeqCst);
}

/// 重ね画面に渡す画素。先頭 8 バイトが 幅・高さ (u32 リトルエンディアン)、続きが RGBA。
#[tauri::command]
fn get_snap(m: usize, state: State<AppState>) -> Result<Response, String> {
    let frozen = state.frozen.lock().unwrap();
    let f = frozen.as_ref().ok_or("固めた画面がありません")?;
    let rect = f.monitors.get(m).ok_or("モニタの番号が正しくありません")?;
    let part = f.desktop.crop(rect).ok_or("モニタを切り出せませんでした")?;
    let mut out = Vec::with_capacity(8 + part.data.len());
    out.extend_from_slice(&(part.w as u32).to_le_bytes());
    out.extend_from_slice(&(part.h as u32).to_le_bytes());
    out.extend_from_slice(&part.to_rgba());
    Ok(Response::new(out))
}

/// 選び終わった。x, y, w, h は、そのモニタの左上からの物理ピクセル。
#[tauri::command]
fn finish_region(app: AppHandle, m: usize, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
    let (purpose, shot, mon) = {
        let state = app.state::<AppState>();
        let frozen = state.frozen.lock().unwrap();
        let f = frozen.as_ref().ok_or("固めた画面がありません")?;
        let mon = *f.monitors.get(m).ok_or("モニタの番号が正しくありません")?;
        let shot = if f.purpose == Purpose::Shot {
            f.desktop.crop(&geom::Rect::new(mon.x + x, mon.y + y, w, h))
        } else {
            None
        };
        (f.purpose, shot, mon)
    };
    // 重ね画面を閉じるのは、このコマンドが返ったあと (自分の窓を、自分の処理の中で壊さない)
    let app2 = app.clone();
    std::thread::spawn(move || {
        end_selection(&app2);
        match purpose {
            Purpose::Shot => {
                if let Some(img) = shot {
                    if let Err(e) = save_image(&app2, &img) {
                        report_error(&app2, &e);
                    }
                }
            }
            Purpose::Record => {
                // 重ね画面が録画に写り込まないように、閉じ終わるのを少し待つ
                std::thread::sleep(std::time::Duration::from_millis(400));
                // H.264 は、幅と高さが偶数でないといけない
                let (w, h) = (w & !1, h & !1);
                let result = if w <= 0 || h <= 0 {
                    Err("範囲が小さすぎます".to_string())
                } else {
                    capture::monitor_handle_at(mon.x + mon.w / 2, mon.y + mon.h / 2)
                        .ok_or_else(|| "モニタが見つかりません".to_string())
                        .and_then(|handle| {
                            start_recording(
                                &app2,
                                record::Target::Monitor(handle),
                                Some((x as u32, y as u32, (x + w) as u32, (y + h) as u32)),
                            )
                        })
                };
                if let Err(e) = result {
                    report_error(&app2, &e);
                }
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn cancel_region(app: AppHandle) {
    std::thread::spawn(move || end_selection(&app));
}

// ---- ホットキー ----

/// 設定のとおりにホットキーを登録し直す。登録できなかったものを、理由つきで返す。
fn apply_hotkeys(app: &AppHandle, cfg: &Config) -> Vec<String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut map = HashMap::new();
    let mut errors = Vec::new();
    let hk = &cfg.hotkeys;
    for (action, label, text) in [
        (Action::All, "全体", &hk.all),
        (Action::Monitor, "モニタ", &hk.monitor),
        (Action::Window, "ウィンドウ", &hk.window),
        (Action::Region, "範囲", &hk.region),
        (Action::TimerWindow, "タイマー (ウィンドウ)", &hk.timer_window),
        (Action::TimerMonitor, "タイマー (モニタ)", &hk.timer_monitor),
        (Action::TimerAll, "タイマー (全体)", &hk.timer_all),
        (Action::RecWindow, "録画 (ウィンドウ)", &hk.rec_window),
        (Action::RecMonitor, "録画 (モニタ)", &hk.rec_monitor),
        (Action::RecRegion, "録画 (範囲)", &hk.rec_region),
    ] {
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        match text.parse::<Shortcut>() {
            Err(e) => errors.push(format!("{label} ({text}): 読み取れません — {e}")),
            Ok(sc) => match gs.register(sc) {
                Ok(()) => {
                    map.insert(sc.id(), action);
                }
                Err(e) => errors.push(format!("{label} ({text}): 登録できません (ほかのアプリが使っている可能性があります) — {e}")),
            },
        }
    }
    *app.state::<AppState>().keys.lock().unwrap() = map;
    errors
}

// ---- 設定画面とのやりとり ----

#[tauri::command]
fn get_config(state: State<AppState>) -> Config {
    state.cfg.lock().unwrap().clone()
}

/// 設定を保存して、ホットキーを登録し直す。登録できなかったホットキーの説明を返す (空なら全部できた)。
#[tauri::command]
fn set_config(app: AppHandle, state: State<AppState>, cfg: Config) -> Result<Vec<String>, String> {
    let cfg = cfg.normalized();
    config::save(&state.cfg_path, &cfg)?;
    *state.cfg.lock().unwrap() = cfg.clone();
    let errors = apply_hotkeys(&app, &cfg);
    *state.key_errors.lock().unwrap() = errors.clone();
    Ok(errors)
}

#[tauri::command]
fn get_hotkey_errors(state: State<AppState>) -> Vec<String> {
    state.key_errors.lock().unwrap().clone()
}

#[tauri::command]
fn default_config() -> Config {
    Config::default()
}

/// 保存先の、いまの実際の場所 (空欄のとき、どこに入るかを見せる)
#[tauri::command]
fn resolved_save_dir(app: AppHandle, state: State<AppState>) -> String {
    let cfg = state.cfg.lock().unwrap().clone();
    save_dir(&app, &cfg).display().to_string()
}

#[tauri::command]
fn open_save_dir(app: AppHandle) {
    open_folder(&app);
}

fn open_folder(app: &AppHandle) {
    let cfg = app.state::<AppState>().cfg.lock().unwrap().clone();
    let dir = save_dir(app, &cfg);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::process::Command::new("explorer").arg(dir).spawn();
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

// ---- 起動 ----

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let action = app.state::<AppState>().keys.lock().unwrap().get(&shortcut.id()).copied();
                    if let Some(a) = action {
                        run_action(app, a);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let cfg_path = app.path().app_config_dir()?.join("config.json");
            let cfg = config::load(&cfg_path);
            app.manage(AppState {
                cfg: Mutex::new(cfg.clone()),
                cfg_path,
                frozen: Mutex::new(None),
                keys: Mutex::new(HashMap::new()),
                busy: AtomicBool::new(false),
                key_errors: Mutex::new(Vec::new()),
                rec: Mutex::new(None),
            });

            // トレイ
            let open = MenuItem::with_id(app, "open", "設定を開く", true, None::<&str>)?;
            let folder = MenuItem::with_id(app, "folder", "保存先を開く", true, None::<&str>)?;
            let stop = MenuItem::with_id(app, "stop_rec", "録画を停止", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &folder, &stop, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().cloned().ok_or("アイコンがありません")?)
                .tooltip("shotkey")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, ev| match ev.id.as_ref() {
                    "open" => show_main(app),
                    "folder" => open_folder(app),
                    "stop_rec" => stop_recording(app),
                    "quit" => {
                        stop_recording_blocking(app); // 録画中に終わっても、MP4 を壊さない
                        app.exit(0)
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, ev| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            // 登録できなかったホットキーは、設定画面が開いたときに get_hotkey_errors で見せる
            let errors = apply_hotkeys(app.handle(), &cfg);
            for e in &errors {
                eprintln!("shotkey: {e}");
            }
            *app.state::<AppState>().key_errors.lock().unwrap() = errors;
            Ok(())
        })
        .on_window_event(|window, event| {
            // 設定の窓を閉じても、終わらない (トレイに残る)。終わるのは、トレイの「終了」
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            set_config,
            get_hotkey_errors,
            default_config,
            resolved_save_dir,
            open_save_dir,
            get_snap,
            finish_region,
            cancel_region,
            stop_rec
        ])
        .run(tauri::generate_context!())
        .expect("shotkey の起動に失敗");
}
