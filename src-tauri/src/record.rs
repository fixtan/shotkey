//! 録画。windows-capture の Graphics Capture で画面を取り、Media Foundation の H.264 で MP4 にする。
//! 音は、パソコンから出ている音 (システム音) を WASAPI のループバックで録って、AAC で MP4 に入れる。ffmpeg は使わない。
//!
//! 録画するのは、ウィンドウ 1 つ、モニタ 1 つ、またはモニタの中の範囲。
//! 開始 (start) したら、別のスレッドで撮り続ける。停止は stop で、撮影を止めてから、エンコーダーを閉じる。
//! (この順を逆にすると、MP4 が壊れる)

use std::path::PathBuf;

/// 録画する相手
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Target {
    /// いま手前にあるウィンドウ
    Window,
    /// モニタ (HMONITOR の値)
    Monitor(usize),
}

#[cfg_attr(not(windows), allow(dead_code))]
pub struct Options {
    pub path: PathBuf,
    pub fps: u32,
    pub bitrate: u32,
    pub cursor: bool,
    /// システム音を録る
    pub audio: bool,
    /// モニタを録るとき、その中の範囲だけを録る: (x0, y0, x1, y1) モニタの左上からの物理ピクセル
    pub crop: Option<(u32, u32, u32, u32)>,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};
    use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
    use windows_capture::encoder::{
        AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder, VideoSettingsSubType,
    };
    use windows_capture::frame::Frame;
    use windows_capture::graphics_capture_api::InternalCaptureControl;
    use windows_capture::monitor::Monitor;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
        SecondaryWindowSettings, Settings,
    };
    use windows_capture::window::Window;

    type BoxError = Box<dyn std::error::Error + Send + Sync>;

    /// 映像のスレッドに渡すもの
    struct Flags {
        opts: Options,
        /// 音のスレッドから届く PCM (48kHz / 16bit / 2ch)。音を録らないときは None
        audio_rx: Option<Receiver<Vec<u8>>>,
        /// エンコーダーができたら true にする。音のスレッドは、それまでの音を捨てて、時計を合わせ直す
        started: Arc<AtomicBool>,
    }

    struct Recorder {
        encoder: Option<VideoEncoder>,
        opts: Options,
        audio_rx: Option<Receiver<Vec<u8>>>,
        started: Arc<AtomicBool>,
    }

    impl Recorder {
        /// 最初のフレームが来たときに、エンコーダーを作る (大きさは、そのフレームで決める)
        fn make_encoder(&self, w: u32, h: u32) -> Result<VideoEncoder, BoxError> {
            // H.264 は、幅と高さが偶数でないといけない
            let (w, h) = (w & !1, h & !1);
            Ok(VideoEncoder::new(
                VideoSettingsBuilder::new(w, h)
                    .sub_type(VideoSettingsSubType::H264) // 標準は HEVC (H.265)。再生できない環境が多いので、H.264 にする
                    .bitrate(self.opts.bitrate)
                    .frame_rate(self.opts.fps),
                AudioSettingsBuilder::default().disabled(self.audio_rx.is_none()),
                ContainerSettingsBuilder::default(),
                &self.opts.path,
            )?)
        }
    }

    impl Recorder {
        /// 届いている音を、エンコーダーに渡す
        fn pump_audio(&mut self) {
            if let (Some(rx), Some(enc)) = (&self.audio_rx, self.encoder.as_mut()) {
                while let Ok(chunk) = rx.try_recv() {
                    let _ = enc.send_audio_buffer(&chunk, 0); // 音の失敗で、録画は止めない
                }
            }
        }
    }

    impl GraphicsCaptureApiHandler for Recorder {
        type Flags = Flags;
        type Error = BoxError;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Recorder { encoder: None, opts: ctx.flags.opts, audio_rx: ctx.flags.audio_rx, started: ctx.flags.started })
        }

        fn on_frame_arrived(&mut self, frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), Self::Error> {
            // モニタの外にはみ出さないように、範囲を整える
            let crop = self.opts.crop.and_then(|(x0, y0, x1, y1)| {
                let (x1, y1) = (x1.min(frame.width()), y1.min(frame.height()));
                if x0 < x1 && y0 < y1 { Some((x0, y0, x1, y1)) } else { None }
            });
            let (w, h) = match crop {
                Some((x0, y0, x1, y1)) => (x1 - x0, y1 - y0),
                None => (frame.width(), frame.height()),
            };
            if self.encoder.is_none() {
                self.encoder = Some(self.make_encoder(w, h)?);
                self.started.store(true, Ordering::SeqCst);
            }
            self.pump_audio();

            match crop {
                None => {
                    // 全体: GPU のまま渡せるので速い
                    self.encoder.as_mut().unwrap().send_frame(frame)?;
                }
                Some((x0, y0, x1, y1)) => {
                    // 範囲: 切り出して CPU に降ろす。時刻は、先に取る (buffer_crop がフレームを借りるため)
                    let ts = frame.timestamp()?.Duration;
                    let buf = frame.buffer_crop(x0, y0, x1, y1)?;
                    let (bw, bh) = (buf.width() as usize, buf.height() as usize);
                    let mut tmp = Vec::new();
                    let data = buf.as_nopadding_buffer(&mut tmp);
                    // send_frame_buffer は「下の行から並んだ BGRA」を要求する。行の順を逆にする。
                    let mut flipped = Vec::with_capacity(data.len());
                    for row in (0..bh).rev() {
                        flipped.extend_from_slice(&data[row * bw * 4..(row + 1) * bw * 4]);
                    }
                    self.encoder.as_mut().unwrap().send_frame_buffer(&flipped, ts)?;
                }
            }
            Ok(())
        }

        fn on_closed(&mut self) -> Result<(), Self::Error> {
            Ok(()) // 録画しているウィンドウが閉じられた。MP4 は、stop で閉じる
        }
    }

    /// システム音を録るスレッド。既定の再生デバイスの音を、ループバックで拾う。
    /// 何も鳴っていないときは WASAPI が何も返さないので、時計に合わせて無音を足す (映像とずれないように)。
    fn audio_loop(stop: Arc<AtomicBool>, started: Arc<AtomicBool>, tx: Sender<Vec<u8>>, ready: Sender<Result<(), String>>) {
        use wasapi::{initialize_mta, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

        const RATE: u64 = 48_000;
        const FRAME: usize = 4; // 16bit × 2ch
        let _ = initialize_mta();

        let init = || -> Result<_, Box<dyn std::error::Error>> {
            let enumerator = DeviceEnumerator::new()?;
            let device = enumerator.get_default_device(&Direction::Render)?;
            let mut client = device.get_iaudioclient()?;
            let fmt = WaveFormat::new(16, 16, &SampleType::Int, RATE as usize, 2, None);
            let (_default, min) = client.get_device_period()?;
            let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: min };
            // 再生デバイスを、録音の向きで開く = ループバック
            client.initialize_client(&fmt, &Direction::Capture, &mode)?;
            let event = client.set_get_eventhandle()?;
            let cap = client.get_audiocaptureclient()?;
            client.start_stream()?;
            Ok((client, cap, event))
        };
        let (client, cap, event) = match init() {
            Ok(v) => {
                let _ = ready.send(Ok(()));
                v
            }
            Err(e) => {
                let _ = ready.send(Err(e.to_string()));
                return;
            }
        };

        let mut queue: VecDeque<u8> = VecDeque::new();
        let mut t0 = Instant::now();
        let mut sent: u64 = 0; // 送った フレーム数
        while !stop.load(Ordering::SeqCst) {
            let _ = event.wait_for_event(20); // 時間切れでも、そのまま進む (無音を足すため)
            for _ in 0..64 {
                match cap.get_next_packet_size() {
                    Ok(Some(n)) if n > 0 => {
                        if cap.read_from_device_to_deque(&mut queue).is_err() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            if !started.load(Ordering::SeqCst) {
                // 映像がまだ来ていない。この間の音は捨てて、映像が始まった時点を 0 にする
                queue.clear();
                t0 = Instant::now();
                sent = 0;
                continue;
            }
            // 経った時間に対して音が足りなければ、無音で埋める (30 ms 以上遅れたとき)
            let pending = (queue.len() / FRAME) as u64;
            let expected = t0.elapsed().as_micros() as u64 * RATE / 1_000_000;
            if sent + pending + RATE * 30 / 1000 < expected {
                let gap = (expected - sent - pending) as usize;
                queue.extend(std::iter::repeat(0u8).take(gap * FRAME));
            }
            let n = queue.len() / FRAME * FRAME;
            if n > 0 {
                let chunk: Vec<u8> = queue.drain(..n).collect();
                sent += (chunk.len() / FRAME) as u64;
                if tx.send(chunk).is_err() {
                    break;
                }
            }
        }
        let _ = client.stop_stream();
    }

    pub struct Session {
        control: CaptureControl<Recorder, BoxError>,
        finish: Box<dyn FnOnce() -> Result<(), String> + Send>,
        audio_stop: Arc<AtomicBool>,
        audio_thread: Option<JoinHandle<()>>,
        pub path: PathBuf,
        /// 音を録ろうとして、デバイスが開けなかったときの理由
        pub audio_error: Option<String>,
    }

    fn begin<T>(item: T, opts: Options) -> Result<Session, String>
    where
        T: TryInto<windows_capture::settings::GraphicsCaptureItemType> + Send + 'static,
    {
        let path = opts.path.clone();
        let cursor = if opts.cursor { CursorCaptureSettings::WithCursor } else { CursorCaptureSettings::WithoutCursor };
        // 画面が変わるたびに全部を受けると重いので、fps に合わせて間引く
        let interval = MinimumUpdateIntervalSettings::Custom(Duration::from_millis((1000 / opts.fps.max(1)) as u64));

        // 先に音のデバイスが開けるかを確かめる。開けたときだけ、MP4 に音の入れ物を作る
        let audio_stop = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));
        let mut audio_rx = None;
        let mut audio_thread = None;
        let mut audio_error = None;
        if opts.audio {
            let (tx, rx) = mpsc::channel::<Vec<u8>>();
            let (rtx, rrx) = mpsc::channel::<Result<(), String>>();
            let (stop2, started2) = (audio_stop.clone(), started.clone());
            let th = std::thread::spawn(move || audio_loop(stop2, started2, tx, rtx));
            match rrx.recv_timeout(Duration::from_secs(3)) {
                Ok(Ok(())) => {
                    audio_rx = Some(rx);
                    audio_thread = Some(th);
                }
                Ok(Err(e)) => audio_error = Some(e),
                Err(_) => {
                    audio_stop.store(true, Ordering::SeqCst);
                    audio_error = Some("音のデバイスが応答しません".into());
                }
            }
        }

        let settings = Settings::new(
            item,
            cursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            interval,
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8, // 範囲録画で send_frame_buffer に渡すため、BGRA
            Flags { opts, audio_rx, started },
        );
        let control = match Recorder::start_free_threaded(settings) {
            Ok(c) => c,
            Err(e) => {
                audio_stop.store(true, Ordering::SeqCst);
                return Err(e.to_string());
            }
        };
        let handler = control.callback();
        let finish = move || -> Result<(), String> {
            let mut rec = handler.lock();
            rec.pump_audio(); // 残っている音を、閉じる前に渡す
            match rec.encoder.take() {
                Some(e) => e.finish().map_err(|e| e.to_string()),
                None => Ok(()), // 1 枚もフレームが来なかった
            }
        };
        Ok(Session { control, finish: Box::new(finish), audio_stop, audio_thread, path, audio_error })
    }

    pub fn start(target: Target, opts: Options) -> Result<Session, String> {
        match target {
            Target::Window => begin(Window::foreground().map_err(|e| e.to_string())?, opts),
            Target::Monitor(h) => begin(Monitor::from_raw_hmonitor(h as *mut std::ffi::c_void), opts),
        }
    }

    impl Session {
        /// 撮影を止めてから、エンコーダーを閉じる。保存したファイルを返す。
        pub fn stop(self) -> Result<PathBuf, String> {
            let Session { control, finish, audio_stop, audio_thread, path, .. } = self;
            let stopped = control.stop().map_err(|e| e.to_string());
            // 音のスレッドも止める (映像を止めたあと、エンコーダーを閉じる前)
            audio_stop.store(true, Ordering::SeqCst);
            if let Some(t) = audio_thread {
                let _ = t.join();
            }
            let finished = finish();
            stopped?;
            finished?;
            if path.exists() { Ok(path) } else { Err("映像が 1 枚も届きませんでした".into()) }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub struct Session {
        pub path: PathBuf,
        #[allow(dead_code)]
        pub audio_error: Option<String>,
    }

    pub fn start(_target: Target, _opts: Options) -> Result<Session, String> {
        Err("録画は Windows 専用です".into())
    }

    impl Session {
        pub fn stop(self) -> Result<PathBuf, String> {
            Ok(self.path)
        }
    }
}

pub use imp::*;
