#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod workspace;
mod image_import;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{ipc::Channel, Manager};
use tauri_plugin_updater::UpdaterExt;

const MAX_BYTES: u64 = 250 * 1024 * 1024;
const MAX_PREVIEW_CHUNK: u64 = 1024 * 1024;
const MAX_DOCUMENTS: usize = 100;
const MAX_PAGES: u32 = 10000;
const MAX_OUTPUTS: usize = 500;

#[derive(Clone)]
struct Selected {
    path: PathBuf,
    bytes: u64,
    pages: u32,
    digest: [u8; 32],
    source_digest: [u8; 32],
    working: Option<Arc<tempfile::NamedTempFile>>,
    features: Features,
    locked: bool,
}
impl Selected { fn working_path(&self) -> &Path { self.working.as_ref().map_or(self.path.as_path(), |file| file.path()) } }
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Features {
    forms: bool, signatures: bool, tags: bool, attachments: usize,
    needs_appearances: bool, unsupported_forms: bool,
    bookmarks: Vec<Bookmark>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Bookmark { title: String, page: Option<u32>, children: Vec<Bookmark> }

struct Store {
    workspace: workspace::Workspace,
    documents: Mutex<HashMap<String, Selected>>,
    dropped_paths: Mutex<HashSet<PathBuf>>,
    operation: Mutex<()>,
    job: Mutex<Option<(String, Arc<AtomicBool>)>>,
    results: Mutex<HashMap<String, PathBuf>>,
    compression_preview: Mutex<Option<Arc<CompressionPreview>>>,
    pending_update: Mutex<Option<(String, tauri_plugin_updater::Update)>>,
}
struct CompressionPreview {
    id: String,
    ids: Vec<String>,
    compression: Compression,
    target_bytes: Option<u64>,
    outputs: Vec<tempfile::NamedTempFile>,
    digests: Vec<[u8; 32]>,
    sizes: Vec<u64>,
    recovered: bool,
    // Keep the private directory alive until the cached outputs are released.
    _workspace: tempfile::TempDir,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentInfo {
    id: String,
    name: String,
    bytes: u64,
    pages: u32,
    preview_bytes: u64,
    locked: bool,
    features: Features,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    id: String,
    name: String,
    files: usize,
    input_bytes: u64,
    output_bytes: u64,
    file_bytes: Vec<u64>,
    recovered: bool,
}
#[derive(Clone, Serialize)]
struct Progress {
    message: String,
    cancellable: bool,
}
#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Merge,
    Compress,
    Split,
    Organize,
}
#[derive(Clone, Copy, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Compression {
    #[default]
    Lossless,
    Balanced,
    Small,
}
#[derive(Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum SplitMode {
    #[default]
    EveryPage,
    Groups,
    Ranges,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageEdit {
    page: u32,
    rotation: u16,
    #[serde(default)]
    document_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    job_id: String,
    operation: Operation,
    ids: Vec<String>,
    #[serde(default)]
    compression: Compression,
    #[serde(default)]
    split_mode: SplitMode,
    #[serde(default)]
    pages_per_file: u32,
    #[serde(default)]
    ranges: String,
    #[serde(default)]
    pages: Vec<PageEdit>,
    #[serde(default)]
    measure_only: bool,
    #[serde(default)]
    preview_id: Option<String>,
    #[serde(default)] target_bytes: Option<u64>,
    #[serde(default)] output_names: Vec<String>,
    #[serde(default)] flatten_forms: bool,
    #[serde(default)] acknowledge_changes: bool,
}
struct JobLease {
    store: Arc<Store>,
    id: String,
}
impl Drop for JobLease {
    fn drop(&mut self) {
        if let Ok(mut job) = self.store.job.lock() {
            if job.as_ref().is_some_and(|(id, _)| id == &self.id) {
                *job = None;
            }
        }
    }
}
fn cancelled(cancel: Option<&AtomicBool>) -> Result<(), String> {
    if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        Err("Cancelled. No output was saved.".into())
    } else {
        Ok(())
    }
}
fn progress(channel: &Channel<Progress>, message: impl Into<String>, cancellable: bool) {
    let _ = channel.send(Progress {
        message: message.into(),
        cancellable,
    });
}
fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
fn engine(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    if let Some(value) = std::env::var_os("PDF_TOOLS_QPDF") {
        let path = PathBuf::from(value);
        if path.is_absolute() && path.is_file() {
            return Ok(path);
        }
        return Err("PDF_TOOLS_QPDF must point to an absolute qpdf executable path.".into());
    }
    let filename = if cfg!(windows) { "qpdf.exe" } else { "qpdf" };
    let path = app
        .path()
        .resource_dir()
        .map_err(|_| "Cannot locate application resources.")?
        .join("engine")
        .join(filename);
    if !path.is_file() {
        return Err("The local PDF engine is missing. Install a complete copy of PDF Tools, or ask the developer to finish the engine bundle.".into());
    }
    Ok(path)
}
// No shell and no document diagnostic logs. Bound output and terminate stalled jobs.
fn run(command: &mut Command, cancel: Option<&AtomicBool>) -> Result<(i32, Vec<u8>), String> {
    cancelled(cancel)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "Cannot start the local PDF engine.")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Cannot read PDF engine results.")?;
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        stdout
            .take(2 * 1024 * 1024)
            .read_to_end(&mut output)
            .map(|_| output)
    });
    let started = Instant::now();
    let status = loop {
        if cancelled(cancel).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err("Cancelled. No output was saved.".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < Duration::from_secs(600) => {
                thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err("PDF processing stopped after an engine error or a ten-minute time limit. Try a smaller document.".into());
            }
        }
    };
    let output = reader
        .join()
        .map_err(|_| "Cannot read PDF engine results.")?
        .map_err(|_| "Cannot read PDF engine results.")?;
    cancelled(cancel)?;
    Ok((status.code().unwrap_or(-1), output))
}
fn checked(command: &mut Command, cancel: Option<&AtomicBool>) -> Result<Vec<u8>, String> {
    let (code, output) = run(command, cancel)?;
    if code != 0 {
        return Err(if code == 3 {
            "The PDF could not pass validation. No output was saved."
        } else {
            "The PDF engine could not process this file. No output was saved."
        }
        .into());
    }
    Ok(output)
}
fn check_version(engine: &Path) -> Result<(), String> {
    let output = checked(Command::new(engine).arg("--version"), None)?;
    let text = String::from_utf8_lossy(&output);
    let version = text
        .split_whitespace()
        .nth(2)
        .ok_or("Cannot identify qpdf version.")?;
    let mut parts = version
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    if major < 12 || (major == 12 && minor < 1) {
        return Err("qpdf 12.1 or newer is required.".into());
    }
    Ok(())
}
fn digest(path: &Path, cancel: Option<&AtomicBool>) -> Result<[u8; 32], String> {
    let mut file = File::open(path).map_err(|_| "Cannot read an input document.")?;
    let mut hash = Sha256::new();
    let mut chunk = [0u8; 65536];
    let mut total = 0u64;
    loop {
        cancelled(cancel)?;
        let count = file
            .read(&mut chunk)
            .map_err(|_| "Cannot read an input document.")?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_BYTES {
            return Err("Choose a PDF no larger than 250 MB.".into());
        }
        hash.update(&chunk[..count]);
    }
    Ok(hash.finalize().into())
}
fn count_pages(engine: &Path, path: &Path, cancel: Option<&AtomicBool>) -> Result<u32, String> {
    let output = checked(Command::new(engine).arg("--show-npages").arg(path), cancel)?;
    String::from_utf8_lossy(&output)
        .trim()
        .parse::<u32>()
        .map_err(|_| "Cannot count PDF pages.".into())
}
fn inspect(engine: &Path, path: &Path, cancel: Option<&AtomicBool>) -> Result<Selected, String> {
    let path = fs::canonicalize(path).map_err(|_| "Cannot access this document.")?;
    let metadata = fs::metadata(&path).map_err(|_| "Cannot inspect this document.")?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err("Choose a PDF no larger than 250 MB.".into());
    }
    let mut signature = [0u8; 5];
    File::open(&path)
        .and_then(|mut file| file.read_exact(&mut signature))
        .map_err(|_| "Cannot read this document.")?;
    if &signature != b"%PDF-" {
        return Err("The selected file does not have a PDF header.".into());
    }
    let (encrypted, _) = run(
        Command::new(engine).arg("--is-encrypted").arg(&path),
        cancel,
    )?;
    if encrypted != 2 {
        return Err("Encrypted PDFs are not supported. Choose an unencrypted working copy.".into());
    }
    let features = inspect_features(engine, &path, cancel)?;
    let pages = count_pages(engine, &path, cancel)?;
    if pages == 0 || pages > MAX_PAGES {
        return Err("Choose a document with 1 to 10,000 pages.".into());
    }
    Ok(Selected {
        digest: digest(&path, cancel)?,
        source_digest: digest(&path, cancel)?,
        working: None, features, locked: false,
        path,
        bytes: metadata.len(),
        pages,
    })
}
fn inspect_json(engine: &Path, path: &Path, cancel: Option<&AtomicBool>) -> Result<serde_json::Value, String> {
    let json_file = tempfile::NamedTempFile::new_in(path.parent().ok_or("Cannot inspect PDF information.")?).map_err(|_| "Cannot inspect PDF information.")?;
    checked(Command::new(engine).args(["--json", "--json-stream-data=none"]).arg(path).arg(json_file.path()), cancel)?;
    if json_file.as_file().metadata().map_err(|_| "Cannot inspect PDF information.")?.len() > 64 * 1024 * 1024 { return Err("Document structure is too complex.".into()); }
    serde_json::from_reader(json_file.as_file()).map_err(|_| "Cannot inspect PDF information.".into())
}
fn bookmarks(values: &serde_json::Value) -> Vec<Bookmark> {
    values.as_array().map(|items| items.iter().map(|item| Bookmark {
        title: item["title"].as_str().unwrap_or("").to_owned(),
        page: item["destpageposfrom1"].as_u64().and_then(|n| u32::try_from(n).ok()),
        children: bookmarks(&item["kids"]),
    }).collect()).unwrap_or_default()
}
fn inspect_features(engine: &Path, path: &Path, cancel: Option<&AtomicBool>) -> Result<Features, String> {
    let json = inspect_json(engine, path, cancel)?;
    let objects = json.pointer("/qpdf/1").and_then(|v| v.as_object()).ok_or("Cannot inspect document structure.")?;
    let root = objects.get("trailer").and_then(|v| v.pointer("/value/~1Root")).and_then(|v| v.as_str()).ok_or("Cannot inspect document catalog.")?;
    let catalog = &objects[&format!("obj:{root}")]["value"];
    let fields = json.pointer("/acroform/fields").and_then(|v| v.as_array()).ok_or("Cannot inspect forms.")?;
    let signatures = fields.iter().any(|field| field["fieldtype"] == "/Sig") || catalog.get("/Perms").is_some();
    Ok(Features {
        forms: !fields.is_empty(), signatures, tags: catalog.get("/StructTreeRoot").is_some(),
        attachments: json["attachments"].as_object().map_or(0, |items| items.len()),
        needs_appearances: json.pointer("/acroform/needappearances").and_then(|v| v.as_bool()).unwrap_or(false),
        unsupported_forms: catalog.pointer("/~1AcroForm/~1XFA").is_some() || objects.values().any(|obj| obj.pointer("/value/~1XFA").is_some()),
        bookmarks: bookmarks(&json["outlines"]),
    })
}
fn document_info(id: String, document: &Selected) -> DocumentInfo {
    DocumentInfo { id, name: name(&document.path), bytes: document.bytes, pages: document.pages,
        preview_bytes: document.working.as_ref().and_then(|file| file.as_file().metadata().ok()).map_or(document.bytes, |m| m.len()),
        locked: document.locked, features: document.features.clone() }
}
fn copy_into(source: &Path, destination: &File, cancel: Option<&AtomicBool>) -> Result<(), String> {
    let mut input = File::open(source).map_err(|_| "Cannot read an input document.")?;
    let mut output = destination
        .try_clone()
        .map_err(|_| "Cannot open the working copy.")?;
    output
        .set_len(0)
        .map_err(|_| "Cannot prepare a working copy.")?;
    output
        .seek(SeekFrom::Start(0))
        .map_err(|_| "Cannot prepare a working copy.")?;
    let mut chunk = [0u8; 65536];
    let mut total = 0u64;
    loop {
        cancelled(cancel)?;
        let count = input
            .read(&mut chunk)
            .map_err(|_| "Cannot read an input document.")?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_BYTES {
            return Err("An input document exceeds 250 MB. No output was saved.".into());
        }
        output
            .write_all(&chunk[..count])
            .map_err(|_| "Cannot write a working copy. Check available disk space.")?;
    }
    output
        .sync_all()
        .map_err(|_| "Cannot finish writing a working copy.")?;
    Ok(())
}
fn parse_range(value: &str, total: u32) -> Result<Vec<u32>, String> {
    if value.trim().is_empty() {
        return Err("Enter a page range.".into());
    }
    let mut result = Vec::new();
    for segment in value.split(',') {
        let parts: Vec<&str> = segment.trim().split('-').map(str::trim).collect();
        if parts.is_empty()
            || parts.len() > 2
            || parts
                .iter()
                .any(|part| part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()))
        {
            return Err("Use page numbers or ranges such as 1-3,5.".into());
        }
        let first = parts[0]
            .parse::<u32>()
            .map_err(|_| "Invalid page number.")?;
        let last = parts
            .last()
            .unwrap()
            .parse::<u32>()
            .map_err(|_| "Invalid page number.")?;
        if first == 0 || last < first || last > total {
            return Err(format!(
                "Use page numbers between 1 and {total}, with ranges in ascending order."
            ));
        }
        if result.len() + (last - first + 1) as usize > MAX_PAGES as usize {
            return Err("Choose at most 10,000 pages per output.".into());
        }
        result.extend(first..=last);
    }
    Ok(result)
}
fn split_plan(request: &Request, total: u32) -> Result<Vec<Vec<u32>>, String> {
    let result = match request.split_mode {
        SplitMode::Ranges => {
            if request.ranges.len() > 20000 {
                return Err("The page range list is too long.".into());
            }
            let ranges: Vec<&str> = request.ranges.split(';').collect();
            if ranges.len() > MAX_OUTPUTS {
                return Err("Create at most 500 PDFs at once.".into());
            }
            ranges
                .into_iter()
                .map(|range| parse_range(range, total))
                .collect::<Result<Vec<_>, _>>()?
        }
        SplitMode::EveryPage | SplitMode::Groups => {
            let size = if matches!(request.split_mode, SplitMode::EveryPage) {
                1
            } else {
                request.pages_per_file
            };
            if size == 0 || size > total {
                return Err(format!("Use a group size between 1 and {total}."));
            }
            if total.div_ceil(size) as usize > MAX_OUTPUTS {
                return Err("Create at most 500 PDFs at once. Choose a larger group size.".into());
            }
            (1..=total)
                .step_by(size as usize)
                .map(|start| (start..=total.min(start + size - 1)).collect())
                .collect()
        }
    };
    if result.iter().map(Vec::len).sum::<usize>() > 20000 {
        return Err("Choose at most 20,000 output pages in total.".into());
    }
    Ok(result)
}
fn range_argument(pages: &[u32]) -> String {
    // Collapse consecutive pages to keep Windows command lines short.
    let mut chunks = Vec::new();
    let mut index = 0;
    while index < pages.len() {
        let first = pages[index];
        let mut last = first;
        while index + 1 < pages.len() && pages[index + 1] == last + 1 {
            index += 1;
            last = pages[index];
        }
        chunks.push(if first == last {
            first.to_string()
        } else {
            format!("{first}-{last}")
        });
        index += 1;
    }
    chunks.join(",")
}
fn validate_output(
    engine: &Path,
    output: &Path,
    expected: usize,
    cancel: &AtomicBool,
) -> Result<u64, String> {
    checked(
        Command::new(engine).arg("--check").arg(output),
        Some(cancel),
    )?;
    if count_pages(engine, output, Some(cancel))? as usize != expected {
        return Err("Output pages do not match the requested pages. No output was saved.".into());
    }
    let bytes = fs::metadata(output)
        .map_err(|_| "Cannot inspect output size.")?
        .len();
    if bytes == 0 {
        return Err("The PDF engine produced an empty file. No output was saved.".into());
    }
    Ok(bytes)
}
fn image_runtime(engine: &Path) -> Result<(PathBuf, PathBuf), String> {
    let directory = engine.parent().ok_or("Image engine unavailable.")?;
    let worker = directory.join(if cfg!(windows) {
        "pdf-image-worker.exe"
    } else {
        "pdf-image-worker"
    });
    let library = directory.join(if cfg!(windows) {
        "pdfium.dll"
    } else {
        "libpdfium.dylib"
    });
    if !worker.is_file() || !library.is_file() {
        return Err("The image engine is missing. Install a complete copy of PDF Tools.".into());
    }
    Ok((worker, library))
}
fn optimize_images(
    engine: &Path,
    output: &tempfile::NamedTempFile,
    workspace: &Path,
    compression: Compression,
    pages: usize,
    tuning: Option<(u8, u32)>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let (worker, library) = image_runtime(engine)?;
    let directory = tempfile::Builder::new()
        .prefix("images-")
        .tempdir_in(workspace)
        .map_err(|_| "Cannot create an image workspace.")?;
    let index = directory.path().join("index.json");
    let mut prefix = std::ffi::OsString::from("--json-stream-prefix=");
    prefix.push(directory.path().join("stream"));
    checked(
        Command::new(engine)
            .args(["--json-output=2", "--json-stream-data=file"])
            .arg(prefix)
            .arg(output.path())
            .arg(&index),
        Some(cancel),
    )?;
    let patch = directory.path().join("patch.json");
    let config = directory.path().join("operation.json");
    let operation = serde_json::json!({
        "library": library, "document": output.path(), "index": index, "patch": patch,
        "quality": tuning.map(|t| t.0), "dpi": tuning.map(|t| t.1),
        "directory": directory.path(), "preset": if compression == Compression::Balanced { "balanced" } else { "small" },
    });
    fs::write(
        &config,
        serde_json::to_vec(&operation).map_err(|_| "Cannot prepare image compression.")?,
    )
    .map_err(|_| "Cannot prepare image compression.")?;
    let result = checked(Command::new(worker).arg(config), Some(cancel)).map_err(|error| {
        if error.starts_with("Cancelled") {
            error
        } else {
            "Image compression could not finish. No output was saved.".into()
        }
    })?;
    let stats: serde_json::Value =
        serde_json::from_slice(&result).map_err(|_| "Invalid image compression result.")?;
    let images = stats["images"]
        .as_u64()
        .ok_or("Invalid image compression result.")?;
    if images == 0 {
        return Ok(());
    }
    let candidate = tempfile::Builder::new()
        .suffix(".pdf")
        .tempfile_in(directory.path())
        .map_err(|_| "Cannot create compressed output.")?;
    let mut update = std::ffi::OsString::from("--update-from-json=");
    update.push(patch);
    checked(
        Command::new(engine)
            .arg(update)
            .args(["--object-streams=generate", "--compression-level=9"])
            .arg(output.path())
            .arg(candidate.path()),
        Some(cancel),
    )?;
    let size = validate_output(engine, candidate.path(), pages, cancel)?;
    if size
        < fs::metadata(output.path())
            .map_err(|_| "Cannot inspect output size.")?
            .len()
    {
        copy_into(candidate.path(), output.as_file(), Some(cancel))?;
    }
    Ok(())
}

