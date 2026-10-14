use image::{ImageDecoder, ImageReader};
use std::{fs, io::{Cursor, Write}, path::Path};

pub fn supported(path: &Path) -> bool {
    path.extension().is_some_and(|ext| matches!(ext.to_string_lossy().to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp"))
}

pub fn convert(source: &Path, destination: &Path) -> Result<(), String> {
    let data = fs::read(source).map_err(|_| "Cannot read image.")?;
    let mut reader = ImageReader::new(Cursor::new(data)).with_guessed_format().map_err(|_| "Cannot read image format.")?;
    if !matches!(reader.format(), Some(image::ImageFormat::Jpeg | image::ImageFormat::Png | image::ImageFormat::WebP)) {
        return Err("Choose JPEG, PNG, or WebP images.".into());
    }
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits.max_image_width = Some(16000);
    limits.max_image_height = Some(16000);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| "Cannot decode image.")?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 32_000_000 {
        return Err("Image exceeds the 32 megapixel limit.".into());
    }
    let orientation = decoder.orientation().map_err(|_| "Cannot read image orientation.")?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|_| "Cannot decode image.")?;
    image.apply_orientation(orientation);
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    for pixel in rgba.pixels() {
        let alpha = u32::from(pixel[3]);
        for channel in &pixel.0[..3] {
            pixels.push(((u32::from(*channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8);
        }
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&pixels).map_err(|_| "Cannot convert image.")?;
    let compressed = encoder.finish().map_err(|_| "Cannot convert image.")?;
    let scale = 792.0 / f64::from(width.max(height));
    let page_width = f64::from(width) * scale;
    let page_height = f64::from(height) * scale;
    let content = format!("q {page_width:.4} 0 0 {page_height:.4} 0 0 cm /Image Do Q\n");
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_width:.4} {page_height:.4}] /Resources << /XObject << /Image 4 0 R >> >> /Contents 5 0 R >>").into_bytes(),
    ];
    let mut stream = format!("<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n", compressed.len()).into_bytes();
    stream.extend(compressed); stream.extend(b"\nendstream"); objects.push(stream);
    objects.push(format!("<< /Length {} >>\nstream\n{content}endstream", content.len()).into_bytes());
    let mut pdf = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend(object); pdf.extend(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) { pdf.extend(format!("{offset:010} 00000 n \n").as_bytes()); }
    pdf.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len()).as_bytes());
    fs::write(destination, pdf).map_err(|_| "Cannot create image page.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supported_images_become_clean_single_pages() {
        let folder = tempfile::tempdir().unwrap();
        for (suffix, format) in [("png", image::ImageFormat::Png), ("jpg", image::ImageFormat::Jpeg), ("webp", image::ImageFormat::WebP)] {
            let source = folder.path().join(format!("source.{suffix}"));
            let output = folder.path().join(format!("page-{suffix}.pdf"));
            let fixture = image::RgbImage::from_pixel(40, 20, image::Rgb([20, 80, 160]));
            fixture.save_with_format(&source, format).unwrap();
            let original = fs::read(&source).unwrap();
            convert(&source, &output).unwrap();
            let qpdf = Path::new(env!("CARGO_MANIFEST_DIR")).join("engine/qpdf");
            let checked = std::process::Command::new(&qpdf).arg("--check").arg(&output).output().unwrap();
            assert!(checked.status.success(), "{}", String::from_utf8_lossy(&checked.stderr));
            let pages = std::process::Command::new(&qpdf).arg("--show-npages").arg(&output).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&pages.stdout).trim(), "1");
            assert_eq!(fs::read(&source).unwrap(), original);
            assert!(String::from_utf8_lossy(&fs::read(&output).unwrap()).contains("/MediaBox [0 0 792.0000 396.0000]"));
        }
        let qpdf = Path::new(env!("CARGO_MANIFEST_DIR")).join("engine/qpdf");
        let merged = folder.path().join("merged.pdf");
        let status = std::process::Command::new(&qpdf).args(["--empty", "--pages"])
            .arg(folder.path().join("page-png.pdf")).arg("1")
            .arg(folder.path().join("page-jpg.pdf")).arg("1")
            .arg(folder.path().join("page-webp.pdf")).arg("1")
            .arg("--").arg(&merged).status().unwrap();
        assert!(status.success());
        let pages = std::process::Command::new(&qpdf).arg("--show-npages").arg(&merged).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&pages.stdout).trim(), "3");
    }
    #[test]
    fn transparency_is_white_and_corrupt_input_is_rejected() {
        use std::io::Read;
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("transparent.png");
        let output = folder.path().join("page.pdf");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 0])).save(&source).unwrap();
        convert(&source, &output).unwrap();
        let pdf = fs::read(&output).unwrap();
        let start = pdf.windows(7).position(|s| s == b"stream\n").unwrap() + 7;
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(&pdf[start..]).read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, [255, 255, 255]);
        fs::write(&source, b"invalid image").unwrap();
        assert!(convert(&source, &output).is_err());
    }
}
