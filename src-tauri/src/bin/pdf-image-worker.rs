#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType};
use pdfium_render::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    library: PathBuf,
    document: PathBuf,
    index: PathBuf,
    patch: PathBuf,
    directory: PathBuf,
    preset: String,
    #[serde(default)] quality: Option<u8>,
    #[serde(default)] dpi: Option<u32>,
}
#[derive(Clone)]
struct Candidate {
    key: String,
    dict: Value,
    bytes: u64,
}
struct Optimizer<'a> {
    directory: &'a Path,
    candidates: HashMap<[u8; 32], Vec<Candidate>>,
    visited: HashSet<[u8; 32]>,
    replacements: serde_json::Map<String, Value>,
    quality: u8,
    dpi: u32,
    examined: usize,
    uses: HashMap<[u8; 32], usize>,
}
impl Optimizer<'_> {
    fn count_uses(&mut self, object: PdfPageObject<'_>, depth: usize) -> Result<(), String> {
        if depth > 16 || self.examined > 200000 { return Err("This PDF is too complex for image compression.".into()); }
        self.examined += 1;
        if let Some(form) = object.as_x_object_form_object() {
            for child in form.iter() { self.count_uses(child, depth + 1)?; }
        } else if let Some(image) = object.as_image_object() {
            if let Ok(raw) = image.get_raw_image_data() {
                if raw.len() <= 32 * 1024 * 1024 { let hash: [u8; 32] = Sha256::digest(&raw).into(); *self.uses.entry(hash).or_default() += 1; }
            }
        }
        Ok(())
    }
    fn visit(&mut self, object: PdfPageObject<'_>, depth: usize) -> Result<(), String> {
        if depth > 16 || self.examined > 200000 {
            return Err("This PDF is too complex for image compression.".into());
        }
        self.examined += 1;
        if let Some(form) = object.as_x_object_form_object() {
            for child in form.iter() {
                self.visit(child, depth + 1)?;
            }
        } else if let Some(image) = object.as_image_object() {
            let Ok(width) = image.width() else {
                return Ok(());
            };
            let Ok(height) = image.height() else {
                return Ok(());
            };
            if width <= 0 || height <= 0 || (width as u64) * (height as u64) > 32_000_000 {
                return Ok(());
            }
            let Ok(raw) = image.get_raw_image_data() else {
                return Ok(());
            };
            if raw.len() > 32 * 1024 * 1024 {
                return Ok(());
            }
            let hash: [u8; 32] = Sha256::digest(&raw).into();
            if !self.visited.insert(hash) {
                return Ok(());
            }
            let Some(candidates) = self.candidates.get(&hash) else {
                return Ok(());
            };
            let first = &candidates[0];
            // Same bytes with different colour/decode dictionaries are ambiguous: preserve them.
            if candidates.iter().any(|item| {
                ["/ColorSpace", "/Decode", "/Width", "/Height", "/Filter"]
                    .iter()
                    .any(|key| item.dict.get(*key) != first.dict.get(*key))
            }) {
                return Ok(());
            }
            if first.dict["/Width"].as_u64() != Some(width as u64)
                || first.dict["/Height"].as_u64() != Some(height as u64)
            {
                return Ok(());
            }
            let Ok(decoded) = image.get_raw_image() else {
                return Ok(());
            };
            if decoded.width() != width as u32 || decoded.height() != height as u32 {
                return Ok(());
            }
            if let Some(rgba) = decoded.as_rgba8() {
                if rgba.pixels().any(|pixel| pixel[3] != 255) {
                    return Ok(());
                }
            }
            // Nested form transforms and reused images may have different physical sizes.
            // Preserve those images rather than guessing the lowest safe resolution.
            if depth > 0 || candidates.len() != 1 || self.uses.get(&hash).copied().unwrap_or(0) != 1 { return Ok(()); }
            let Ok(bounds) = object.bounds() else { return Ok(()); };
            let display_width = bounds.width().value.abs() as f64;
            let display_height = bounds.height().value.abs() as f64;
            if display_width <= 0.0 || display_height <= 0.0 { return Ok(()); }
            let scale = ((display_width * self.dpi as f64 / 72.0) / width as f64)
                .max((display_height * self.dpi as f64 / 72.0) / height as f64).min(1.0);
            let target_width = ((width as f64 * scale).round() as u32).max(1);
            let target_height = ((height as f64 * scale).round() as u32).max(1);
            let resized = decoded
                .resize_exact(target_width, target_height, FilterType::Lanczos3)
                .to_rgb8();
            let mut encoded = Vec::new();
            JpegEncoder::new_with_quality(&mut encoded, self.quality)
                .encode(
                    &resized,
                    target_width,
                    target_height,
                    image::ExtendedColorType::Rgb8,
                )
                .map_err(|_| "Could not encode a compressed image.")?;
            if encoded.len() as u64 + 64 >= first.bytes {
                return Ok(());
            }
            let path = self
                .directory
                .join(format!("compressed-image-{}.jpg", self.replacements.len()));
            fs::write(&path, encoded).map_err(|_| "Could not write a compressed image.")?;
            for item in candidates {
                let mut dict = item.dict.clone();
                let map = dict.as_object_mut().ok_or("Invalid image dictionary.")?;
                for key in ["/Decode", "/DecodeParms", "/Length", "/SMaskInData"] {
                    map.remove(key);
                }
                map.insert("/Width".into(), target_width.into());
                map.insert("/Height".into(), target_height.into());
                map.insert("/ColorSpace".into(), "/DeviceRGB".into());
                map.insert("/BitsPerComponent".into(), 8.into());
                map.insert("/Filter".into(), "/DCTDecode".into());
                self.replacements.insert(
                    item.key.clone(),
                    json!({ "stream": { "dict": dict, "datafile": path } }),
                );
            }
        }
        Ok(())
    }
}
fn process(config: Config) -> Result<usize, String> {
    let directory =
        fs::canonicalize(&config.directory).map_err(|_| "Image workspace unavailable.")?;
    if fs::metadata(&config.index)
        .map_err(|_| "Image index unavailable.")?
        .len()
        > 32 * 1024 * 1024
    {
        return Err("Image index exceeds the supported limit.".into());
    }
    let index: Value =
        serde_json::from_slice(&fs::read(&config.index).map_err(|_| "Image index unavailable.")?)
            .map_err(|_| "Invalid image index.")?;
    let objects = index
        .pointer("/qpdf/1")
        .and_then(Value::as_object)
        .ok_or("Invalid image index.")?;
    let mut candidates: HashMap<[u8; 32], Vec<Candidate>> = HashMap::new();
    for (key, object) in objects {
        let Some(stream) = object.get("stream") else {
            continue;
        };
        let Some(dict) = stream.get("dict") else {
            continue;
        };
        if dict["/Subtype"] != "/Image"
            || dict["/BitsPerComponent"] != 8
            || dict.get("/SMask").is_some()
            || dict.get("/Mask").is_some()
            || dict["/ImageMask"] == true
        {
            continue;
        }
        let Some(width) = dict["/Width"].as_u64() else {
            continue;
        };
        let Some(height) = dict["/Height"].as_u64() else {
            continue;
        };
        if width == 0 || height == 0 || width.saturating_mul(height) > 32_000_000 {
            continue;
        }
        let Some(path) = stream["datafile"].as_str() else {
            continue;
        };
        let data = fs::canonicalize(path).map_err(|_| "Image data unavailable.")?;
        if !data.starts_with(&directory) {
            return Err("Image data is outside the private workspace.".into());
        }
        let bytes = fs::metadata(&data)
            .map_err(|_| "Image data unavailable.")?
            .len();
        if !(8 * 1024..=32 * 1024 * 1024).contains(&bytes) {
            continue;
        }
        let raw = fs::read(&data).map_err(|_| "Image data unavailable.")?;
        let hash: [u8; 32] = Sha256::digest(&raw).into();
        candidates.entry(hash).or_default().push(Candidate {
            key: key.clone(),
            dict: dict.clone(),
            bytes,
        });
    }
    let pdfium = Pdfium::new(
        Pdfium::bind_to_library(&config.library).map_err(|_| "Image engine could not start.")?,
    );
    let document = pdfium
        .load_pdf_from_file(&config.document, None)
        .map_err(|_| "Image engine could not read this PDF.")?;
    let (quality, dpi) = match config.preset.as_str() {
        "balanced" => (80, 240),
        "small" => (65, 150),
        _ => return Err("Invalid image quality setting.".into()),
    };
    let mut optimizer = Optimizer {
        directory: &directory,
        candidates,
        visited: HashSet::new(),
        replacements: serde_json::Map::new(),
        quality: config.quality.unwrap_or(quality),
        dpi: config.dpi.unwrap_or(dpi),
        examined: 0,
        uses: HashMap::new(),
    };
    for page in document.pages().iter() { for object in page.objects().iter() { optimizer.count_uses(object, 0)?; } }
    optimizer.examined = 0;
    for page in document.pages().iter() {
        for object in page.objects().iter() {
            optimizer.visit(object, 0)?;
        }
    }
    let count = optimizer.replacements.len();
    fs::write(
        config.patch,
        serde_json::to_vec(&json!({ "qpdf": [index["qpdf"][0].clone(), optimizer.replacements] }))
            .map_err(|_| "Could not prepare compressed images.")?,
    )
    .map_err(|_| "Could not write compressed images.")?;
    Ok(count)
}
fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() == 3 && args[1] == "--check-runtime" {
        match Pdfium::bind_to_library(PathBuf::from(&args[2])) {
            Ok(bindings) => {
                let _pdfium = Pdfium::new(bindings);
                println!("PDFium image engine ready");
            }
            Err(_) => std::process::exit(2),
        }
        return;
    }
    let result = (|| {
        let path = args.get(1).ok_or("Missing image operation.")?;
        let bytes = fs::read(path).map_err(|_| "Image operation unavailable.")?;
        let config = serde_json::from_slice(&bytes).map_err(|_| "Invalid image operation.")?;
        process(config)
    })();
    match result {
        Ok(images) => println!("{}", json!({ "images": images })),
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    }
}