fn preserve_navigation(engine: &Path, output: &tempfile::NamedTempFile, documents: &[Selected], layout: &[(usize,u32,u16)], workspace: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let json = inspect_json(engine, output.path(), Some(cancel))?;
    let original = json.pointer("/qpdf/1").and_then(|v| v.as_object()).ok_or("Cannot check document navigation.")?;
    let root_ref = original["trailer"].pointer("/value/~1Root").and_then(|v| v.as_str()).ok_or("Cannot check document catalog.")?;
    let root_key = format!("obj:{root_ref}");
    let mut root = original[&root_key]["value"].clone();
    let identity = documents.len() == 1 && layout.len() == documents[0].pages as usize && layout.iter().enumerate().all(|(i,(d,p,_))| *d == 0 && *p == i as u32+1);
    let mut changes = serde_json::Map::new();
    if !identity {
        if let Some(root) = root.as_object_mut() { root.remove("/StructTreeRoot"); root.remove("/MarkInfo"); root.remove("/Metadata"); root.remove("/Dests"); }
        // Named destinations from a partial primary document can target discarded pages.
        if let Some(names_ref) = root.get("/Names").and_then(|v| v.as_str()) {
            let key = format!("obj:{names_ref}");
            if let Some(mut names) = original.get(&key).cloned() { if let Some(dict) = names["value"].as_object_mut() { dict.remove("/Dests"); changes.insert(key, names); } }
        }
        for page in json["pages"].as_array().ok_or("Cannot inspect output pages.")? {
            let key = format!("obj:{}",page["object"].as_str().ok_or("Invalid output page.")?);
            if let Some(mut obj) = original.get(&key).cloned() { if let Some(dict) = obj["value"].as_object_mut() { dict.remove("/StructParents"); changes.insert(key, obj); } }
        }
    }
    let page_refs: Vec<String> = json["pages"].as_array().ok_or("Cannot inspect output pages.")?.iter().map(|page| page["object"].as_str().unwrap_or("").to_string()).collect();
    #[derive(Clone)] struct Outline { title: String, destination: Option<String>, children: Vec<Outline> }
    fn map_bookmarks(items: &[Bookmark], document: usize, layout: &[(usize,u32,u16)], refs: &[String]) -> Vec<Outline> {
        items.iter().filter_map(|item| {
            let destination = item.page.and_then(|page| layout.iter().position(|(d,p,_)| *d == document && *p == page)).map(|i| refs[i].clone());
            let children = map_bookmarks(&item.children,document,layout,refs);
            if destination.is_none() && children.is_empty() { None } else { Some(Outline { title:item.title.clone(),destination,children }) }
        }).collect()
    }
    let outlines: Vec<_> = documents.iter().enumerate().flat_map(|(i,doc)| map_bookmarks(&doc.features.bookmarks,i,layout,&page_refs)).collect();
    let mut next = json.pointer("/qpdf/0/maxobjectid").and_then(|v|v.as_u64()).ok_or("Cannot inspect output objects.")? + 1;
    let outline_root = format!("{} 0 R",next); next+=1;
    fn emit(items: &[Outline], parent: &str, next: &mut u64, changes: &mut serde_json::Map<String,serde_json::Value>) -> Option<(String,String,usize)> {
        if items.is_empty() { return None; }
        let ids: Vec<_> = items.iter().map(|_| { let id=format!("{} 0 R",*next); *next+=1; id }).collect();
        let mut count=items.len();
        for (i,item) in items.iter().enumerate() {
            let mut obj=serde_json::json!({"/Title":format!("u:{}",item.title),"/Parent":parent});
            if i>0 { obj["/Prev"]=ids[i-1].clone().into(); } if i+1<ids.len() {obj["/Next"]=ids[i+1].clone().into();}
            if let Some(dest)=&item.destination { obj["/Dest"]=serde_json::json!([dest,"/Fit"]); }
            if let Some((first,last,children))=emit(&item.children,&ids[i],next,changes) { obj["/First"]=first.into(); obj["/Last"]=last.into(); obj["/Count"]=children.into(); count+=children; }
            changes.insert(format!("obj:{}",ids[i]),serde_json::json!({"value":obj}));
        }
        Some((ids[0].clone(),ids.last().unwrap().clone(),count))
    }
    if let Some((first,last,count))=emit(&outlines,&outline_root,&mut next,&mut changes) {
        changes.insert(format!("obj:{outline_root}"),serde_json::json!({"value":{"/Type":"/Outlines","/First":first,"/Last":last,"/Count":count}})); root["/Outlines"]=outline_root.into();
    } else if let Some(root)=root.as_object_mut() { root.remove("/Outlines"); }
    changes.insert(root_key,serde_json::json!({"value":root}));
    let patch=workspace.join("navigation.json");
    fs::write(&patch,serde_json::to_vec(&serde_json::json!({"qpdf":[{"jsonversion":2},changes]})).map_err(|_| "Cannot preserve navigation.")?).map_err(|_| "Cannot preserve navigation.")?;
    let candidate=tempfile::Builder::new().suffix(".pdf").tempfile_in(workspace).map_err(|_| "Cannot preserve navigation.")?;
    let mut argument=std::ffi::OsString::from("--update-from-json="); argument.push(patch);
    checked(Command::new(engine).arg(argument).arg(output.path()).arg(candidate.path()),Some(cancel))?;
    validate_output(engine,candidate.path(),layout.len(),cancel)?;
    copy_into(candidate.path(),output.as_file(),Some(cancel))
}

