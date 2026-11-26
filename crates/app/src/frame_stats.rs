//! How long each window takes to draw its frames, printed every second in
//! which it drew some: with the `frame-stats` feature, for `tools/perf`.

use std::collections::HashMap;
use std::time::Duration;

use gpui::{App, FrameDurationSnapshot, WindowId};

/// A frame that takes longer misses a refresh at 60 Hz.
const BUDGET_NANOS: u64 = 16_000_000;

pub fn init(cx: &mut App) {
    cx.spawn(async move |cx| {
        let mut before: HashMap<WindowId, FrameDurationSnapshot> = HashMap::new();
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let windows = cx.update(|cx| cx.windows());
            // A window closed since, such as the panel's, is forgotten.
            before.retain(|id, _| windows.iter().any(|handle| handle.window_id() == *id));
            for handle in windows {
                let Ok(now) = handle.update(cx, |_, window, _| window.frame_duration_snapshot())
                else {
                    continue;
                };
                let mut frames = now.draw_duration_histogram.clone();
                if let Some(earlier) = before.get(&handle.window_id()) {
                    frames.subtract(&earlier.draw_duration_histogram).ok();
                }
                if !frames.is_empty() {
                    eprintln!(
                        "frames: {} drawn, slowest {:.1} ms, {} over 16 ms",
                        frames.len(),
                        frames.max() as f64 / 1e6,
                        frames.count_between(BUDGET_NANOS, u64::MAX),
                    );
                }
                before.insert(handle.window_id(), now);
            }
        }
    })
    .detach();
}
