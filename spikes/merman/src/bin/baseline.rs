//! Binary-size baseline: same profile and the `png` crate, no merman.
fn main() {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, 1, 1);
    enc.set_color(png::ColorType::Rgba);
    enc.write_header().expect("header").write_image_data(&[0, 0, 0, 0]).expect("data");
    println!("{}", out.len());
}
