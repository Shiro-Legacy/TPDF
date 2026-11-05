//! Bench driver for dynamic XFA forms (design/app-xfa.md):
//!   xfa_probe materialize IN.pdf OUT.pdf        writes the materialized document, prints timing
//!   xfa_probe render FILE PAGE DPI OUT.png      renders through the app's open (materializes)
//!   xfa_probe pages FILE                        page count and sizes through the app's open
//!   xfa_probe fields FILE                       the `fields` op's items for every page (JSON lines)
//!   xfa_probe fill FILE OUT.pdf ID=VALUE...     fills widgets by id (page 0 unless ID is P:ID), saves
//!   xfa_probe datasets FILE                     prints the datasets packet's XML
//!   xfa_probe set-datasets FILE XML OUT.pdf     replaces the datasets packet with the file XML and
//!                                               saves, widgets untouched (a fill by another XFA
//!                                               processor); prints what a reopen decides
//!   xfa_probe state FILE [PATH=VALUE]           the form's nodes after its scripts ran (initialize,
//!                                               then the handlers of the node at data PATH filled
//!                                               with VALUE): SOM name, presence, value; and the report
//!   xfa_probe click FILE SOM|undo...            presses the push buttons whose field names end with
//!                                               SOM (or undoes the last step), in order, through
//!                                               `field-set`; prints timing, widget counts, the
//!                                               widgets per `Row[n]` and the datasets' rows
use std::sync::Arc;
use std::time::Instant;

use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppDocument, AppEngine, AppSession};
use tandem_pdf_engine::document::Document;
use tandem_pdf_engine::view::{RenderOptions, RgbaImage};
use tandem_pdf_engine::xfa;

fn session(bytes: Vec<u8>) -> Result<(AppEngine, AppSession), String> {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "open", "name": "bench.pdf", "pdf": {"$buffer": 0}});
    let t = Instant::now();
    let (reply, _) = engine.dispatch(&mut session, request, vec![bytes]);
    let took = t.elapsed();
    if reply["ok"] != true {
        return Err(format!("open failed: {reply}"));
    }
    println!("open in {took:?}: {reply}");
    Ok((engine, session))
}

