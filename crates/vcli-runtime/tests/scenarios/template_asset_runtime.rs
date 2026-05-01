//! Scenario: template predicates receive daemon-materialized asset bytes.

#[path = "../common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;

use common::*;
use image::{ImageBuffer, Rgba};
use vcli_core::frame::{Frame, FrameFormat};
use vcli_core::geom::Rect;
use vcli_core::predicate::{Confidence, PredicateKind};
use vcli_core::watch::{Lifetime, Watch, WatchWhen};
use vcli_core::{program::DslVersion, trigger::Trigger, Program, Region};
use vcli_perception::Perception;
use vcli_runtime::clock::ManualClock;

#[test]
fn template_watch_fires_when_asset_bytes_are_supplied() {
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx) = unbounded::<Event>();

    let template = template_png();
    let mut assets = BTreeMap::new();
    assets.insert("pattern".into(), template);
    let capture = Box::new(ScriptedCapture::new(vec![scene_with_template()]));
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(ManualClock::new(1_000));
    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();

    let mut preds = BTreeMap::new();
    preds.insert(
        "skip".into(),
        PredicateKind::Template {
            image: "sha256:pattern".into(),
            confidence: Confidence(0.95),
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 12,
                    h: 12,
                },
            },
            throttle_ms: 200,
        },
    );
    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "template_runtime".into(),
        id: None,
        trigger: Trigger::OnSubmit,
        predicates: preds,
        watches: vec![Watch {
            when: WatchWhen::ByName("skip".into()),
            steps: vec![],
            throttle_ms: 0,
            lifetime: Lifetime::OneShot,
        }],
        body: vec![],
        on_complete: None,
        on_fail: None,
        timeout_ms: None,
        labels: BTreeMap::new(),
        priority: Default::default(),
    };

    let mut sched = Scheduler::new(
        SchedulerConfig::default(),
        capture,
        input,
        Perception::new(),
        clock,
        cmd_rx,
        ev_tx,
    );
    cmd_tx
        .send(SchedulerCommand::SubmitValidated {
            program_id: id,
            program,
            assets,
        })
        .unwrap();

    sched.tick_once_pub();

    let events = drain_events(&ev_rx);
    let types: Vec<_> = events.iter().map(event_type).collect();
    assert!(types.iter().any(|t| t == "watch.fired"), "types: {types:?}");
    assert!(
        types.iter().any(|t| t == "program.completed"),
        "types: {types:?}"
    );
}

fn template_png() -> Vec<u8> {
    let mut img = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_pixel(4, 4, Rgba([255, 255, 255, 255]));
    for y in 1..3 {
        for x in 1..3 {
            img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
        }
    }
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

fn scene_with_template() -> Frame {
    let width = 12usize;
    let height = 12usize;
    let stride = width * 4;
    let mut pixels = vec![255u8; stride * height];
    for y in 6..8usize {
        for x in 5..7usize {
            let offset = y * stride + x * 4;
            pixels[offset] = 0;
            pixels[offset + 1] = 0;
            pixels[offset + 2] = 0;
            pixels[offset + 3] = 255;
        }
    }
    Frame::new(
        FrameFormat::Rgba8,
        Rect {
            x: 0,
            y: 0,
            w: 12,
            h: 12,
        },
        stride,
        Arc::from(pixels),
        0,
    )
}