fn record_result(
    store: &Store,
    path: PathBuf,
    files: usize,
    input_bytes: u64,
    output_bytes: u64,
    recovered: bool,
) -> Result<Saved, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let mut results = store
        .results
        .lock()
        .map_err(|_| "The PDF was saved, but its folder cannot be shown.")?;
    if results.len() >= 100 {
        results.clear();
    }
    let saved = Saved {
        id: id.clone(),
        name: name(&path),
        files,
        input_bytes,
        output_bytes,
        file_bytes: Vec::new(),
        recovered,
    };
    results.insert(id, path);
    Ok(saved)
}

#[tauri::command]
async fn engine_status(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = engine(&app)?;
        check_version(&engine)?;
        let (worker, library) = image_runtime(&engine)?;
        checked(
            Command::new(worker).arg("--check-runtime").arg(library),
            None,
        )
        .map_err(|_| "The image engine could not start. Install a complete copy of PDF Tools.")?;
        Ok(())
    })
    .await
    .map_err(|_| "Engine check failed.")?
}
fn select_paths(
    store: &Store,
    engine: &Path,
    paths: Vec<PathBuf>,
) -> Result<Vec<DocumentInfo>, String> {
    if paths.len() > MAX_DOCUMENTS {
        return Err("Choose at most 100 documents at once.".into());
    }
    let selected = paths.iter().map(|path| {
        let path = fs::canonicalize(path).map_err(|_| "Cannot access this PDF.")?;
        let file = tempfile::Builder::new().suffix(".pdf").tempfile_in(store.workspace.path()).map_err(|_| "Cannot create a working copy.")?;
        copy_into(&path, file.as_file(), None)?;
        let source_digest = digest(file.path(), None)?;
        if image_import::supported(&path) {
            let converted = tempfile::Builder::new().suffix(".pdf").tempfile_in(store.workspace.path()).map_err(|_| "Cannot create image page.")?;
            image_import::convert(file.path(), converted.path())?;
            let mut document = inspect(engine, converted.path(), None)?;
            document.path = path; document.bytes = file.as_file().metadata().map_err(|_| "Cannot inspect image.")?.len();
            document.source_digest = source_digest; document.working = Some(Arc::new(converted));
            return Ok(document);
        }
        let (encrypted, _) = run(Command::new(engine).arg("--is-encrypted").arg(file.path()), None)?;
        if encrypted == 0 {
            let bytes = file.as_file().metadata().map_err(|_| "Cannot inspect PDF.")?.len();
            return Ok(Selected { path, bytes, pages: 0, digest: source_digest, source_digest, working: Some(Arc::new(file)), features: Features::default(), locked: true });
        }
        let mut document = inspect(engine, file.path(), None)?;
        document.path = path; document.working = Some(Arc::new(file));
        Ok(document)
    }).collect::<Result<Vec<_>, String>>()?;
    let mut stored = store
        .documents
        .lock()
        .map_err(|_| "Document selection is unavailable.")?;
    let new_count = selected
        .iter()
        .filter(|doc| {
            !stored
                .values()
                .any(|old| old.path == doc.path && old.digest == doc.digest)
        })
        .count();
    if stored.len() + new_count > MAX_DOCUMENTS {
        return Err(
            "Clear documents in another tool before selecting more than 100 documents.".into(),
        );
    }
    let mut result = Vec::new();
    let mut added = HashSet::new();
    for document in selected {
        if !added.insert(document.path.clone()) {
            continue;
        }
        let existing = stored
            .iter()
            .find(|(_, old)| old.path == document.path && old.digest == document.digest)
            .map(|(id, _)| id.clone());
        let id = existing.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        result.push(document_info(id.clone(), &document));
        stored.insert(id, document);
    }
    Ok(result)
}
#[tauri::command]
async fn import_dropped_documents(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Store>>,
    paths: Vec<String>,
    allow_images: bool,
) -> Result<Vec<DocumentInfo>, String> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = store
            .operation
            .try_lock()
            .map_err(|_| "Another operation is in progress.")?;
        if paths.is_empty() || paths.len() > MAX_DOCUMENTS {
            return Err("Drop between 1 and 100 PDFs.".into());
        }
        let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        {
            let mut allowed = store
                .dropped_paths
                .lock()
                .map_err(|_| "File drop is unavailable.")?;
            if paths.iter().any(|path| !allowed.contains(path)) {
                return Err("Drop the files into the app again.".into());
            }
            for path in &paths {
                allowed.remove(path);
            }
        }
        if paths.iter().any(|path| {
            !path.is_file()
                || !path
                    .extension()
                    .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("pdf"))
                    && !(allow_images && image_import::supported(path))
        }) {
            return Err(if allow_images { "Choose PDFs, JPEG, PNG, or WebP images." } else { "Drop PDF files only." }.into());
        }
        let engine = engine(&app)?;
        check_version(&engine)?;
        select_paths(&store, &engine, paths)
    })
    .await
    .map_err(|_| "PDF import failed.")?
}

