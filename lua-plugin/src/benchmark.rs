use crate::{SceneCommand, ScopedNodeName, UiCommand};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BenchmarkMetric {
    pub iterations: u32,
    pub elapsed: Duration,
    pub checksum: f64,
}

impl BenchmarkMetric {
    pub fn average_ns(self) -> f64 {
        self.elapsed.as_secs_f64() * 1_000_000_000.0 / self.iterations as f64
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RustBenchmarkReport {
    pub event: BenchmarkMetric,
    pub transform: BenchmarkMetric,
    pub numeric: BenchmarkMetric,
    pub text: BenchmarkMetric,
    pub widgets: BenchmarkMetric,
}

#[inline]
fn metric(start: Instant, checksum: f64) -> BenchmarkMetric {
    BenchmarkMetric {
        iterations: 1000,
        elapsed: start.elapsed(),
        checksum,
    }
}

/// Direct Rust baseline. The Lua counterpart uses the same loop bounds and
/// arithmetic but pays the Lua userdata/bridge/message costs.
pub fn run_rust_baseline() -> RustBenchmarkReport {
    let mut event_checksum = 0.0;
    let start = Instant::now();
    let mut callback = |value: f64| event_checksum += value;
    for i in 0..1000 {
        callback(i as f64 * 0.5);
    }
    let event = metric(start, event_checksum);

    let start = Instant::now();
    let mut checksum = 0.0;
    let target = ScopedNodeName {
        scope: None,
        name: "BoxA".into(),
    };
    let mut scene_commands = Vec::with_capacity(4_000);
    for i in 0..1000 {
        let t = i as f64 * 0.001;
        let (s, c) = t.sin_cos();
        let x = (i as f64 * 0.01) + s;
        let y = (i as f64 * 0.02) + c;
        let scale = 1.0 + (i % 17) as f64 * 0.001;
        let matrix_trace = c * scale + c * scale + scale + 1.0;
        checksum += x + y + s * c + matrix_trace;
        scene_commands.push(SceneCommand::SetPosition(
            target.clone(),
            x as f32,
            y as f32,
            0.0,
        ));
        scene_commands.push(SceneCommand::SetRotationAngles(
            target.clone(),
            0.0,
            0.0,
            t as f32,
        ));
        scene_commands.push(SceneCommand::SetScale(
            target.clone(),
            scale as f32,
            scale as f32,
            1.0,
        ));
    }
    let transform = metric(start, checksum);

    let start = Instant::now();
    let mut value = 1.0;
    for i in 0..1000 {
        value = (value * 1.0001 + i as f64 * 0.00001).sin().abs();
    }
    let numeric = metric(start, value);

    let start = Instant::now();
    let mut text = String::new();
    for i in 0..1000 {
        text.push_str("lua-rust-");
        text.push_str(&(i % 100).to_string());
    }
    let text_metric = metric(start, text.len() as f64);

    let start = Instant::now();
    let mut enabled = false;
    let mut opacity = 0.0;
    let mut ui_commands = Vec::with_capacity(13_000);
    for i in 0..1000 {
        enabled = !enabled;
        opacity = (i as f64 / 1000.0).fract();
        ui_commands.push(UiCommand::SetText("hud_title".into(), format!("bench-{i}")));
        ui_commands.push(UiCommand::SetVisible("hud_title".into(), enabled));
        ui_commands.push(UiCommand::SetEnabled("send_button".into(), enabled));
        ui_commands.push(UiCommand::SetWidth(
            "hud_title".into(),
            200.0 + i as f32 % 10.0,
        ));
        ui_commands.push(UiCommand::SetHeight("hud_title".into(), 30.0));
        ui_commands.push(UiCommand::SetPosition(
            "hud_title".into(),
            i as f32 % 20.0,
            10.0,
        ));
        ui_commands.push(UiCommand::SetChecked("demo_toggle".into(), enabled));
        ui_commands.push(UiCommand::SetSelected(
            "demo_selector".into(),
            Some(i as usize % 2),
        ));
        ui_commands.push(UiCommand::SetScroll(
            "demo_scroll_viewer".into(),
            0.0,
            i as f32 % 10.0,
        ));
        ui_commands.push(UiCommand::SetProgress(
            "demo_progress".into(),
            i as f32 / 1000.0,
        ));
        ui_commands.push(UiCommand::SetOpacity("demo_image".into(), opacity as f32));
        ui_commands.push(UiCommand::SetGridRow(
            "demo_grid_child".into(),
            i as usize % 2,
        ));
        ui_commands.push(UiCommand::SetColor(
            "hud_title".into(),
            (i % 255) as f32 / 255.0,
            1.0 - (i % 255) as f32 / 255.0,
            0.5,
            1.0,
        ));
    }
    let widgets = metric(
        start,
        opacity + enabled as u8 as f64 + ui_commands.len() as f64,
    );

    RustBenchmarkReport {
        event,
        transform,
        numeric,
        text: text_metric,
        widgets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_runs_all_thousand_iterations() {
        let report = run_rust_baseline();
        for metric in [
            report.event,
            report.transform,
            report.numeric,
            report.text,
            report.widgets,
        ] {
            assert_eq!(metric.iterations, 1000);
            assert!(metric.elapsed.as_nanos() > 0);
            assert!(metric.checksum.is_finite());
        }
        assert_eq!(report.event.checksum, 249_750.0);
        assert_eq!(report.text.checksum, 10_900.0);
    }
}
