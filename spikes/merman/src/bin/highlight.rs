//! Which way of marking the selected node survives merman's PNG path?
//! Renders `out/map.mmd` (written by the main binary) with each variant appended and
//! counts highlight-yellow pixels in the PNG.

use merman::svg::export::RasterOptions;
use merman::{OperationControl, PngRequest, RenderOutput, RenderRequest, Renderer, SvgRequest};

fn yellow_pixels(png_bytes: &[u8]) -> usize {
    let mut dec = png::Decoder::new(std::io::Cursor::new(png_bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().expect("info");
    let mut buf = vec![0; reader.output_buffer_size().expect("size")];
    let info = reader.next_frame(&mut buf).expect("frame");
    buf[..info.buffer_size()]
        .chunks_exact(4)
        .filter(|p| (p[0] as i32 - 0xff).abs() < 30 && (p[1] as i32 - 0xd8).abs() < 30 && (p[2] as i32 - 0x4a).abs() < 30)
        .count()
}

fn main() {
    let base = std::fs::read_to_string("out/map.mmd").expect("run the main binary first");
    let r = Renderer::new();
    let variants = [
        ("none", String::new()),
        ("class applyfn sel", "  classDef sel stroke:#ffd84a,stroke-width:6px\n  class applyfn sel\n".to_string()),
        ("style applyfn …", "  style applyfn stroke:#ffd84a,stroke-width:6px\n".to_string()),
        (
            "classDef selLogic (full kind style + ring), class applyfn selLogic",
            "  classDef selLogic fill:#8f9be8,stroke:#ffd84a,stroke-width:6px,color:#101433,font-family:monospace\n  class applyfn selLogic\n"
                .to_string(),
        ),
        ("logic classDef itself gets the yellow stroke", "@@LOGIC".to_string()),
        ("applyfn removed from logic, then class applyfn selLogic", "@@MOVE".to_string()),
    ];
    for (name, extra) in variants {
        let src = match extra.as_str() {
            "@@LOGIC" => base.replace("stroke:#4c58a8,stroke-width:1px", "stroke:#ffd84a,stroke-width:6px"),
            "@@MOVE" => base.replace("class applyfn,", "class ")
                + "  classDef selLogic fill:#8f9be8,stroke:#ffd84a,stroke-width:6px,color:#101433,font-family:monospace\n  class applyfn selLogic\n",
            _ => format!("{base}{extra}"),
        };
        let out = r
            .render(RenderRequest::png(
                &src,
                OperationControl::new(),
                PngRequest { svg: SvgRequest::default(), options: RasterOptions::default().with_scale(1.0) },
            ))
            .expect("render");
        let RenderOutput::Png(Some(o)) = out else { panic!() };
        println!("{:>7} yellow px  {name}", yellow_pixels(&o.bytes));
    }
}
