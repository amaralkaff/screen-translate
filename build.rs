fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();

        let ico_path = "assets/icon.ico";
        if !std::path::Path::new(ico_path).exists()
            && std::path::Path::new("assets/logo.png").exists()
        {
            generate_ico("assets/logo.png", ico_path);
        }

        if std::path::Path::new(ico_path).exists() {
            res.set_icon(ico_path);
        }
        res.compile().expect("Failed to compile Windows resources");
    }
}

#[cfg(target_os = "windows")]
fn generate_ico(png_path: &str, ico_path: &str) {
    use image::codecs::ico::{IcoEncoder, IcoFrame};
    use image::imageops::FilterType;
    use image::ExtendedColorType;

    let img = image::open(png_path).expect("Failed to open logo.png");

    let sizes = [16u32, 32, 48, 256];
    let mut frames = Vec::new();
    for &size in &sizes {
        let resized = img
            .resize(size, size, FilterType::Lanczos3)
            .to_rgba8();
        let (w, h) = resized.dimensions();
        let frame =
            IcoFrame::as_png(&resized.into_raw(), w, h, ExtendedColorType::Rgba8)
                .expect("Failed to create ICO frame");
        frames.push(frame);
    }

    let file = std::fs::File::create(ico_path).expect("Failed to create icon.ico");
    let encoder = IcoEncoder::new(file);
    encoder
        .encode_images(&frames)
        .expect("Failed to encode ICO");

    println!("cargo:warning=Generated {} from {}", ico_path, png_path);
}