#[tauri::command]
async fn choose_documents(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Store>>,
    multiple: bool,
    allow_images: bool,
) -> Result<Vec<DocumentInfo>, String> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = store
            .operation
            .try_lock()
            .map_err(|_| "Another operation is in progress.")?;
        let engine = engine(&app)?;
        check_version(&engine)?;
        let dialog = if allow_images { rfd::FileDialog::new().add_filter("PDFs and images", &["pdf", "jpg", "jpeg", "png", "webp"]) } else { rfd::FileDialog::new().add_filter("PDF documents", &["pdf"]) };
        let paths = if multiple {
            dialog.pick_files()
        } else {
            dialog.pick_file().map(|path| vec![path])
        };
        let Some(paths) = paths else {
            return Ok(Vec::new());
        };
        select_paths(&store, &engine, paths)
    })
    .await
    .map_err(|_| "Document selection failed.")?
}
#[tauri::command]
fn retain_documents(state: tauri::State<'_, Arc<Store>>, ids: Vec<String>) -> Result<(), String> {
    let _busy = state
        .operation
        .try_lock()
        .map_err(|_| "Another operation is in progress.")?;
    let keep: HashSet<_> = ids.into_iter().collect();
    state
        .documents
        .lock()
        .map_err(|_| "Document selection is unavailable.")?
        .retain(|id, _| keep.contains(id));
    Ok(())
}
#[tauri::command]
async fn unlock_document(app: tauri::AppHandle, state: tauri::State<'_, Arc<Store>>, id: String, password: String) -> Result<DocumentInfo, String> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = store.operation.try_lock().map_err(|_| "Another operation is in progress.")?;
        if password.len() > 1024 || password.contains(['\n', '\r', '\0']) { return Err("Invalid password.".into()); }
        let document = store.documents.lock().map_err(|_| "Document unavailable.")?.get(&id).cloned().ok_or("Document unavailable.")?;
        if !document.locked { return Ok(document_info(id, &document)); }
        let engine = engine(&app)?;
        let mut secret = tempfile::NamedTempFile::new_in(store.workspace.path()).map_err(|_| "Cannot unlock PDF.")?;
        secret.write_all(password.as_bytes()).map_err(|_| "Cannot unlock PDF.")?;
        let output = tempfile::Builder::new().suffix(".pdf").tempfile_in(store.workspace.path()).map_err(|_| "Cannot unlock PDF.")?;
        let mut argument = std::ffi::OsString::from("--password-file="); argument.push(secret.path());
        checked(Command::new(&engine).arg(argument).arg("--decrypt").arg(document.working_path()).arg(output.path()), None).map_err(|_| "Incorrect password or unsupported encryption.")?;
        let mut unlocked = inspect(&engine, output.path(), None)?;
        unlocked.path = document.path; unlocked.bytes = document.bytes; unlocked.source_digest = document.source_digest;
        unlocked.working = Some(Arc::new(output));
        let info = document_info(id.clone(), &unlocked);
        store.documents.lock().map_err(|_| "Document unavailable.")?.insert(id, unlocked);
        Ok(info)
    }).await.map_err(|_| "Cannot unlock PDF.")?
}
#[tauri::command]
async fn read_document_range(state: tauri::State<'_, Arc<Store>>, id: String, begin: u64, end: u64, preview_id: Option<String>, output_index: Option<usize>) -> Result<tauri::ipc::Response, String> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if end <= begin || end - begin > MAX_PREVIEW_CHUNK { return Err("Invalid preview range.".into()); }
        let mut file = if let Some(preview_id) = preview_id {
            let preview = store.compression_preview.lock().map_err(|_| "Preview unavailable.")?.as_ref().filter(|p| p.id == preview_id).cloned().ok_or("Preview expired.")?;
            preview.outputs.get(output_index.unwrap_or(0)).ok_or("Preview unavailable.")?.reopen().map_err(|_| "Preview unavailable.")?
        } else {
            let document = store.documents.lock().map_err(|_| "Document unavailable.")?.get(&id).cloned().ok_or("Document unavailable.")?;
            if document.locked { return Err("Unlock the PDF first.".into()); }
            File::open(document.working_path()).map_err(|_| "Preview unavailable.")?
        };
        if end > file.metadata().map_err(|_| "Preview unavailable.")?.len() { return Err("Invalid preview range.".into()); }
        file.seek(SeekFrom::Start(begin)).map_err(|_| "Preview unavailable.")?;
        let mut bytes = vec![0; (end - begin) as usize]; file.read_exact(&mut bytes).map_err(|_| "Preview unavailable.")?;
        Ok(tauri::ipc::Response::new(bytes))
    }).await.map_err(|_| "Preview unavailable.")?
}
#[tauri::command]
fn cancel_job(state: tauri::State<'_, Arc<Store>>, job_id: String) -> Result<(), String> {
    let job = state.job.lock().map_err(|_| "Job status is unavailable.")?;
    if let Some((id, cancel)) = job.as_ref() {
        if id == &job_id {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}
#[tauri::command]
fn discard_compression_preview(state: tauri::State<'_, Arc<Store>>) -> Result<(), String> {
    let _busy = state
        .operation
        .try_lock()
        .map_err(|_| "Another operation is in progress.")?;
    *state
        .compression_preview
        .lock()
        .map_err(|_| "Compression preview is unavailable.")? = None;
    Ok(())
}
#[tauri::command]
async fn process_documents(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Store>>,
    request: Request,
    on_progress: Channel<Progress>,
) -> Result<Option<Saved>, String> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = store.operation.try_lock().map_err(|_| "Another operation is in progress.")?;
        uuid::Uuid::parse_str(&request.job_id).map_err(|_| "Invalid job identifier.")?;
        let cancel = Arc::new(AtomicBool::new(false));
        *store.job.lock().map_err(|_| "Job status is unavailable.")? = Some((request.job_id.clone(), cancel.clone()));
        let _lease = JobLease { store: store.clone(), id: request.job_id.clone() };
        if request.measure_only {
            *store.compression_preview.lock().map_err(|_| "Compression preview is unavailable.")? = None;
        }
        if request.measure_only && request.operation != Operation::Compress {
            return Err("Only compression supports size measurement.".into());
        }
        match request.operation {
            Operation::Merge if (1..=MAX_DOCUMENTS).contains(&request.ids.len()) => (),
            Operation::Compress if (1..=MAX_DOCUMENTS).contains(&request.ids.len()) => (),
            Operation::Split if request.ids.len() == 1 => (),
            Operation::Organize if (1..=MAX_DOCUMENTS).contains(&request.ids.len()) => (),
            _ => return Err("Merge needs at least two PDFs. Split and Organize need one PDF.".into()),
        }
        if request.ids.iter().collect::<HashSet<_>>().len() != request.ids.len() { return Err("A document was selected twice.".into()); }
        let documents: Vec<Selected> = {
            let stored = store.documents.lock().map_err(|_| "Document selection is unavailable.")?;
            request.ids.iter().map(|id| stored.get(id).cloned().ok_or_else(|| "A selected document is no longer available.".to_string())).collect::<Result<_, _>>()?
        };
        if documents.iter().map(|doc| doc.pages as usize).sum::<usize>() > 20000 { return Err("Choose at most 20,000 input pages.".into()); }
        let multiple_outputs = request.operation == Operation::Split || (request.operation == Operation::Compress && documents.len() > 1);
        let plan = if request.operation == Operation::Split { split_plan(&request, documents[0].pages)? } else { Vec::new() };
        if let Some(target) = request.target_bytes {
            if request.operation != Operation::Compress || !(100_000..=MAX_BYTES).contains(&target) || request.compression == Compression::Lossless { return Err("Choose a target between 0.1 and 250 MB with image compression.".into()); }
        }
        if matches!(request.operation, Operation::Organize | Operation::Merge) && !request.pages.is_empty() {
            if request.pages.len() > 20000 { return Err("Choose at most 20,000 output pages.".into()); }
            for page in &request.pages {
                let index = request.ids.iter().position(|id| id == &page.document_id).unwrap_or(0);
                if (!page.document_id.is_empty() && !request.ids.contains(&page.document_id)) || page.page == 0 || page.page > documents[index].pages || ![0, 90, 180, 270].contains(&page.rotation) { return Err("Invalid page selection or rotation.".into()); }
            }
        } else if request.operation == Operation::Organize { return Err("Keep at least one page.".into()); }
        if documents.iter().any(|doc| doc.locked) { return Err("Unlock the PDFs first.".into()); }
        if documents.iter().any(|doc| doc.features.unsupported_forms) { return Err("XFA forms are unsupported. Use a flattened copy.".into()); }
        let page_operation = request.operation != Operation::Compress;
        if page_operation && documents.iter().any(|doc| doc.features.forms) && !request.flatten_forms { return Err("Select Flatten forms before saving.".into()); }
        if page_operation && documents.iter().any(|doc| doc.features.needs_appearances) { return Err("Save the filled form in its original application before flattening.".into()); }
        if documents.iter().any(|doc| doc.features.signatures || (page_operation && doc.features.tags)) && !request.acknowledge_changes { return Err("Review document changes before saving.".into()); }
        let engine = engine(&app)?; check_version(&engine)?;
        let suffix = match request.operation { Operation::Merge => "merged", Operation::Compress => "compressed", Operation::Split => "split", Operation::Organize => "organized" };
        let filename = if request.operation == Operation::Merge { "merged.pdf".into() } else { format!("{}-{suffix}.pdf", documents[0].path.file_stem().unwrap_or_default().to_string_lossy()) };
        if !request.measure_only {
            progress(&on_progress, if multiple_outputs { "Choose an output folder." } else { "Choose where to save a new PDF." }, false);
        }
        let destination = if request.measure_only { Some(std::env::temp_dir()) } else if multiple_outputs { rfd::FileDialog::new().pick_folder() } else {
            rfd::FileDialog::new().set_file_name(&filename).add_filter("PDF document", &["pdf"]).save_file()
        };
        let Some(mut destination) = destination else { return Ok(None); };
        if !request.measure_only && !multiple_outputs {
            if destination.extension().and_then(|value| value.to_str()).is_none_or(|value| !value.eq_ignore_ascii_case("pdf")) { destination.set_extension("pdf"); }
            if fs::symlink_metadata(&destination).is_ok() { return Err("Choose a new filename. Existing files are never overwritten.".into()); }
        }
        let parent = if request.measure_only || multiple_outputs { destination.as_path() } else { destination.parent().ok_or("Choose a valid output folder.")? };
        // Measurement uses private OS temporary storage; saving uses the destination filesystem.
        let workspace = store.workspace.job()?;
        let mut snapshots = Vec::new();
        for (index, document) in documents.iter().enumerate() {
            progress(&on_progress, format!("Preparing document {} of {}...", index + 1, documents.len()), true);
            cancelled(Some(&cancel))?;
            let snapshot = tempfile::Builder::new().suffix(".pdf").tempfile_in(workspace.path()).map_err(|_| "Cannot create a working copy.")?;
            copy_into(document.working_path(), snapshot.as_file(), Some(&cancel))?;
            let verified = inspect(&engine, snapshot.path(), Some(&cancel))?;
            if verified.digest != document.digest || verified.pages != document.pages || verified.bytes != fs::metadata(document.working_path()).map_err(|_| "Working copy unavailable.")?.len() { return Err("An input document changed. Remove it and add it again.".into()); }
            if document.features.signatures || (page_operation && document.features.forms) {
                let prepared = tempfile::Builder::new().suffix(".pdf").tempfile_in(workspace.path()).map_err(|_| "Cannot prepare forms.")?;
                let mut command = Command::new(&engine);
                if document.features.signatures { command.arg("--remove-restrictions"); }
                if page_operation && document.features.forms { command.arg("--flatten-annotations=all").arg("--remove-acroform"); }
                checked(command.arg(snapshot.path()).arg(prepared.path()), Some(&cancel))?;
                validate_output(&engine, prepared.path(), document.pages as usize, &cancel)?;
                copy_into(prepared.path(), snapshot.as_file(), Some(&cancel))?;
            }
            snapshots.push(snapshot);
        }
        let input_bytes = documents.iter().map(|doc| doc.bytes).sum::<u64>();
        let jobs = if request.operation == Operation::Split { plan.len() } else if multiple_outputs { documents.len() } else { 1 };
        let cached = if request.operation == Operation::Compress && !request.measure_only {
            if let Some(id) = &request.preview_id {
                let preview = store.compression_preview.lock().map_err(|_| "Compression preview is unavailable.")?
                    .as_ref().filter(|preview| &preview.id == id && preview.ids == request.ids && preview.compression == request.compression && preview.target_bytes == request.target_bytes)
                    .cloned().ok_or("Compression settings changed. Wait for the new size measurement.")?;
                Some(preview)
            } else { None }
        } else { None };
        let mut outputs = Vec::new(); let mut output_bytes = 0u64; let mut sizes = Vec::new();
        let mut recovered = cached.as_ref().is_some_and(|preview| preview.recovered);
        for index in 0..jobs {
            progress(&on_progress, if jobs == 1 { "Processing the PDF locally...".into() } else { format!("Creating PDF {} of {jobs}...", index + 1) }, true);
            let output = tempfile::Builder::new().suffix(".pdf").tempfile_in(workspace.path()).map_err(|_| "Cannot create temporary output.")?;
            if let Some(preview) = &cached {
                copy_into(preview.outputs[index].path(), output.as_file(), Some(&cancel))?;
                if digest(output.path(), Some(&cancel))? != preview.digests[index] {
                    return Err("The measured output changed. Check compression again.".into());
                }
                output_bytes += preview.sizes[index]; sizes.push(preview.sizes[index]); outputs.push(output);
                continue;
            }
            let path_text = |path: &Path| -> Result<String, String> {
                path.to_str().map(str::to_owned).ok_or_else(|| "Choose an output folder with a valid Unicode name.".into())
            };
            // QPDFJob JSON avoids Windows command-length limits for large page plans.
            let mut job = serde_json::json!({ "outputFile": path_text(output.path())? });
            let expected = match request.operation {
                Operation::Merge | Operation::Organize => {
                    job["inputFile"] = path_text(snapshots[0].path())?.into();
                    let edits: Vec<(usize, u32, u16)> = if request.pages.is_empty() {
                        documents.iter().enumerate().flat_map(|(i, doc)| (1..=doc.pages).map(move |p| (i,p,0))).collect()
                    } else { request.pages.iter().map(|p| (request.ids.iter().position(|id| id == &p.document_id).unwrap_or(0), p.page, p.rotation)).collect() };
                    job["pages"] = edits.iter().map(|(i,p,_)| Ok(serde_json::json!({ "file": path_text(snapshots[*i].path())?, "range": p.to_string() }))).collect::<Result<Vec<_>, String>>()?.into();
                    let rotations: Vec<String> = [90,180,270].iter().filter_map(|r| { let positions: Vec<u32> = edits.iter().enumerate().filter(|(_,(_,_,rotation))| rotation == r).map(|(i,_)| i as u32+1).collect(); if positions.is_empty() { None } else { Some(format!("+{r}:{}",range_argument(&positions))) } }).collect();
                    if !rotations.is_empty() { job["rotate"] = serde_json::json!(rotations); }
                    if documents.len() > 1 { job["copyAttachmentsFrom"] = snapshots.iter().enumerate().skip(1).map(|(i,file)| Ok(serde_json::json!({"file": path_text(file.path())?, "prefix": format!("document-{}-",i+1)}))).collect::<Result<Vec<_>, String>>()?.into(); }
                    edits.len()
                }
                Operation::Compress => {
                    job["inputFile"] = path_text(snapshots[index].path())?.into();
                    job["compressStreams"] = "y".into();
                    job["recompressFlate"] = "".into();
                    job["compressionLevel"] = "9".into();
                    job["objectStreams"] = "generate".into();
                    documents[index].pages as usize
                }
                Operation::Split => {
                    job["inputFile"] = path_text(snapshots[0].path())?.into();
                    job["pages"] = serde_json::json!([{ "file": path_text(snapshots[0].path())?, "range": range_argument(&plan[index]) }]);
                    plan[index].len()
                }
            };
            let config = tempfile::Builder::new().suffix(".json").tempfile_in(workspace.path()).map_err(|_| "Cannot prepare the PDF operation.")?;
            serde_json::to_writer(config.as_file(), &job).map_err(|_| "Cannot prepare the PDF operation.")?;
            config.as_file().sync_all().map_err(|_| "Cannot finish preparing the PDF operation.")?;
            let mut argument = std::ffi::OsString::from("--job-json-file="); argument.push(config.path());
            let mut command = Command::new(&engine); command.arg(argument);
            // qpdf exit 3 means warnings, not a failed operation. Accept the candidate
            // only if its independent, strict structure/stream check below succeeds.
            let (code, _) = run(&mut command, Some(&cancel))?;
            if ![0, 3].contains(&code) {
                return Err("The PDF engine could not process this file. No output was saved.".into());
            }
            progress(&on_progress, format!("Checking output {} of {jobs}...", index + 1), true);
            let mut size = validate_output(&engine, output.path(), expected, &cancel)?;
            if page_operation {
                let layout: Vec<(usize,u32,u16)> = if request.operation == Operation::Split { plan[index].iter().map(|p| (0,*p,0)).collect() }
                    else if request.pages.is_empty() { documents.iter().enumerate().flat_map(|(i,doc)| (1..=doc.pages).map(move |p| (i,p,0))).collect() }
                    else { request.pages.iter().map(|p| (request.ids.iter().position(|id| id == &p.document_id).unwrap_or(0),p.page,p.rotation)).collect() };
                preserve_navigation(&engine, &output, &documents, &layout, workspace.path(), &cancel)?;
                size = validate_output(&engine, output.path(), expected, &cancel)?;
            }
            if request.operation == Operation::Compress && request.compression != Compression::Lossless {
                progress(&on_progress, format!("Compressing images in PDF {} of {jobs}...", index + 1), true);
                if let Some(target) = request.target_bytes {
                    let baseline = tempfile::Builder::new().suffix(".pdf").tempfile_in(workspace.path()).map_err(|_| "Cannot prepare compression.")?;
                    copy_into(output.path(), baseline.as_file(), Some(&cancel))?;
                    for tuning in [(80,300),(78,240),(75,200),(70,150)] {
                        if size <= target { break; }
                        let attempt = tempfile::Builder::new().suffix(".pdf").tempfile_in(workspace.path()).map_err(|_| "Cannot prepare compression.")?;
                        copy_into(baseline.path(), attempt.as_file(), Some(&cancel))?;
                        optimize_images(&engine, &attempt, workspace.path(), request.compression, expected, Some(tuning), &cancel)?;
                        let attempt_size = validate_output(&engine, attempt.path(), expected, &cancel)?;
                        if attempt_size < size { copy_into(attempt.path(), output.as_file(), Some(&cancel))?; size = attempt_size; }
                    }
                } else { optimize_images(&engine, &output, workspace.path(), request.compression, expected, None, &cancel)?; }
                size = validate_output(&engine, output.path(), expected, &cancel)?;
            }
            if request.operation == Operation::Compress && !documents[index].features.signatures && size >= fs::metadata(snapshots[index].path()).map_err(|_| "Working copy unavailable.")?.len() {
                copy_into(snapshots[index].path(), output.as_file(), Some(&cancel))?; size = fs::metadata(snapshots[index].path()).map_err(|_| "Working copy unavailable.")?.len();
            } else { recovered |= code == 3; }
            output.as_file().sync_all().map_err(|_| "Cannot finish writing the PDF.")?;
            output_bytes += size; sizes.push(size); outputs.push(output);
        }
        cancelled(Some(&cancel))?;
        if request.measure_only {
            let id = uuid::Uuid::new_v4().to_string();
            let digests = outputs.iter().map(|output| digest(output.path(), Some(&cancel))).collect::<Result<Vec<_>, _>>()?;
            cancelled(Some(&cancel))?;
            let measured = Saved { id: id.clone(), name: String::new(), files: jobs, input_bytes, output_bytes, file_bytes: sizes.clone(), recovered };
            *store.compression_preview.lock().map_err(|_| "Compression preview is unavailable.")? = Some(Arc::new(CompressionPreview {
                id, ids: request.ids, compression: request.compression, target_bytes: request.target_bytes, outputs, digests, sizes, recovered, _workspace: workspace,
            }));
            return Ok(Some(measured));
        }
        progress(&on_progress, "Saving verified output...", false);
        // After this point, a short publication step completes without cancellation.
        if multiple_outputs {
            let prefix = if request.operation == Operation::Split { "Split PDFs-" } else { "Compressed PDFs-" };
            let folder = tempfile::Builder::new().prefix(prefix).tempdir_in(parent).map_err(|_| "Cannot create an output folder.")?;
            for (index, output) in outputs.into_iter().enumerate() {
                let filename = if request.operation == Operation::Compress {
                    let stem: String = documents[index].path.file_stem().unwrap_or_default().to_string_lossy().chars().take(50).collect();
                    format!("{:03}-{stem}-compressed.pdf", index + 1)
                } else { let fallback = format!("part-{:03}",index+1); let label = request.output_names.get(index).map_or(fallback.clone(), |s| workspace::safe_name(s, &fallback)); format!("{:03}-{label}.pdf",index+1) };
                let path = folder.path().join(filename);
                let staged = tempfile::Builder::new().suffix(".pdf").tempfile_in(folder.path()).map_err(|_| "Cannot save PDF.")?;
                copy_into(output.path(), staged.as_file(), None)?;
                staged.persist_noclobber(&path).map_err(|_| "Cannot save PDFs. No output folder was retained.")?;
            }
            let path = folder.keep();
            Ok(Some(record_result(&store, path, jobs, input_bytes, output_bytes, recovered)?))
        } else {
            let output = outputs.pop().ok_or("No PDF was produced.")?;
            let staged = tempfile::Builder::new().prefix(".pdf-tools-save-").tempfile_in(parent).map_err(|_| "Cannot save PDF.")?;
            copy_into(output.path(), staged.as_file(), None)?;
            staged.persist_noclobber(&destination).map_err(|_| "Cannot save the PDF. Choose a new filename and a writable folder.")?;
            Ok(Some(record_result(&store, destination, 1, input_bytes, output_bytes, recovered)?))
        }
    }).await.map_err(|_| "PDF processing failed.")?
}
#[tauri::command]
async fn reveal_result(
    state: tauri::State<'_, Arc<Store>>,
    result_id: String,
) -> Result<(), String> {
    let path = state
        .results
        .lock()
        .map_err(|_| "Saved output is unavailable.")?
        .get(&result_id)
        .cloned()
        .ok_or("Saved output is no longer available in this session.")?;
    tauri::async_runtime::spawn_blocking(move || {
        if !path.exists() {
            return Err("The saved output was moved or removed.".into());
        }
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut value = Command::new("/usr/bin/open");
            if !path.is_dir() {
                value.arg("-R");
            }
            value.arg(&path);
            value
        };
        #[cfg(target_os = "windows")]
        let mut command = {
            let root = std::env::var_os("WINDIR").ok_or("Cannot locate Windows Explorer.")?;
            let mut value = Command::new(PathBuf::from(root).join("explorer.exe"));
            value.arg(if path.is_dir() {
                path.as_os_str().to_owned()
            } else {
                format!("/select,{}", path.display()).into()
            });
            value
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let mut command = {
            let mut value = Command::new("xdg-open");
            value.arg(if path.is_dir() {
                path.clone()
            } else {
                path.parent()
                    .ok_or("Cannot locate output folder.")?
                    .to_path_buf()
            });
            value
        };
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "Cannot open the saved output folder.")?
            .wait()
            .map_err(|_| "Cannot open the saved output folder.")?;
        Ok(())
    })
    .await
    .map_err(|_| "Cannot show the saved output.")?
}
#[tauri::command]
async fn open_release_notes(app: tauri::AppHandle) -> Result<(), String> {
    // Only the installed version's release page can be opened, never a caller-supplied URL.
    let url = format!("https://github.com/el-jsn/offline-pdf-tools/releases/tag/v{}", app.package_info().version);
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        let mut command = { let mut command = Command::new("/usr/bin/open"); command.arg(&url); command };
        #[cfg(target_os = "windows")]
        let mut command = { let mut command = Command::new("cmd.exe"); command.args(["/C", "start", "", &url]); command };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let mut command = { let mut command = Command::new("xdg-open"); command.arg(&url); command };
        let status = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .status().map_err(|_| "Cannot open release notes in your browser.")?;
        if !status.success() { return Err("Cannot open release notes in your browser.".into()); }
        Ok(())
    }).await.map_err(|_| "Cannot open release notes in your browser.")?
}

