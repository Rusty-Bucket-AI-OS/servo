/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The Rusty Bucket content process: Servo as a WASI program that paints with the software GL into memory.
//!
//! - `servo-content URL OUT.png [WIDTH HEIGHT]`: loads the page, waits for the load and a frame, writes the picture to `OUT.png`
//!   and prints how long that took.
//! - `servo-content --stdio [URL]`: a browser tab. It speaks `rbcp` (see `protocol.rs`) on its standard input and output: rectangles of
//!   the picture, the title, the address and the loading state go out; navigation, resizes, keys, the pointer and the wheel come in.

mod protocol;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io::Write;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use dpi::PhysicalSize;
use embedder_traits::EventLoopWaker;
use servo::{
    Code, DeviceIntRect, DeviceIntSize, DevicePoint, InputEvent, Key, KeyState, KeyboardEvent, LoadStatus, Location, Modifiers,
    MouseButton, MouseButtonAction, MouseButtonEvent, MouseMoveEvent, Preferences, RenderingContext, ServoBuilder,
    SoftwareRenderingContext, WebView, WebViewBuilder, WebViewDelegate, WebViewPoint, WheelDelta, WheelEvent, WheelMode,
};
use url::Url;

use protocol::Command;

mod resources;

/// Servo wakes the embedder when it has work for `spin_event_loop`; the loop sleeps on this until then (or until a command arrives).
#[derive(Clone, Default)]
struct Waker(Arc<(Mutex<bool>, Condvar)>);

impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        let (flag, cv) = &*self.0;
        *flag.lock().unwrap_or_else(|p| p.into_inner()) = true;
        cv.notify_one();
    }
}

impl Waker {
    fn wait(&self, timeout: Duration) {
        let (flag, cv) = &*self.0;
        let mut woken = flag.lock().unwrap_or_else(|p| p.into_inner());
        if !*woken {
            woken = cv.wait_timeout(woken, timeout).unwrap_or_else(|p| p.into_inner()).0;
        }
        *woken = false;
    }
}

#[derive(Default)]
struct Delegate {
    loaded: Cell<bool>,
    frames: Cell<u32>,
    frame_after_load: Cell<bool>,
    /// A frame was painted and has not been sent yet.
    painted: Cell<bool>,
    changed: Cell<bool>,
    title: RefCell<String>,
    url: RefCell<String>,
}

impl WebViewDelegate for Delegate {
    fn notify_new_frame_ready(&self, webview: WebView) {
        self.frames.set(self.frames.get() + 1);
        if self.loaded.get() {
            self.frame_after_load.set(true);
        }
        webview.paint();
        self.painted.set(true);
    }

    fn notify_load_status_changed(&self, _webview: WebView, status: LoadStatus) {
        self.loaded.set(status == LoadStatus::Complete);
        self.changed.set(true);
    }

    fn notify_page_title_changed(&self, _webview: WebView, title: Option<String>) {
        *self.title.borrow_mut() = title.unwrap_or_default();
        self.changed.set(true);
    }

    fn notify_url_changed(&self, _webview: WebView, url: Url) {
        *self.url.borrow_mut() = url.to_string();
        self.changed.set(true);
    }
}

fn preferences() -> Preferences {
    let mut preferences = Preferences::default();
    // One content process holds many threads; keep the pools small (the process may start at most 64).
    preferences.layout_threads = 2;
    preferences.thread_pool_workers_max = 2;
    preferences.thread_pool_async_runtime_workers_max = 2;
    preferences.thread_pool_canvas_workers = 0;
    preferences.thread_pool_webrender_workers_max = 2;
    preferences.perf_thread_boost_enabled = false;
    preferences.network_http_proxy_uri = String::new();
    preferences.network_https_proxy_uri = String::new();
    // There is no operating system certificate store here: trust the web's roots (webpki-roots) and, if given, the PEM file
    // named by RB_CA_BUNDLE (the system's `/certs` folder, or the CA of a proxy that inspects TLS).
    preferences.network_use_webpki_roots = true;
    preferences
}