fn write_png(path: &str, image: &RgbaImage) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(file, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&image.pixels)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("materialize") if args.len() == 4 => {
            let bytes = std::fs::read(&args[2])?;
            let t = Instant::now();
            let doc = Document::open_owned(bytes, Default::default())?;
            println!(
                "opened in {:?}; dynamic: {}",
                t.elapsed(),
                xfa::is_dynamic(&doc)
            );
            let t = Instant::now();
            match xfa::materialize(&doc) {
                Ok(out) => {
                    println!("materialized {} bytes in {:?}", out.len(), t.elapsed());
                    std::fs::write(&args[3], out)?;
                }
                Err(e) => println!("materialize failed after {:?}: {e}", t.elapsed()),
            }
        }
        Some("render") if args.len() == 6 => {
            let (page, scale) = (args[3].parse()?, args[4].parse::<f64>()? / 72.0);
            let bytes = Arc::new(std::fs::read(&args[2])?);
            let t = Instant::now();
            let mut doc = AppDocument::load(bytes, b"")?;
            println!("loaded in {:?}", t.elapsed());
            let t = Instant::now();
            let image = doc.view()?.render(page, scale, RenderOptions::default())?;
            println!(
                "rendered {}x{} in {:?}",
                image.width,
                image.height,
                t.elapsed()
            );
            write_png(&args[5], &image)?;
        }
        Some("pages") if args.len() == 3 => {
            let bytes = Arc::new(std::fs::read(&args[2])?);
            let mut doc = AppDocument::load(bytes, b"")?;
            let view = doc.view()?;
            println!("pages: {}", view.page_count());
            for p in 0..view.page_count() {
                println!("  {p}: {:?}", view.page_size(p)?);
            }
        }
        Some("fields") if args.len() == 3 => {
            let (mut engine, mut session) = session(std::fs::read(&args[2])?)?;
            let bytes = Arc::new(std::fs::read(&args[2])?);
            let pages = AppDocument::load(bytes, b"")?.view()?.page_count();
            for page in 0..pages {
                let (reply, _) = engine.dispatch(
                    &mut session,
                    json!({"id": 2, "op": "fields", "page": page}),
                    vec![],
                );
                if reply["ok"] != true {
                    println!("page {page}: {reply}");
                    continue;
                }
                for item in reply["items"].as_array().into_iter().flatten() {
                    println!("{page}\t{item}");
                }
            }
        }
        Some("fill") if args.len() >= 5 => {
            let (mut engine, mut session) = session(std::fs::read(&args[2])?)?;
            for spec in &args[4..] {
                let (id, value) = spec.split_once('=').ok_or("ID=VALUE")?;
                let (page, id) = match id.split_once(':') {
                    Some((p, i)) => (p.parse::<i64>()?, i),
                    None => (0, id),
                };
                let value: Value = match value {
                    "true" => json!(true),
                    "false" => json!(false),
                    v => json!(v),
                };
                let request = json!({"id": 3, "op": "field-set", "page": page, "widgetId": id.parse::<i64>()?, "value": value});
                let t = Instant::now();
                let (reply, _) = engine.dispatch(&mut session, request, vec![]);
                println!("field-set {spec} in {:?}: {reply}", t.elapsed());
                println!("messages: {}", reply["messages"]);
            }
            let (reply, outputs) =
                engine.dispatch(&mut session, json!({"id": 4, "op": "save"}), vec![]);
            println!("save: {reply}");
            if let Some(bytes) = outputs.into_iter().next() {
                std::fs::write(&args[3], bytes)?;
                println!("wrote {}", args[3]);
            }
        }
        Some("set-datasets") if args.len() == 5 => {
            let bytes = std::fs::read(&args[2])?;
            let xml = std::fs::read(&args[3])?;
            let mut doc = Document::open_owned(bytes, Default::default())?;
            let id = xfa::packets::read(&doc)?
                .entries
                .iter()
                .find(|(name, _)| name == b"datasets")
                .map(|(_, id)| *id)
                .ok_or("no datasets packet")?;
            doc.put_stream_decoded(id, Default::default(), xml)?;
            let out = tandem_pdf_engine::writer::write_document(
                &doc,
                Default::default(),
                tandem_pdf_engine::writer::WriteOptions::APP,
            )?;
            let reopened = Document::open(&out, Default::default())?;
            println!("reopen decides: {:?}", xfa::needs_rebuild(&reopened));
            std::fs::write(&args[4], &out)?;
            println!("wrote {}", args[4]);
        }
        Some("datasets") if args.len() == 3 => {
            let bytes = std::fs::read(&args[2])?;
            let doc = Document::open_owned(bytes, Default::default())?;
            let packets = xfa::packets::read(&doc)?;
            println!(
                "entries: {:?}",
                packets
                    .entries
                    .iter()
                    .map(|(n, _)| String::from_utf8_lossy(n).to_string())
                    .collect::<Vec<_>>()
            );
            println!("{}", packets.datasets.unwrap_or_default());
        }
        Some("state") if args.len() == 3 || args.len() == 4 => {
            use tandem_pdf_engine::xfa::types::{DataPath, Node, Presence};
            let bytes = std::fs::read(&args[2])?;
            let doc = Document::open_owned(bytes, Default::default())?;
            let packets = xfa::packets::read(&doc)?;
            let mut form = xfa::template::read(&packets.template)?;
            let mut datasets = match &packets.datasets {
                Some(text) => Some(xfa::xml::parse(text).map_err(|e| e.to_string())?),
                None => None,
            };
            let fill = match args.get(3) {
                Some(spec) => {
                    let (path, value) = spec.split_once('=').ok_or("PATH=VALUE")?;
                    let path = DataPath::parse(path).ok_or("a data path like form1[0]/F[0]")?;
                    // The fill is in the data before the form binds, as in the app.
                    if let Some(datasets) = &mut datasets {
                        xfa::sync::update_element(datasets, &[(path.clone(), value.to_string())])?;
                    }
                    Some((path, value.to_string()))
                }
                None => None,
            };
            xfa::bind::bind(&mut form, datasets.as_ref())?;
            let t = Instant::now();
            let mut runner = xfa::script::Runner::new(&mut form);
            let report = runner.initialize();
            println!("initialize: {report:?} in {:?}", t.elapsed());
            if let Some((path, value)) = &fill {
                let t = Instant::now();
                let report = runner.fire(path, value);
                println!("fire {path}={value}: {report:?} in {:?}", t.elapsed());
            }
            drop(runner);
            println!("signature: {:016x}", xfa::signature(&form));
            fn walk(nodes: &[Node], depth: usize) {
                for node in nodes {
                    let c = node.common();
                    let presence = match c.presence {
                        Presence::Visible => "",
                        Presence::Hidden => " hidden",
                        Presence::Invisible => " invisible",
                        Presence::Inactive => " inactive",
                    };
                    match node {
                        Node::Subform(s) => {
                            println!("{:width$}{}{presence}", "", s.som, width = depth * 2);
                            walk(&s.children, depth + 1);
                        }
                        Node::Field(f) => println!(
                            "{:width$}{}{presence} = {:?}",
                            "",
                            f.som,
                            f.value.plain(),
                            width = depth * 2
                        ),
                        Node::ExclGroup(g) => println!(
                            "{:width$}{}{presence} = {:?}",
                            "",
                            g.som,
                            g.value,
                            width = depth * 2
                        ),
                        Node::Draw(d) => {
                            if !presence.is_empty() {
                                println!(
                                    "{:width$}#draw {}{presence}",
                                    "",
                                    c.name,
                                    width = depth * 2
                                );
                            }
                            let _ = d;
                        }
                    }
                }
            }
            walk(&form.root.children, 0);
        }
        Some("click") if args.len() >= 4 => {
            let (mut engine, mut session) = session(std::fs::read(&args[2])?)?;
            let listing = |engine: &mut AppEngine, session: &mut AppSession| {
                let pages = session.doc.as_ref().map_or(0, |d| d.page_count());
                let mut items = Vec::new();
                for page in 0..pages {
                    let request = json!({"id": 2, "op": "fields", "page": page});
                    let (reply, _) = engine.dispatch(session, request, vec![]);
                    for item in reply["items"].as_array().into_iter().flatten() {
                        items.push((page, item.clone()));
                    }
                }
                items
            };
            let report = |engine: &mut AppEngine, session: &mut AppSession| {
                let items = listing(engine, session);
                let mut rows: std::collections::BTreeMap<String, usize> = Default::default();
                for (_, item) in &items {
                    let name = item["name"].as_str().unwrap_or("");
                    if let Some(at) = name.find(".Row[") {
                        let end = name[at..].find(']').map_or(name.len(), |e| at + e + 1);
                        *rows.entry(name[at + 1..end].to_string()).or_default() += 1;
                    }
                }
                let buttons = items.iter().filter(|(_, i)| i["kind"] == "button").count();
                let data = session
                    .doc
                    .as_ref()
                    .and_then(|d| xfa::packets::read(d.document()).ok())
                    .and_then(|p| p.datasets)
                    .unwrap_or_default();
                let data_rows = (data.match_indices("<Row").map(|(at, _)| at + 4))
                    .filter(|&at| data[at..].starts_with(['>', '/', ' ', '\n', '\r', '\t']))
                    .count();
                println!(
                    "  pages {}, widgets {}, buttons {buttons}, widgets per row {rows:?}, datasets rows {data_rows}",
                    session.doc.as_ref().map_or(0, |d| d.page_count()),
                    items.len()
                );
                items
            };
            let mut items = report(&mut engine, &mut session);
            for spec in &args[3..] {
                let t = Instant::now();
                let reply = if spec == "undo" {
                    engine
                        .dispatch(&mut session, json!({"id": 3, "op": "undo"}), vec![])
                        .0
                } else {
                    let Some((page, item)) = items.iter().find(|(_, i)| {
                        i["kind"] == "button"
                            && i["name"]
                                .as_str()
                                .is_some_and(|n| n.ends_with(spec.as_str()))
                    }) else {
                        println!("no button named ...{spec}");
                        continue;
                    };
                    let request = json!({"id": 3, "op": "field-set", "page": page, "widgetId": item["id"], "value": ""});
                    engine.dispatch(&mut session, request, vec![]).0
                };
                println!(
                    "{spec} in {:?}: ok {} messages {}",
                    t.elapsed(),
                    reply["ok"],
                    reply["messages"]
                );
                items = report(&mut engine, &mut session);
            }
        }
        _ => {
            return Err(
                "usage: xfa_probe materialize|render|pages|fields|fill|datasets|state|click ..."
                    .into(),
            );
        }
    }
    Ok(())
}