#[tauri::command]
fn set_view_title(window: tauri::Window, tool: String) -> Result<(), String> {
    let title = match tool.as_str() {
        "merge" => "Merge PDFs",
        "compress" => "Compress PDF",
        "split" => "Split PDF",
        "organize" => "Organize pages",
        _ => return Err("Unknown PDF tool.".into()),
    };
    window
        .set_title(title)
        .map_err(|_| "Cannot update the window title.".into())
}

#[tauri::command]
async fn confirm_app_update(version: String) -> Result<bool, String> {
    if version.len() > 64
        || !version
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || b".-+".contains(&value))
    {
        return Err("Invalid update version.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        rfd::MessageDialog::new()
            .set_title("Install update")
            .set_description(format!(
                "Install PDF Tools {version}? The app will restart and clear unsaved selections."
            ))
            .set_buttons(rfd::MessageButtons::OkCancel)
            .set_level(rfd::MessageLevel::Info)
            .show()
            == rfd::MessageDialogResult::Ok
    })
    .await
    .map_err(|_| "Could not confirm the update.".into())
}
#[derive(Serialize)]
struct UpdateInfo {
    id: String,
    version: String,
}
#[tauri::command]
async fn check_app_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Store>>,
) -> Result<Option<UpdateInfo>, String> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Network checks do not hold the PDF operation lock.
        *store
            .pending_update
            .lock()
            .map_err(|_| "Update status is unavailable.")? = None;
        let updater = app
            .updater_builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| "Updates are unavailable.")?;
        let update = tauri::async_runtime::block_on(updater.check()).map_err(|error| {
            match error {
                tauri_plugin_updater::Error::ReleaseNotFound => "No update release could be found.",
                tauri_plugin_updater::Error::TargetNotFound(_)
                | tauri_plugin_updater::Error::TargetsNotFound(_) => {
                    "No update has been published for this computer."
                }
                tauri_plugin_updater::Error::Reqwest(_) => {
                    "The update service could not be reached. Try again later."
                }
                _ => "The update release could not be read. Try again later.",
            }
            .to_string()
        })?;
        if let Some(mut update) = update {
            update.timeout = Some(Duration::from_secs(1200));
            let info = UpdateInfo {
                id: uuid::Uuid::new_v4().to_string(),
                version: update.version.clone(),
            };
            *store
                .pending_update
                .lock()
                .map_err(|_| "Update status is unavailable.")? = Some((info.id.clone(), update));
            Ok(Some(info))
        } else {
            Ok(None)
        }
    })
    .await
    .map_err(|_| "Update check failed.")?
}
#[tauri::command]
async fn install_app_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Store>>,
    update_id: String,
    on_progress: Channel<Progress>,
) -> Result<(), String> {
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = store
            .operation
            .try_lock()
            .map_err(|_| "Finish the current operation before installing an update.")?;
        let update = {
            let mut pending = store
                .pending_update
                .lock()
                .map_err(|_| "Update status is unavailable.")?;
            if pending.as_ref().is_none_or(|(id, _)| id != &update_id) {
                return Err("Check for updates again before installing.".into());
            }
            pending.take().ok_or("No update is available.")?.1
        };
        progress(&on_progress, "Downloading update...", false);
        let mut downloaded = 0u64;
        tauri::async_runtime::block_on(update.download_and_install(
            |chunk, total| {
                downloaded += chunk as u64;
                if let Some(total) = total.filter(|total| *total > 0) {
                    progress(
                        &on_progress,
                        format!(
                            "Downloading update: {}%",
                            (downloaded.saturating_mul(100) / total).min(100)
                        ),
                        false,
                    );
                }
            },
            || progress(&on_progress, "Verifying and installing update...", false),
        ))
        .map_err(|_| "Update could not be installed. Check for updates and try again.")?;
        progress(&on_progress, "Restarting...", false);
        app.restart();
    })
    .await
    .map_err(|_| "Update installation failed.")?
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Arc::new(Store { workspace: workspace::Workspace::new().expect("Cannot initialize private storage"),
            documents: Mutex::default(), dropped_paths: Mutex::default(), operation: Mutex::default(), job: Mutex::default(), results: Mutex::default(), compression_preview: Mutex::default(), pending_update: Mutex::default() }))
        // Tauri routes the main window's webview drops through WindowEvent.
        .on_window_event(|window, event| {
            #[cfg(target_os = "macos")]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" && window.hide().is_ok() {
                    api.prevent_close();
                }
            }
            if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                let state = window.state::<Arc<Store>>();
                if let Ok(mut allowed) = state.dropped_paths.lock() {
                    allowed.clear();
                    if paths.len() <= MAX_DOCUMENTS {
                        allowed.extend(paths.iter().cloned());
                    }
                };
            }
        })
        .invoke_handler(tauri::generate_handler![
            engine_status,
            choose_documents,
            import_dropped_documents,
            retain_documents,
            read_document_range,
            unlock_document,
            cancel_job,
            process_documents,
            discard_compression_preview,
            reveal_result,
            open_release_notes,
            set_view_title,
            check_app_update,
            install_app_update,
            confirm_app_update
        ])
        .build(tauri::generate_context!())
        .expect("Cannot start PDF Tools")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { has_visible_windows: false, .. } = &event {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            if matches!(event, tauri::RunEvent::Exit) {
                let state = app.state::<Arc<Store>>();
                if let Ok(mut docs) = state.documents.lock() { docs.clear(); }
                state.workspace.cleanup();
                if let Ok(mut preview) = state.compression_preview.lock() {
                    *preview = None;
                };
            }
        });
}
