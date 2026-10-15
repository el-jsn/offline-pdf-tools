//! Run the bundled compressor in a cancellable process, on private copies only.
use super::*;

pub(super) fn tunings(mode: Compression) -> &'static [(u8, u32)] {
    match mode {
        Compression::Balanced => &[(80, 300), (75, 240), (70, 200), (60, 150)],
        Compression::Small => &[
            (80, 300),
            (75, 240),
            (70, 200),
            (60, 150),
            (60, 120),
            (60, 96),
            (60, 72),
        ],
        Compression::Lossless => &[],
    }
}

fn runtime(engine: &Path) -> Result<PathBuf, String> {
    let executable = engine
        .parent()
        .ok_or("Compression engine unavailable.")?
        .join(if cfg!(windows) {
            "compress-pdf.exe"
        } else {
            "compress-pdf"
        });
    if !executable.is_file() {
        return Err(
            "The compression engine is missing. Install a complete copy of PDF Tools.".into(),
        );
    }
    Ok(executable)
}

fn command(
    executable: &Path,
    input: &Path,
    output: &Path,
    mode: Compression,
    tuning: Option<(u8, u32)>,
) -> Result<Command, String> {
    let preset = match mode {
        Compression::Balanced => "--standard",
        Compression::Small => "--more",
        Compression::Lossless => {
            return Err("Original quality does not use image compression.".into())
        }
    };
    let mut command = Command::new(executable);
    // Keep document metadata, tags, output intents and embedded standard fonts.
    // The CLI's document-derived report is drained in memory, never logged.
    command
        .arg(input)
        .arg("--output")
        .arg(output)
        .arg(preset)
        .args([
            "--strip",
            "none",
            "--unembed-standard-fonts",
            "false",
            "--verify",
            "structural",
        ])
        .env("RUST_LOG", "off");
    if let Some((quality, dpi)) = tuning {
        command
            .arg("--quality")
            .arg(quality.to_string())
            .arg("--dpi")
            .arg(dpi.to_string())
            .arg("--threshold-dpi")
            .arg(dpi.to_string());
    }
    Ok(command)
}

fn retained_document_data(
    before: &serde_json::Value,
    after: &serde_json::Value,
) -> Result<(), String> {
    if forms::values(before)? != forms::values(after)? {
        return Err(
            "Compression could not preserve the saved form answers. No output was saved.".into(),
        );
    }
    let attachment_names = |index: &serde_json::Value| -> Vec<String> {
        index["attachments"]
            .as_object()
            .map(|entries| entries.keys().cloned().collect())
            .unwrap_or_default()
    };
    if attachment_names(before) != attachment_names(after) {
        return Err("Compression could not preserve attachments. No output was saved.".into());
    }
    if bookmarks(&before["outlines"]) != bookmarks(&after["outlines"]) {
        return Err("Compression could not preserve bookmarks. No output was saved.".into());
    }
    let catalog = |index: &serde_json::Value| -> Option<serde_json::Value> {
        let all = index.pointer("/qpdf/1")?.as_object()?;
        let root = all.get("trailer")?.pointer("/value/~1Root")?.as_str()?;
        all.get(&format!("obj:{root}"))?.get("value").cloned()
    };
    let before_catalog = catalog(before).ok_or("Cannot inspect the original document catalog.")?;
    let after_catalog = catalog(after).ok_or("Cannot inspect the compressed document catalog.")?;
    for key in ["/StructTreeRoot", "/Metadata", "/OutputIntents"] {
        if before_catalog.get(key).is_some() && after_catalog.get(key).is_none() {
            return Err("Compression could not preserve document metadata or structure. No output was saved.".into());
        }
    }
    Ok(())
}

