//! Release timing of typical UI lines (ignored by default):
//! `cargo test --release -p door-graphics-text --test perf -- --ignored --nocapture`
mod support;
use door_graphics_text::*;
use std::time::Instant;

fn micros(iterations: u32, mut f: impl FnMut(u32)) -> f64 {
    let start = Instant::now();
    for i in 0..iterations {
        f(i);
    }
    start.elapsed().as_secs_f64() * 1e6 / f64::from(iterations)
}

#[test]
#[ignore]
fn ui_line_timings() {
    let system = TextSystem::new(support::fonts());
    let lines = [
        (
            "latin",
            "en-US",
            "Settings - Network & internet - Advanced options",
        ),
        ("cjk", "zh-Hans", "文件资源管理器 - 下载 - 简体中文测试"),
        ("arabic", "ar", "مرحبا بالعالم - إعدادات الشبكة"),
        ("mixed", "en-US", "Report 2026 שלום 你好 \u{1F600} done"),
    ];
    let mut pixels = vec![255u8; 800 * 40 * 4];
    for (name, locale, text) in lines {
        for scale in [1.0f32, 1.5] {
            let format = TextFormat::new(14.0)
                .with_scale(scale)
                .with_locale(Locale::new(locale))
                .with_line_height(20.0);
            system.clear_caches();
            let cold = micros(1, |_| {
                let layout = system.layout_line(text, &format).unwrap();
                let mut target = Target::new(&mut pixels, 800, 40, 3200).unwrap();
                let clip = target.bounds();
                system
                    .draw(&layout, &mut target, clip, 4, 4, [0, 0, 0, 255])
                    .unwrap();
            });
            let layout_warm = micros(2000, |_| {
                std::hint::black_box(system.layout_line(text, &format).unwrap());
            });
            let layout = system.layout_line(text, &format).unwrap();
            let draw_warm = micros(2000, |_| {
                let mut target = Target::new(&mut pixels, 800, 40, 3200).unwrap();
                let clip = target.bounds();
                system
                    .draw(&layout, &mut target, clip, 4, 4, [0, 0, 0, 255])
                    .unwrap();
            });
            // New text each time: shaping misses, glyphs already cached.
            let shape_cold = micros(500, |i| {
                let unique = format!("{text} {i}");
                std::hint::black_box(system.layout_line(&unique, &format).unwrap());
            });
            let glyphs = layout.glyphs().len();
            println!(
                "{name:7} {scale}x glyphs={glyphs:3} cold(first layout+raster+draw)={cold:9.1}us warm layout={layout_warm:6.1}us warm draw={draw_warm:6.1}us layout with shaping miss={shape_cold:6.1}us"
            );
        }
    }
    println!("{:?}", system.cache_stats());
}
