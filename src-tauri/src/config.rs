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
    pub rec_window: String,    // 録画 開始/停止: 手前にあるウィンドウ
    pub rec_monitor: String,   // 録画 開始/停止: マウスのあるモニタ
    pub rec_region: String,    // 録画 開始/停止: 範囲を選んで、そこだけ
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
            rec_window: "Ctrl+Alt+8".into(),
            rec_monitor: "Ctrl+Alt+9".into(),
            rec_region: "Ctrl+Alt+0".into(),
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
    pub video_template: String, // 動画のファイル名 ({n} {date} {time})
    pub video_fps: u32,
    pub video_mbps: u32,        // 画質: ビットレート (Mbps)
    pub video_cursor: bool,     // マウスカーソルを録画に入れる
    pub video_audio: bool,      // パソコンから出ている音 (システム音) を録る
    pub video_limit_secs: u32,  // 録画を自動で止める秒数 (0 = 止めない)
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
            video_template: "rec_{date}_{n}".into(),
            video_fps: 30,
            video_mbps: 8,
            video_cursor: true,
            video_audio: true,
            video_limit_secs: 0,
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
        self.video_fps = self.video_fps.clamp(10, 60);
        self.video_mbps = self.video_mbps.clamp(1, 100);
        self.video_limit_secs = self.video_limit_secs.min(24 * 3600);
        if self.video_template.trim().is_empty() { self.video_template = Config::default().video_template; }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_still_loads() {
        // v0.1.x の config.json: 廃止した項目 (timer, timer_mode, beep) が残っていて、動画の項目が無い
        let old = r#"{ "save_dir": "D:\\shots", "template": "cap_{n}", "beep": true, "timer_mode": "window",
            "hotkeys": { "all": "Ctrl+Alt+1", "timer": "Ctrl+Alt+5" } }"#;
        let c: Config = serde_json::from_str(old).unwrap();
        let c = c.normalized();
        assert_eq!(c.save_dir, "D:\\shots");
        assert_eq!(c.template, "cap_{n}");
        assert_eq!(c.hotkeys.all, "Ctrl+Alt+1");
        // 無い項目は、初期値で補われる
        assert_eq!(c.hotkeys.timer_window, "Ctrl+Alt+5");
        assert_eq!(c.hotkeys.rec_region, "Ctrl+Alt+0");
        assert_eq!(c.video_fps, 30);
        assert!(c.video_audio);
        assert_eq!(c.video_limit_secs, 0);
        assert!(c.sound);
    }

    #[test]
    fn ui_payload_parses() {
        // 設定画面が送ってくる形 (数値は数値)
        let j = r#"{ "save_dir": "", "template": "shot_{date}_{n}", "digits": 3, "format": "jpg", "jpg_quality": 90,
            "clipboard": true, "sound": false, "timer_secs": 5, "video_template": "rec_{n}", "video_fps": 60,
            "video_mbps": 12, "video_cursor": false,
            "hotkeys": { "all": "", "monitor": "", "window": "", "region": "", "timer_window": "", "timer_monitor": "",
                         "timer_all": "", "rec_window": "Ctrl+Alt+8", "rec_monitor": "", "rec_region": "" } }"#;
        let c: Config = serde_json::from_str(j).unwrap();
        assert_eq!((c.video_fps, c.video_mbps, c.video_cursor), (60, 12, false));
        assert_eq!(c.hotkeys.rec_window, "Ctrl+Alt+8");
    }

    #[test]
    fn normalized_clamps() {
        let mut c = Config::default();
        c.video_fps = 500;
        c.video_mbps = 0;
        c.video_limit_secs = 999_999;
        c.digits = 99;
        c.format = "bmp".into();
        c.video_template = "  ".into();
        let c = c.normalized();
        assert_eq!((c.video_fps, c.video_mbps, c.digits), (60, 1, 9));
        assert_eq!(c.video_limit_secs, 86400);
        assert_eq!(c.format, "png");
        assert_eq!(c.video_template, "rec_{date}_{n}");
    }
}
