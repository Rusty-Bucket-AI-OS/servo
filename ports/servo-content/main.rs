/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! `servo-content URL OUT.png [WIDTH HEIGHT]`: loads a page with Servo, paints it with the software GL into memory, waits for
//! the load to finish and a frame to be ready, and writes the frame to `OUT.png`. The Rusty Bucket content process, first stage:
//! the pixel buffer and the protocol to the browser shell come next. Prints how long the load and the first frame took.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use dpi::PhysicalSize;
use embedder_traits::EventLoopWaker;
use servo::{
    DeviceIntRect, DeviceIntSize, LoadStatus, Preferences, RenderingContext, ServoBuilder, SoftwareRenderingContext, WebView, WebViewBuilder,
    WebViewDelegate,
};
use url::Url;

/// Servo wakes the embedder when it has work for `spin_event_loop`; the loop sleeps on this until then.
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
    title: RefCell<String>,
}

impl WebViewDelegate for Delegate {
    fn notify_new_frame_ready(&self, webview: WebView) {
        self.frames.set(self.frames.get() + 1);
        if self.loaded.get() {
            self.frame_after_load.set(true);
        }
        webview.paint();
    }

    fn notify_load_status_changed(&self, _webview: WebView, status: LoadStatus) {
        if status == LoadStatus::Complete {
            self.loaded.set(true);
        }
    }

    fn notify_page_title_changed(&self, _webview: WebView, title: Option<String>) {
        *self.title.borrow_mut() = title.unwrap_or_default();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(url), Some(out)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: servo-content URL OUT.png [WIDTH HEIGHT]");
        std::process::exit(2);
    };
    let url = Url::parse(url).unwrap_or_else(|e| {
        eprintln!("bad URL: {e}");
        std::process::exit(2)
    });
    let width = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1920);
    let height = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1080);

    let started = Instant::now();
    let rendering_context = Rc::new(
        SoftwareRenderingContext::new(PhysicalSize::new(width, height)).expect("invariant: the software GL starts"),
    );
    rendering_context.make_current().expect("invariant: the software GL is current");

    let waker = Waker::default();
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
    let servo = ServoBuilder::default().preferences(preferences).event_loop_waker(Box::new(waker.clone())).build();
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