pub(super) fn optimize(
    engine: &Path,
    output: &tempfile::NamedTempFile,
    workspace: &Path,
    mode: Compression,
    pages: usize,
    tuning: Option<(u8, u32)>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    cancelled(Some(cancel))?;
    diagnostics::stage("Compress document");
    let executable = runtime(engine)?;
    diagnostics::compression_engine();
    let directory = tempfile::Builder::new()
        .prefix("compression-")
        .tempdir_in(workspace)
        .map_err(|_| "Cannot prepare compression.")?;
    let input = directory.path().join("input.pdf");
    let candidate = directory.path().join("compressed.pdf");
    // The candidate path is fresh. Never let the compressor overwrite a source.
    fs::copy(output.path(), &input).map_err(|_| "Cannot prepare compression.")?;
    let before = inspect_json(engine, &input, Some(cancel))?;
    checked(
        &mut command(&executable, &input, &candidate, mode, tuning)?,
        Some(cancel),
    )
    .map_err(|error| {
        if error.starts_with("Cancelled") {
            error
        } else {
            "Compression could not finish safely. No output was saved.".into()
        }
    })?;
    let size = validate_output(engine, &candidate, pages, cancel)?;
    let after = inspect_json(engine, &candidate, Some(cancel))?;
    retained_document_data(&before, &after)?;
    if size
        < output
            .as_file()
            .metadata()
            .map_err(|_| "Cannot inspect output size.")?
            .len()
    {
        copy_into(&candidate, output.as_file(), Some(cancel))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn form_and_navigation_regressions_are_rejected() {
        let before = serde_json::json!({"acroform":{"fields":[{"fullname":"synthetic","fieldtype":"/Tx","value":"u:Saved","pageposfrom1":1}]},"attachments":{"synthetic.txt":{}},"outlines":[{"title":"Synthetic","destpageposfrom1":1,"kids":[]}],"qpdf":[{}, {"trailer":{"value":{"/Root":"1 0 R"}},"obj:1 0 R":{"value":{"/StructTreeRoot":"2 0 R"}}}]});
        assert!(retained_document_data(&before, &before).is_ok());
        for path in ["/acroform/fields/0/value", "/outlines/0/title"] {
            let mut after = before.clone();
            *after.pointer_mut(path).unwrap() = serde_json::json!("changed");
            assert!(retained_document_data(&before, &after).is_err());
        }
        let mut after = before.clone();
        after["attachments"] = serde_json::json!({});
        assert!(retained_document_data(&before, &after).is_err());
        let mut after = before.clone();
        after["qpdf"][1]["obj:1 0 R"]["value"] = serde_json::json!({});
        assert!(retained_document_data(&before, &after).is_err());
    }

    #[test]
    #[ignore = "Requires the explicitly generated synthetic compression fixture"]
    fn synthetic_document_through_both_presets() {
        let fixtures = PathBuf::from(
            std::env::var_os("PDF_TOOLS_COMPRESSION_FIXTURE")
                .expect("Generate the synthetic fixture first"),
        );
        let source = fixtures.join("synthetic.pdf");
        let engine = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engine/qpdf");
        let before = digest(&source, None).unwrap();
        let document = inspect(&engine, &source, None).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        for (preset, tuning) in [
            (Compression::Balanced, None),
            (Compression::Small, None),
            (Compression::Balanced, Some((60, 150))),
            (Compression::Small, Some((60, 72))),
        ] {
            let output = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
            copy_into(&source, output.as_file(), None).unwrap();
            optimize(
                &engine,
                &output,
                directory.path(),
                preset,
                document.pages as usize,
                tuning,
                &cancel,
            )
            .unwrap();
            assert!(
                validate_output(&engine, output.path(), document.pages as usize, &cancel).unwrap()
                    < document.bytes
            );
            let features = inspect_features(&engine, output.path(), None).unwrap();
            assert!(features.forms && features.tags && features.attachments == 1);
            assert!(features.bookmarks == document.features.bookmarks);
            // A cancelled attempt must not replace an existing private result.
            let saved = digest(output.path(), None).unwrap();
            let stopped = AtomicBool::new(true);
            assert!(optimize(
                &engine,
                &output,
                directory.path(),
                preset,
                document.pages as usize,
                None,
                &stopped
            )
            .unwrap_err()
            .starts_with("Cancelled"));
            assert_eq!(digest(output.path(), None).unwrap(), saved);
        }
        let output = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        copy_into(&source, output.as_file(), None).unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let signal = stopped.clone();
        let trigger = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            signal.store(true, Ordering::Relaxed);
        });
        let result = optimize(
            &engine,
            &output,
            directory.path(),
            Compression::Balanced,
            document.pages as usize,
            None,
            &stopped,
        );
        trigger.join().unwrap();
        assert!(result.unwrap_err().starts_with("Cancelled"));
        assert_eq!(digest(output.path(), None).unwrap(), before);
        assert_eq!(count_pages(&engine, &source, None).unwrap(), document.pages);
        assert_eq!(digest(&source, None).unwrap(), before);
    }
}
