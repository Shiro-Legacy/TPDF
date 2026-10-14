// SPDX-License-Identifier: AGPL-3.0-or-later
#![forbid(unsafe_code)]
use serde_json::json;
use std::{fs, io::Write};
use tandem_pdf_engine::session::{Session, SessionLimits};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 3 {
        return Err(
            "usage: tandem-rust-workflow inspect FILE | view-info FILE | render FILE PAGE DPI OUTPUT.ppm | edit FILE PAGE TARGET TEXT OUTPUT".into(),
        );
    }
    if fs::metadata(&args[2])?.len() > 16 << 20 {
        return Err("input exceeds 16 MiB".into());
    }
    let bytes = fs::read(&args[2])?;
    let mut session = if matches!(args[1].as_str(), "view-info" | "render") {
        // Development CLI only: the password comes from the environment.
        let password = std::env::var("TPDF_PDF_PASSWORD").unwrap_or_default();
        Session::open_view_with_password(bytes, SessionLimits::default(), password.as_bytes())?
    } else {
        Session::open(bytes, SessionLimits::default())?
    };
    match args[1].as_str() {
        "view-info" => {
            let caps = session.capabilities();
            println!(
                "{}",
                json!({"pages":caps.pages.iter().enumerate().map(|(index,p)|json!({
                "page":index,"canRender":p.render_error.is_none(),"renderError":p.render_error.as_ref().map(ToString::to_string),
                "canEditText":p.edit_error.is_none() && p.text_target_count>0,"editError":p.edit_error.as_ref().map(ToString::to_string),
                "textTargetCount":p.text_target_count})).collect::<Vec<_>>(),
                "canSave":caps.save_error.is_none(),"saveError":caps.save_error.as_ref().map(ToString::to_string)})
            );
        }
        "render" if args.len() == 6 => {
            let image = session.render(args[3].parse()?, args[4].parse()?)?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&args[5])?;
            write!(file, "P6\n{} {}\n255\n", image.width(), image.height())?;
            let mut rgb = Vec::with_capacity(image.pixels().len() / 4 * 3);
            for pixel in image.pixels().chunks_exact(4) {
                rgb.extend_from_slice(&pixel[..3]);
            }
            file.write_all(&rgb)?;
        }
        "inspect" => {
            let pages = session.pages()?;
            for page in 0..pages.len() {
                match session.text_targets(page) {
                    Ok(targets) => println!(
                        "{}",
                        json!({"page":page,"targets":targets.iter().enumerate().map(|(id,t)|json!({"target":id,"text":t.text,"bounds":t.bounds,"repertoire":t.repertoire})).collect::<Vec<_>>()})
                    ),
                    Err(error) => println!(
                        "{}",
                        json!({"page":page,"reason":error.reason,"detail":error.detail})
                    ),
                }
            }
        }
        "edit" if args.len() == 7 => {
            let page = args[3].parse()?;
            let target: usize = args[4].parse()?;
            let targets = session.text_targets(page)?;
            session.replace_text(targets.get(target).ok_or("target out of range")?, &args[5])?;
            let saved = session.save()?;
            // A new process/session must be able to consume exactly this output.
            let mut check = Session::open(saved.bytes.clone(), SessionLimits::default())?;
            check.render(page, 72.)?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&args[6])?;
            file.write_all(&saved.bytes)?;
            file.sync_all()?;
            session.acknowledge_saved(saved.token)?;
            println!(
                "{}",
                json!({"saved":args[6],"bytes":saved.bytes.len(),"dirty":session.state().dirty,"reopened":true})
            );
        }
        _ => return Err("invalid command or arguments".into()),
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