fn opts() -> servo::Opts {
    let mut opts = servo::Opts::default();
    opts.certificate_path = std::env::var("RB_CA_BUNDLE").ok().filter(|p| !p.is_empty());
    opts
}

/// Log lines go to standard error (standard output carries `rbcp`); `RB_LOG=trace|debug|info|warn|error` (default warn).
struct StderrLogger(log::LevelFilter);

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.0
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {}
}

fn init_logging() {
    let level = std::env::var("RB_LOG").ok().and_then(|v| v.parse().ok()).unwrap_or(log::LevelFilter::Warn);
    let logger: &'static StderrLogger = Box::leak(Box::new(StderrLogger(level)));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(level);
    }
}

fn main() {
    init_logging();
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--stdio") {
        return stdio(args.get(2).map(String::as_str));
    }
    let (Some(url), Some(out)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: servo-content URL OUT.png [WIDTH HEIGHT] | servo-content --stdio [URL]");
        std::process::exit(2);
    };
    screenshot(url, out, args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1920), args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1080));
}

fn parse_url(text: &str) -> Url {
    Url::parse(text).unwrap_or_else(|e| {
        eprintln!("bad URL {text:?}: {e}");
        std::process::exit(2)
    })
}

fn screenshot(url: &str, out: &str, width: u32, height: u32) {
    let url = parse_url(url);
    let started = Instant::now();
    let rendering_context =
        Rc::new(SoftwareRenderingContext::new(PhysicalSize::new(width, height)).expect("invariant: the software GL starts"));
    rendering_context.make_current().expect("invariant: the software GL is current");
    let waker = Waker::default();
    let servo = ServoBuilder::default().opts(opts()).preferences(preferences()).event_loop_waker(Box::new(waker.clone())).build();
    let delegate = Rc::new(Delegate::default());
    let webview = WebViewBuilder::new(&servo, rendering_context.clone()).url(url.clone()).delegate(delegate.clone()).build();
    eprintln!("servo-content: started in {} ms, loading {url}", started.elapsed().as_millis());
    let mut load_ms = None;
    loop {
        servo.spin_event_loop();
        if load_ms.is_none() && delegate.loaded.get() {
            load_ms = Some(started.elapsed().as_millis());
            eprintln!("servo-content: load complete after {} ms", load_ms.unwrap_or_default());
        }
        if delegate.frame_after_load.get() {
            break;
        }
        waker.wait(Duration::from_millis(20));
        if started.elapsed() > Duration::from_secs(300) {
            eprintln!("servo-content: gave up after 300 s");
            std::process::exit(1);
        }
    }
    let frame_ms = started.elapsed().as_millis();
    let full = DeviceIntRect::from_size(DeviceIntSize::new(width as i32, height as i32));
    let Some(image) = rendering_context.read_to_image(full) else {
        eprintln!("servo-content: no frame to read");
        std::process::exit(1);
    };
    if let Err(e) = image.save(out) {
        eprintln!("servo-content: cannot write {out}: {e}");
        std::process::exit(1);
    }
    println!(
        "servo-content: title {:?}, load {} ms, first frame after load {} ms, {} frames, wrote {out}",
        delegate.title.borrow(),
        load_ms.unwrap_or_default(),
        frame_ms,
        delegate.frames.get()
    );
    drop(webview);
    drop(servo);
}

