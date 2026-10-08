//! 設定。app_config_dir/config.json に 1 つの JSON として持つ。足りない項目は初期値で補う。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Hotkeys {
    pub all: String,     // デスクトップ全体 (全モニタ)
    pub monitor: String, // マウスのあるモニタ
    pub window: String,  // いま手前にあるウィンドウ
    pub region: String,  // 範囲を選ぶ (先に画面を固めてから選ぶ)
    pub timer_window: String,  // 数秒待ってから、そのとき手前にあるウィンドウを撮る
    pub timer_monitor: String, // 数秒待ってから、そのときマウスのあるモニタを撮る
    pub timer_all: String,     // 数秒待ってから、デスクトップ全体を撮る
}

impl Default for Hotkeys {
    fn default() -> Self {
        Hotkeys {
            all: "Ctrl+Alt+1".into(),
            monitor: "Ctrl+Alt+2".into(),
            window: "Ctrl+Alt+3".into(),
            region: "Ctrl+Alt+4".into(),
            timer_window: "Ctrl+Alt+5".into(),
            timer_monitor: "Ctrl+Alt+6".into(),
            timer_all: "Ctrl+Alt+7".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub save_dir: String,    // 空なら ピクチャ\shotkey
    pub template: String,    // {n} 連番, {date} 日付, {time} 時刻
    pub digits: usize,       // 連番の桁数
    pub format: String,      // "png" | "jpg"
    pub jpg_quality: u8,
    pub clipboard: bool,     // 撮ったらクリップボードにも入れる
    pub sound: bool,         // 撮れた瞬間に、シャッター音を鳴らす
    pub timer_secs: u32,
    pub hotkeys: Hotkeys,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            save_dir: String::new(),
            template: "shot_{date}_{n}".into(),
            digits: 3,
            format: "png".into(),
            jpg_quality: 90,
            clipboard: false,
            sound: true,
            timer_secs: 5,
            hotkeys: Hotkeys::default(),
        }
    }
}

impl Config {
    /// 壊れた値を、使える範囲に直す
    pub fn normalized(mut self) -> Config {
        if self.format != "png" && self.format != "jpg" { self.format = "png".into(); }
        self.jpg_quality = self.jpg_quality.clamp(1, 100);
        self.digits = self.digits.clamp(1, 9);
        self.timer_secs = self.timer_secs.min(120);
        if self.template.trim().is_empty() { self.template = Config::default().template; }
        self
    }
}

pub fn load(path: &PathBuf) -> Config {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Config>(&s).ok())
        .unwrap_or_default()
        .normalized()
}

pub fn save(path: &PathBuf, cfg: &Config) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}
