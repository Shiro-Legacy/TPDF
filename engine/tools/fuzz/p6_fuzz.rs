// Scratch fuzz harness (not committed): generated colour spaces, functions,
// shadings, tiling patterns and images rendered in catch_unwind.
// p6_fuzz KIND SEED COUNT [DUMP_CASE]
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;
use tandem_pdf_engine::view::{RenderOptions, ViewDocument};

fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes(),
    );
    bytes
}
fn obj(t: &str) -> Vec<u8> {
    t.as_bytes().to_vec()
}
fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}
fn flate(dict: &str, data: &[u8]) -> Vec<u8> {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
    stream(&format!("{dict} /Filter /FlateDecode"), &packed)
}
fn one_page(resources: &str, content: &[u8], extra: &[Vec<u8>]) -> Vec<u8> {
    let mut objects = vec![
        obj("<< /Type /Catalog /Pages 2 0 R >>"),
        obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        obj(&format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources {resources} /Contents 4 0 R >>")),
        stream("", content),
    ];
    objects.extend_from_slice(extra);
    pdf(&objects)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
    fn num(&mut self) -> String {
        let v = [0.0, 1.0, 0.5, -1.0, 2.0, 100.0, 1e6, -1e6, 1e-6, 0.25, 3.0, 40.0][self.below(12)];
        format!("{v}")
    }
    fn nums(&mut self, n: usize) -> String {
        (0..n).map(|_| format!("{} ", self.num())).collect()
    }
    fn numsr(&mut self, max: usize) -> String {
        let n = self.below(max);
        self.nums(n)
    }
    fn numsr2(&mut self, n: usize) -> String {
        let n = self.below(2) * n;
        self.nums(n)
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

fn space(rng: &mut Rng, depth: u32) -> String {
    let simple = ["/DeviceGray", "/DeviceRGB", "/DeviceCMYK", "/Pattern", "/G", "/RGB", "/Foo"];
    if depth > 2 || rng.below(3) == 0 {
        return rng.pick(&simple).to_string();
    }
    match rng.below(9) {
        0 => format!("[/Separation /A {} 5 0 R]", space(rng, depth + 1)),
        1 => format!("[/DeviceN [/A /B /C] {} 5 0 R]", space(rng, depth + 1)),
        2 => format!("[/DeviceN [/A] {} 5 0 R << /Subtype /NChannel >>]", space(rng, depth + 1)),
        3 => {
            let n = rng.below(5);
            let hival = rng.below(300);
            let len = rng.below(20);
            let hex: String = rng.bytes(len).iter().map(|b| format!("{b:02X}")).collect();
            let _ = n;
            format!("[/Indexed {} {hival} <{hex}>]", space(rng, depth + 1))
        }
        4 => format!("[/ICCBased 6 0 R]"),
        5 => format!("[/Lab << /WhitePoint [{}] /Range [{}] >>]", rng.nums(3), rng.numsr(6)),
        6 => format!("[/CalRGB << /WhitePoint [{}] /Gamma [{}] /Matrix [{}] >>]", rng.nums(3), rng.nums(3), rng.numsr(10)),
        7 => format!("[/CalGray << /WhitePoint [{}] /Gamma {} >>]", rng.nums(3), rng.num()),
        _ => format!("[/Pattern {}]", space(rng, depth + 1)),
    }
}

fn function(rng: &mut Rng) -> Vec<u8> {
    let inputs = rng.below(4);
    let outputs = rng.below(6);
    let domain = "0 1 ".repeat(inputs.max(1));
    match rng.below(5) {
        0 => {
            let progs = ["{}", "{pop}", "{dup dup}", "{0 exch 0 exch 0}", "{1 2 3 4 5 6 7 8 9}", "{dup 0.5 gt {0} {0 0 0 0} ifelse}", "{2 copy 3 index roll}", "{100 {dup} repeat}", "{1 0 div}", "{-1 index}"];
            stream(&format!("/FunctionType 4 /Domain [{domain}] /Range [{}]", "0 1 ".repeat(outputs)), rng.pick(&progs).as_bytes())
        }
        1 => obj(&format!("<< /FunctionType 2 /Domain [0 1] /C0 [{}] /C1 [{}] /N {} >>", rng.nums(outputs), rng.numsr(6), rng.num())),
        2 => {
            let size: Vec<usize> = (0..inputs.max(1)).map(|_| rng.below(4)).collect();
            let sizes: String = size.iter().map(|s| format!("{s} ")).collect();
            let len = rng.below(64);
            let bps = [1, 2, 4, 8, 12, 16, 24, 32, 0, 7][rng.below(10)];
            let data = rng.bytes(len);
            stream(&format!("/FunctionType 0 /Domain [{domain}] /Range [{}] /Size [{sizes}] /BitsPerSample {bps} /Encode [{}] /Decode [{}]", "0 1 ".repeat(outputs), rng.numsr(6), rng.numsr(8)), &data)
        }
        3 => obj(&format!(
            "<< /FunctionType 3 /Domain [{}] /Functions [<< /FunctionType 2 /Domain [0 1] /C1 [{}] /N 1 >> << /FunctionType 2 /Domain [0 1] /C1 [{}] /N 1 >>] /Bounds [{}] /Encode [{}] >>",
            rng.nums(2), rng.nums(outputs), rng.numsr(5), rng.numsr(3), rng.numsr(6)
        )),
        _ => obj(&format!("[<< /FunctionType 2 /Domain [0 1] /C1 [{}] /N 1 >> << /FunctionType 2 /Domain [0 1] /C1 [1] /N 1 >>]", rng.nums(outputs))),
    }
}

fn icc(rng: &mut Rng) -> Vec<u8> {
    let n = rng.below(6);
    let len = rng.below(200);
    let data = rng.bytes(len);
    stream(&format!("/N {n} /Alternate {}", rng.pick(&["/DeviceRGB", "/DeviceGray", "/DeviceCMYK", "/Lab"])), &data)
}

fn shading(rng: &mut Rng) -> Vec<u8> {
    let t = 1 + rng.below(7);
    let mesh_len = rng.below(300);
    let mesh = rng.bytes(mesh_len);
    let d = format!(
        "/ShadingType {t} /ColorSpace 7 0 R /Coords [{}] /Domain [{}] /Matrix [{}] /Function {} /Extend [{} {}] /BitsPerCoordinate {} /BitsPerComponent {} /BitsPerFlag {} /VerticesPerRow {} /Decode [{}] /Background [{}] /BBox [{}]",
        { let n = 4 + rng.below(3); rng.nums(n) },
        rng.numsr(5),
        rng.numsr2(6),
        if rng.below(4) == 0 { "[5 0 R 5 0 R]" } else { "5 0 R" },
        rng.pick(&["true", "false"]),
        rng.pick(&["true", "false"]),
        [1, 2, 4, 8, 12, 16, 24, 32, 0, 33][rng.below(10)],
        [1, 2, 4, 8, 12, 16, 0, 33][rng.below(8)],
        [2, 4, 8, 0, 1][rng.below(5)],
        [0, 1, 2, 3, 100, 70000][rng.below(6)],
        rng.numsr(12),
        rng.numsr(5),
        rng.numsr2(4),
    );
    if t >= 4 { stream(&d, &mesh) } else { obj(&format!("<< {d} >>")) }
}

fn tiling(rng: &mut Rng) -> Vec<u8> {
    let steps = ["0", "-5", "0.001", "1", "7", "100", "-300", "4000", "70000", "1e9", "1e30", "-1e30"];
    let boxes = ["[0 0 10 10]", "[0 0 0 0]", "[-50 -50 50 50]", "[0 0 5000 3]", "[10 10 2 2]", "[0 0 1e6 1e6]", "[0 0 1e30 1]", "[]", "[1 2 3]"];
    let matrices = ["[1 0 0 1 0 0]", "[0.01 0 0 0.01 0 0]", "[50 0 0 50 0 0]", "[1 2 3 4 5 6]", "[0 0 0 0 0 0]", "[1 0 0 -1 0 100]", "[1e5 0 0 1e-5 0 0]", "[1e30 0 0 1e30 0 0]"];
    stream(
        &format!(
            "/Type /Pattern /PatternType 1 /PaintType {} /TilingType {} /BBox {} /XStep {} /YStep {} /Matrix {} /Resources << /Pattern << /P0 8 0 R >> /Shading << /Sh0 8 0 R >> >>",
            1 + rng.below(2), 1 + rng.below(3), rng.pick(&boxes), rng.pick(&steps), rng.pick(&steps), rng.pick(&matrices)
        ),
        rng.pick(&["0 0 1 rg 0 0 4 4 re f 1 w 0 0 m 8 8 l S", "/Pattern cs /P0 scn 0 0 5 5 re f", "/Sh0 sh", "q 3 0 0 3 1 1 cm BI /W 2 /H 2 /BPC 8 /CS /G ID 0A9z EI Q"]).as_bytes(),
    )
}

fn image(rng: &mut Rng, cs: &str) -> Vec<u8> {
    let w = [0, 1, 2, 3, 7, 17, 64, 100000][rng.below(8)];
    let h = [0, 1, 2, 3, 9, 33, 64, 100000][rng.below(8)];
    let bpc = [1, 2, 4, 8, 16, 0, 3, 32][rng.below(8)];
    let len = rng.below(400);
    let data = rng.bytes(len);
    let mask = match rng.below(5) {
        0 => " /SMask 9 0 R".to_string(),
        1 => format!(" /Mask [{}]", rng.numsr(8)),
        2 => " /Mask 9 0 R".to_string(),
        3 => format!(" /ImageMask true /Decode [{}]", rng.numsr(3)),
        _ => String::new(),
    };
    let dict = format!(
        "/Type /XObject /Subtype /Image /Width {w} /Height {h} /BitsPerComponent {bpc} /ColorSpace {cs} /Decode [{}] /Interpolate {}{mask}",
        rng.numsr(9),
        rng.pick(&["true", "false"])
    );
    if rng.below(2) == 0 { flate(&dict, &data) } else { stream(&dict, &data) }
}

fn smask(rng: &mut Rng) -> Vec<u8> {
    let w = [1, 2, 5, 64, 70000][rng.below(5)];
    let h = [1, 2, 5, 64][rng.below(4)];
    let bpc = [1, 8, 16, 4][rng.below(4)];
    let len = rng.below(300);
    let data = rng.bytes(len);
    stream(&format!("/Type /XObject /Subtype /Image /Width {w} /Height {h} /BitsPerComponent {bpc} /ColorSpace /DeviceGray /Matte [{}] /Decode [{}]", rng.numsr(4), rng.numsr(3)), &data)
}

fn mesh(rng: &mut Rng) -> Vec<u8> {
    let t = 4 + rng.below(4);
    let (cs, n) = [("/DeviceGray", 1), ("/DeviceRGB", 3), ("/DeviceCMYK", 4)][rng.below(3)];
    let with_fn = rng.below(3) == 0;
    let comps = if with_fn { 1 } else { n };
    let mut decode = String::from(["0 100 0 100 ", "-50 150 -50 150 ", "0 1e6 0 1e6 ", "0 1 0 1 "][rng.below(4)]);
    for _ in 0..comps {
        decode.push_str("0 1 ");
    }
    let len = [10, 100, 1000, 20000, 200000][rng.below(5)];
    let data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
    let dict = format!(
        "/ShadingType {t} /ColorSpace {cs} {} /BitsPerCoordinate {} /BitsPerComponent {} /BitsPerFlag {} /VerticesPerRow {} /Decode [{decode}]",
        if with_fn { "/Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >>" } else { "" },
        [1, 2, 4, 8, 16, 32][rng.below(6)],
        [1, 2, 4, 8, 16][rng.below(5)],
        [2, 4, 8][rng.below(3)],
        [0, 1, 2, 3, 50, 70000][rng.below(6)],
    );
    flate(&dict, &data)
}

fn case(rng: &mut Rng, kind: &str) -> Vec<u8> {
    if kind == "mesh" {
        let sh = mesh(rng);
        let content = format!("q {} cm /Sh0 sh Q /Pattern cs /SP scn 0 0 100 100 re f", rng.pick(&["1 0 0 1 0 0", "0.5 0 0 0.5 25 25", "1000 0 0 1000 0 0", "0.001 0 0 0.001 0 0"]));
        let objects = vec![
            sh,
            obj("<< /Type /Pattern /PatternType 2 /Shading 5 0 R /Matrix [1 0 0 1 0 0] >>"),
        ];
        return one_page("<< /Shading << /Sh0 5 0 R >> /Pattern << /SP 6 0 R >> >>", content.as_bytes(), &objects);
    }
    let cs = space(rng, 0);
    let content = match kind {
        "color" => format!("/CS0 cs /CS0 CS {} scn {} SCN 5 5 30 30 re B BT /F1 12 Tf 5 40 Td (Ab) Tj ET q 20 0 0 20 40 5 cm /Im0 Do Q q 40 40 40 40 re W n /Sh0 sh Q BI /W 2 /H 2 /BPC 8 /CS /CS0 ID {} EI", rng.numsr(5), rng.numsr(5), "\u{1}\u{2}\u{3}\u{4}\u{5}\u{6}\u{7}\u{8}\u{9}\u{a}\u{b}\u{c}"),
        "shading" => format!("q 0 0 100 100 re W n /Sh0 sh Q /Pattern cs /SP scn 10 10 80 80 re f q {} cm /Sh0 sh Q", rng.nums(6)),
        "tiling" => format!("/Pattern cs /P0 scn 0 0 60 60 re f /Pattern CS /P0 SCN 4 w 70 10 m 95 90 l S BT /F1 30 Tf 2 Tr 10 70 Td (W) Tj ET q {} cm /Pattern cs /P0 scn 0 0 10 10 re f Q", rng.nums(6)),
        _ => format!("q {} cm /Im0 Do Q q 30 0 0 30 10 10 cm /Im0 Do Q", rng.nums(6)),
    };
    let resources = "<< /ColorSpace << /CS0 7 0 R >> /XObject << /Im0 10 0 R >> /Shading << /Sh0 8 0 R >> /Pattern << /P0 11 0 R /SP 12 0 R >> /Font << /F1 13 0 R >> >>";
    let sh = shading(rng);
    let img_cs = if rng.below(2) == 0 { "7 0 R".to_string() } else { space(rng, 0) };
    let objects = vec![
        function(rng),                       // 5
        icc(rng),                             // 6
        obj(&cs),                             // 7
        sh,                                   // 8
        smask(rng),                           // 9
        image(rng, &img_cs),                  // 10
        tiling(rng),                          // 11
        obj("<< /Type /Pattern /PatternType 2 /Shading 8 0 R /Matrix [1 0 0 1 0 0] >>"), // 12
        obj("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"), // 13
    ];
    one_page(resources, content.as_bytes(), &objects)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args[1] == "repeat" {
        let bytes = std::fs::read(&args[2]).unwrap();
        let n: usize = args[3].parse().unwrap();
        let scale: f64 = args[4].parse().unwrap();
        for i in 0..n {
            let view = ViewDocument::open(bytes.clone(), "").unwrap();
            let _ = view.render(0, scale, RenderOptions::default());
            if i % 10 == 0 {
                eprintln!("{i}");
            }
        }
        return;
    }
    let kind = args[1].clone();
    let seed: u64 = args[2].parse().unwrap();
    let count: usize = args[3].parse().unwrap();
    let dump: Option<usize> = args.get(4).and_then(|s| s.parse().ok());
    std::panic::set_hook(Box::new(|info| {
        let loc = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
        eprintln!("PANIC at {loc}: {}", &msg[..msg.len().min(160)]);
    }));
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let (mut panics, mut slow) = (0, 0);
    for i in 0..count {
        let bytes = case(&mut rng, &kind);
        if dump == Some(i) {
            std::fs::write(format!("{}/fuzz-{kind}-{seed}-{i}.pdf", std::env::var("FUZZ_DUMP").unwrap_or_else(|_| ".".into())), &bytes).unwrap();
        }
        if i < std::env::var("SKIP").ok().and_then(|v| v.parse().ok()).unwrap_or(0) {
            continue;
        }
        eprint!("\rcase {i}   ");
        std::io::stderr().flush().ok();
        let start = Instant::now();
        let result = catch_unwind(AssertUnwindSafe(|| {
            if let Ok(view) = ViewDocument::open(bytes, "") {
                for scale in [0.5, 2.0] {
                    let _ = view.render(0, scale, RenderOptions::default());
                }
                let _ = view.structured_text(0);
            }
        }));
        let ms = start.elapsed().as_millis();
        if std::env::var("RSS").is_ok() {
            let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().unwrap();
            let kb: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0);
            eprintln!(" rss case {i}: {} MB", kb / 1024);
        }
        if result.is_err() {
            panics += 1;
            eprintln!(" <- case {i} panicked");
        }
        if ms > 2000 {
            slow += 1;
            eprintln!(" <- case {i} slow {ms} ms");
        }
    }
    eprintln!("\n{kind} seed {seed}: {count} cases, {panics} panics, {slow} slow");
}