/// The part of `now` that differs from `before` (same size, BGRA, bottom row first): its box in top-down picture coordinates,
/// and its pixels top row first.
fn changed_rect(before: &[u8], now: &[u8], width: usize, height: usize) -> Option<(usize, usize, usize, usize, Vec<u8>)> {
    let stride = width * 4;
    let (mut top, mut bottom, mut left, mut right) = (usize::MAX, 0, usize::MAX, 0);
    for row in 0..height {
        let (a, b) = (&before[row * stride..(row + 1) * stride], &now[row * stride..(row + 1) * stride]);
        if a == b {
            continue;
        }
        let first = a.iter().zip(b).position(|(x, y)| x != y).unwrap_or(0) / 4;
        let last = a.iter().zip(b).rposition(|(x, y)| x != y).unwrap_or(0) / 4;
        // `row` counts from the bottom of the picture.
        let y = height - 1 - row;
        top = top.min(y);
        bottom = bottom.max(y);
        left = left.min(first);
        right = right.max(last);
    }
    if top == usize::MAX {
        return None;
    }
    let (w, h) = (right - left + 1, bottom - top + 1);
    let mut pixels = Vec::with_capacity(w * h * 4);
    for y in top..=bottom {
        let row = height - 1 - y;
        pixels.extend_from_slice(&now[row * stride + left * 4..row * stride + (right + 1) * 4]);
    }
    Some((left, top, w, h, pixels))
}

fn key_event(down: bool, mods: u8, key: &str, text: &str) -> InputEvent {
    let mut modifiers = Modifiers::empty();
    for (bit, m) in [(1, Modifiers::SHIFT), (2, Modifiers::CONTROL), (4, Modifiers::ALT), (8, Modifiers::META)] {
        if mods & bit != 0 {
            modifiers |= m;
        }
    }
    let key = if !text.is_empty() && key.chars().count() == 1 { Key::Character(text.to_owned()) } else { key.parse::<Key>().unwrap_or_else(|_| Key::Character(key.to_owned())) };
    let state = if down { KeyState::Down } else { KeyState::Up };
    InputEvent::Keyboard(KeyboardEvent::new_without_event(state, key, Code::Unidentified, Location::Standard, modifiers, false, false))
}

