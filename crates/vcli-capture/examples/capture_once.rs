//! capture_once — grab one screen frame, print dimensions + permission status.
//!
//! Useful for manually verifying macOS Screen Recording permission and the
//! SCK wiring without running the whole daemon.
//!
//! Usage:  cargo run -p vcli-capture --example capture_once
//!         cargo run -p vcli-capture --example capture_once -- --save /tmp/out.png

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(target_os = "macos")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use vcli_capture::{
        capture::Capture,
        macos::MacCapture,
        permission::{
            check_screen_recording_permission, request_screen_recording_permission,
            PermissionStatus,
        },
    };

    let args: Vec<String> = env::args().collect();
    let save_to = args
        .iter()
        .position(|a| a == "--save")
        .and_then(|i| args.get(i + 1).cloned());

    match check_screen_recording_permission()? {
        PermissionStatus::Granted => {
            println!("permission: granted");
        }
        PermissionStatus::Denied => {
            println!("permission: denied — prompting user");
            request_screen_recording_permission()?;
            println!(
                "re-run after granting in System Settings → Privacy & Security → Screen Recording"
            );
            return Ok(());
        }
        PermissionStatus::Unknown => {
            println!("permission: unknown — attempting capture anyway");
        }
    }

    let mut cap = MacCapture::new()?;
    let frame = cap.grab_screen()?;
    println!(
        "captured {}x{} @ stride {} bytes, format {:?}",
        frame.width(),
        frame.height(),
        frame.stride,
        frame.format
    );

    if let Some(path) = save_to {
        use image::{ImageBuffer, Rgba};
        let w = frame.width() as u32;
        let h = frame.height() as u32;
        let mut img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(w, h);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let off = (y as usize) * frame.stride + (x as usize) * 4;
            // RGBA → store as RGBA (frame.format == Rgba8)
            let r = frame.pixels[off];
            let g = frame.pixels[off + 1];
            let b = frame.pixels[off + 2];
            let a = frame.pixels[off + 3];
            *px = Rgba([r, g, b, a]);
        }
        img.save(&path)?;
        println!("saved: {path}");
    }

    let windows = cap.enumerate_windows()?;
    println!("windows visible: {}", windows.len());
    for w in windows.iter().take(10) {
        println!(
            "  id={} app={:?} title={:?} bounds={}x{}@({},{})",
            w.id, w.app, w.title, w.bounds.w, w.bounds.h, w.bounds.x, w.bounds.y
        );
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("capture_once is only implemented for macOS in v0");
    Ok(())
}
