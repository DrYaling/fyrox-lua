use crate::{SceneCommand, ScopedNodeName, UiCommand};
use fyrox::core::instant::Instant;
use std::{rc::Rc, time::Duration};

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
        checksum: checksum as f64,
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
        name: Rc::from("BoxA"),
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
        ui_commands.push(UiCommand::SetText(
            "benchmark_label".into(),
            format!("bench-{i}"),
        ));
        ui_commands.push(UiCommand::SetVisible("benchmark_label".into(), enabled));
        ui_commands.push(UiCommand::SetEnabled("benchmark_button".into(), enabled));
        ui_commands.push(UiCommand::SetWidth(
            "benchmark_label".into(),
            200.0 + i as f32 % 10.0,
        ));
        ui_commands.push(UiCommand::SetHeight("benchmark_label".into(), 30.0));
        ui_commands.push(UiCommand::SetPosition(
            "benchmark_label".into(),
            i as f32 % 20.0,
            10.0,
        ));
        ui_commands.push(UiCommand::SetChecked("benchmark_toggle".into(), enabled));
        ui_commands.push(UiCommand::SetSelected(
            "benchmark_selector".into(),
            Some(i as usize % 2),
        ));
        ui_commands.push(UiCommand::SetScroll(
            "benchmark_scroll".into(),
            0.0,
            i as f32 % 10.0,
        ));
        ui_commands.push(UiCommand::SetProgress(
            "benchmark_progress".into(),
            i as f32 / 1000.0,
        ));
        ui_commands.push(UiCommand::SetOpacity(
            "benchmark_image".into(),
            opacity as f32,
        ));
        ui_commands.push(UiCommand::SetGridRow(
            "benchmark_grid_child".into(),
            i as usize % 2,
        ));
        ui_commands.push(UiCommand::SetColor(
            "benchmark_label".into(),
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

/// Measures the queued-command construction cost for a composite UI layout
/// workload supplied by a host benchmark script.
pub fn run_rust_ui_layout(iterations: u32) -> BenchmarkMetric {
    let mut commands = Vec::with_capacity(iterations as usize);
    let name: Rc<str> = Rc::from("status");
    let mut checksum = 0.0_f64;
    let mut samples = Vec::with_capacity(7);
    for _ in 0..7 {
        commands.clear();
        let start = Instant::now();
        for i in 0..iterations {
            let x = (i % 100) as f32;
            let y = (i % 10) as f32;
            commands.push(UiCommand::SetLayout(
                name.clone(),
                x,
                y,
                100.0 + y,
                20.0 + y,
            ));
            checksum += (x + y) as f64;
        }
        std::hint::black_box(&commands);
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    BenchmarkMetric {
        iterations,
        elapsed: samples[3],
        checksum: checksum / 7.0,
    }
}

/// Reference for the three-command path where each command allocates its target String.
pub fn run_rust_ui_layout_legacy(iterations: u32) -> BenchmarkMetric {
    #[allow(dead_code)]
    enum LegacyUiCommand {
        Position(String, f32, f32),
        Width(String, f32),
        Height(String, f32),
    }
    let mut commands = Vec::with_capacity(iterations as usize * 3);
    let mut checksum = 0.0_f64;
    let mut samples = Vec::with_capacity(7);
    for _ in 0..7 {
        commands.clear();
        let start = Instant::now();
        for i in 0..iterations {
            let x = (i % 100) as f32;
            let y = (i % 10) as f32;
            commands.push(LegacyUiCommand::Position(String::from("status"), x, y));
            commands.push(LegacyUiCommand::Width(String::from("status"), 100.0 + y));
            commands.push(LegacyUiCommand::Height(String::from("status"), 20.0 + y));
            checksum += (x + y) as f64;
        }
        std::hint::black_box(&commands);
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    BenchmarkMetric {
        iterations,
        elapsed: samples[3],
        checksum: checksum / 7.0,
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

    #[test]
    fn shared_name_layout_benchmark_preserves_workload_checksum() {
        let legacy = run_rust_ui_layout_legacy(40);
        let optimized = run_rust_ui_layout(40);
        assert_eq!(legacy.iterations, optimized.iterations);
        assert_eq!(legacy.checksum, optimized.checksum);
        assert!(legacy.elapsed.as_nanos() > 0);
        assert!(optimized.elapsed.as_nanos() > 0);
    }
}