fn stdio(start: Option<&str>) {
    let rendering_context = Rc::new(SoftwareRenderingContext::new(PhysicalSize::new(1280, 720)).expect("invariant: the software GL starts"));
    rendering_context.make_current().expect("invariant: the software GL is current");
    let waker = Waker::default();
    let servo = ServoBuilder::default().opts(opts()).preferences(preferences()).event_loop_waker(Box::new(waker.clone())).build();
    let delegate = Rc::new(Delegate::default());
    let url = start.map(parse_url).unwrap_or_else(|| parse_url("about:blank"));
    let webview = WebViewBuilder::new(&servo, rendering_context.clone()).url(url).delegate(delegate.clone()).build();

    // Commands arrive on a thread of their own (a read on standard input sleeps in the host) and wake the loop.
    let commands: Arc<Mutex<VecDeque<Command>>> = Arc::default();
    {
        let (commands, waker) = (commands.clone(), waker.clone());
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            loop {
                let command = match protocol::read_command(&mut stdin) {
                    Ok(Some(c)) => c,
                    _ => Command::Quit,
                };
                let quit = matches!(command, Command::Quit);
                commands.lock().unwrap_or_else(|p| p.into_inner()).push_back(command);
                waker.wake();
                if quit {
                    return;
                }
            }
        });
    }
    let mut out = std::io::BufWriter::with_capacity(1 << 16, std::io::stdout());
    let (mut width, mut height) = (1280usize, 720usize);
    let mut sent: Vec<u8> = Vec::new();
    let _ = protocol::hello(&mut out, width as u16, height as u16);
    // Servo drops a navigation that arrives before its first browsing context exists (it exists once the initial page has loaded), so
    // navigation and input wait until then; a resize and a quit do not.
    let (mut ready, mut deferred): (bool, VecDeque<Command>) = (false, VecDeque::new());
    loop {
        servo.spin_event_loop();
        ready |= delegate.loaded.get();
        let mut pending: VecDeque<Command> = std::mem::take(&mut deferred);
        pending.extend(commands.lock().unwrap_or_else(|p| p.into_inner()).drain(..));
        for command in pending {
            if !ready && !matches!(command, Command::Resize(..) | Command::Quit) {
                deferred.push_back(command);
                continue;
            }
            match command {
                Command::Quit => return,
                Command::Navigate(text) => match Url::parse(&text) {
                    Ok(u) => webview.load(u),
                    Err(e) => {
                        let _ = protocol::error(&mut out, &format!("Not an address: {e}"));
                    },
                },
                Command::Nav(0) => {
                    webview.go_back(1);
                },
                Command::Nav(1) => {
                    webview.go_forward(1);
                },
                Command::Nav(2) => webview.reload(),
                Command::Nav(_) => {},
                Command::Resize(w, h) if w > 0 && h > 0 => {
                    let size = PhysicalSize::new(u32::from(w), u32::from(h));
                    rendering_context.resize(size);
                    webview.resize(size);
                    (width, height) = (usize::from(w), usize::from(h));
                    sent.clear(); // the next picture goes out whole
                },
                Command::Resize(..) => {},
                Command::Key { down, mods, key, text } => {
                    webview.notify_input_event(key_event(down, mods, &key, &text));
                },
                Command::Pointer { kind, x, y, button } => {
                    let point = WebViewPoint::Device(DevicePoint::new(f32::from(x), f32::from(y)));
                    let event = match kind {
                        0 => InputEvent::MouseMove(MouseMoveEvent::new(point)),
                        k => {
                            let b = match button {
                                1 => MouseButton::Auxiliary,
                                2 => MouseButton::Secondary,
                                _ => MouseButton::Primary,
                            };
                            let action = if k == 1 { MouseButtonAction::Down } else { MouseButtonAction::Up };
                            InputEvent::MouseButton(MouseButtonEvent::new(action, b, point))
                        },
                    };
                    webview.notify_input_event(event);
                },
                Command::Wheel { x, y, dx, dy } => {
                    let point = WebViewPoint::Device(DevicePoint::new(f32::from(x), f32::from(y)));
                    // Browsers say positive is down and right; Servo's delta is how far the view scrolls up and left.
                    let delta = WheelDelta { x: -f64::from(dx), y: -f64::from(dy), z: 0.0, mode: WheelMode::DeltaPixel };
                    webview.notify_input_event(InputEvent::Wheel(WheelEvent::new(delta, point)));
                },
            }
        }
        if delegate.changed.replace(false) {
            let _ = protocol::url(&mut out, &delegate.url.borrow());
            let _ = protocol::title(&mut out, &delegate.title.borrow());
            let _ = protocol::status(&mut out, !delegate.loaded.get(), webview.can_go_back(), webview.can_go_forward());
        }
        if delegate.painted.replace(false) {
            rendering_context.with_frame(|now, size| {
                let (w, h) = (size.width as usize, size.height as usize);
                if now.len() != w * h * 4 {
                    return;
                }
                let rect = if sent.len() == now.len() { changed_rect(&sent, now, w, h) } else { changed_rect(&vec![0u8; now.len()], now, w, h).or(Some((0, 0, w, h, vec![0u8; w * h * 4]))) };
                if let Some((x, y, rw, rh, pixels)) = rect {
                    let _ = protocol::rect(&mut out, x as u16, y as u16, rw as u16, rh as u16, &pixels);
                }
                sent.clear();
                sent.extend_from_slice(now);
                let _ = (width, height);
            });
        }
        let _ = out.flush();
        waker.wait(Duration::from_millis(16));
    }
}

// fontsan's C++ (the OpenType sanitizer) is built with exceptions, and the WASI SDK's C++ runtime has none: these are the two
// functions it calls to throw one. A font that makes it throw ends the content process instead (the shell sees it end).
#[expect(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __cxa_allocate_exception(_size: usize) -> *mut u8 {
    std::process::abort()
}

#[expect(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn __cxa_throw(_exception: *mut u8, _type_info: *mut u8, _destructor: *mut u8) -> ! {
    std::process::abort()
}
