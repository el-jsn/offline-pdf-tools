//! Explicit release checks. Input paths come from the operator, never fixtures
//! committed from customer documents. These checks do not run by default.
use super::*;

fn engine_path() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engine/qpdf") }
fn check_root() -> PathBuf { PathBuf::from(std::env::var_os("PDF_TOOLS_CHECK_ROOT").expect("Set a private acceptance workspace")) }

#[test]
#[ignore = "Requires an explicitly supplied local PDF and private output directory"]
fn supplied_pdf_through_four_operations() {
    let source = PathBuf::from(std::env::var_os("PDF_TOOLS_CHECK_INPUT").expect("Set a local PDF path"));
    let root = check_root().join(format!("provided-case-{}", uuid::Uuid::new_v4())); fs::create_dir(&root).unwrap();
    let engine = engine_path(); let cancel = AtomicBool::new(false);
    let before = digest(&source, None).unwrap();
    let input = root.join("working.pdf"); fs::copy(&source, &input).unwrap();
    let document = inspect(&engine, &input, None).unwrap();
    assert!(!document.locked, "Supply an unlocked test copy");
    repair_snapshot_if_needed(&engine, &input, document.pages as usize, &cancel).unwrap();
    validate_output(&engine, &input, document.pages as usize, &cancel).unwrap();
    for operation in ["merge", "compress", "split", "organize"] {
        let output = tempfile::Builder::new().suffix(".pdf").tempfile_in(&root).unwrap();
        let mut job = serde_json::json!({"inputFile":input,"outputFile":output.path(),"objectStreams":"generate","removeUnreferencedResources":"yes"});
        let layout: Vec<_> = match operation {
            "merge" => (1..=document.pages).chain(std::iter::once(1)).map(|p|(0,p,0)).collect(),
            "split" => (1..=document.pages.min(3)).map(|p|(0,p,0)).collect(),
            "organize" => (1..=document.pages).rev().enumerate().map(|(i,p)|(0,p,if i==0 {90} else {0})).collect(),
            _ => (1..=document.pages).map(|p|(0,p,0)).collect(),
        };
        if operation == "compress" { job["compressStreams"]="y".into(); job["recompressFlate"]="".into(); job["compressionLevel"]="9".into(); }
        else {
            job["pages"] = layout.iter().map(|(_,p,_)|serde_json::json!({"file":input,"range":p.to_string()})).collect::<Vec<_>>().into();
            if operation == "organize" { job["rotate"]=serde_json::json!(["+90:1"]); }
        }
        let config = root.join("job.json"); fs::write(&config,serde_json::to_vec(&job).unwrap()).unwrap();
        checked(Command::new(&engine).arg(format!("--job-json-file={}",config.display())),Some(&cancel)).unwrap();
        validate_output(&engine,output.path(),layout.len(),&cancel).unwrap();
        if operation != "compress" { preserve_navigation(&engine,&output,std::slice::from_ref(&document),&layout,&root,&cancel).unwrap(); }
        validate_output(&engine,output.path(),layout.len(),&cancel).unwrap();
        if operation == "compress" {
            let balanced = tempfile::Builder::new().suffix(".pdf").tempfile_in(&root).unwrap();
            copy_into(output.path(),balanced.as_file(),Some(&cancel)).unwrap();
            optimize_images(&engine,&balanced,&root,Compression::Balanced,document.pages as usize,None,&cancel).unwrap();
            validate_output(&engine,balanced.path(),document.pages as usize,&cancel).unwrap();
            balanced.persist_noclobber(root.join("balanced.pdf")).unwrap();
        }
        output.persist_noclobber(root.join(format!("{operation}.pdf"))).unwrap();
    }
    assert!(digest(&source,None).unwrap()==before,"Original changed");
    fs::remove_file(root.join("job.json")).unwrap();
}

#[test]
#[ignore = "Requires the staged synthetic large scan"]
fn cancel_running_image_worker_and_reuse_engine() {
    let root=check_root(); let input=root.join("Large-scans.pdf"); let engine=engine_path();
    let before=digest(&input,None).unwrap(); let directory=tempfile::tempdir_in(&root).unwrap();
    let output=tempfile::Builder::new().suffix(".pdf").tempfile_in(directory.path()).unwrap();
    copy_into(&input,output.as_file(),None).unwrap();
    let cancel=Arc::new(AtomicBool::new(false)); let signal=cancel.clone(); let work=directory.path().to_owned();
    let trigger=thread::spawn(move || {
        let started=Instant::now();
        while started.elapsed()<Duration::from_secs(20) {
            let ready=fs::read_dir(&work).unwrap().flatten().any(|entry| entry.path().join("operation.json").exists());
            if ready { thread::sleep(Duration::from_millis(600)); signal.store(true,Ordering::Relaxed); return; }
            thread::sleep(Duration::from_millis(20));
        }
        signal.store(true,Ordering::Relaxed);
    });
    let start=Instant::now();
    let result=optimize_images(&engine,&output,directory.path(),Compression::Balanced,20,None,&cancel);
    trigger.join().unwrap(); assert!(result.unwrap_err().starts_with("Cancelled")); assert!(start.elapsed()<Duration::from_secs(15));
    assert!(digest(&input,None).unwrap()==before,"Original changed");
    assert_eq!(count_pages(&engine,&input,None).unwrap(),20);
    drop(output);drop(directory);
}

#[test]
#[ignore = "Requires a disposable mounted filesystem smaller than the synthetic scan"]
fn full_destination_removes_partial_staging_file() {
    let root=check_root(); let volume=root.join("test-volume");
    #[cfg(unix)] { use std::os::unix::fs::MetadataExt; assert_ne!(fs::metadata(&root).unwrap().dev(),fs::metadata(&volume).unwrap().dev(),"Refuse to fill the host filesystem"); }
    let output=tempfile::Builder::new().prefix("acceptance-").tempfile_in(&volume).unwrap();
    let staged_path=output.path().to_owned();
    let result=copy_into(&root.join("Large-scans.pdf"),output.as_file(),None);
    assert!(result.unwrap_err().contains("disk space")); drop(output);
    assert!(!staged_path.exists(),"Partial output survived");
}

#[test]
#[ignore = "Checks real no-clobber publication on synthetic files"]
fn existing_output_is_never_replaced() {
    let directory=tempfile::tempdir_in(check_root()).unwrap(); let target=directory.path().join("existing.pdf"); fs::write(&target,b"synthetic original").unwrap();
    let staged=tempfile::NamedTempFile::new_in(directory.path()).unwrap(); fs::write(staged.path(),b"synthetic new output").unwrap();
    assert!(staged.persist_noclobber(&target).is_err()); assert_eq!(fs::read(&target).unwrap(),b"synthetic original");
}

#[test]
fn replaced_destination_is_rejected() {
    let root=tempfile::tempdir().unwrap(); let selected=root.path().join("selected");
    fs::create_dir(&selected).unwrap();
    let destination=destination::Destination::capture(&selected).unwrap();
    destination.verify().unwrap();
    fs::rename(&selected,root.path().join("previous")).unwrap();
    fs::create_dir(&selected).unwrap();
    assert!(destination.verify().unwrap_err().contains("disconnected or changed"));
    assert_eq!(fs::read_dir(selected).unwrap().count(),0);
}

#[test]
#[ignore = "Requires detaching only the disposable test volume after ready appears"]
fn detached_destination_is_rejected() {
    let root=check_root(); let volume=root.join("test-volume");
    #[cfg(unix)] { use std::os::unix::fs::MetadataExt; assert_ne!(fs::metadata(&root).unwrap().dev(),fs::metadata(&volume).unwrap().dev()); }
    let selected=destination::Destination::capture(&volume).unwrap();
    fs::write(root.join("detach-ready"),b"ready").unwrap();
    let started=Instant::now();
    while !root.join("detach-done").exists() && started.elapsed()<Duration::from_secs(45) { thread::sleep(Duration::from_millis(50)); }
    assert!(root.join("detach-done").exists(),"Detach was not completed");
    assert!(selected.verify().unwrap_err().contains("disconnected or changed"));
    assert!(!volume.join("unexpected.pdf").exists());
}

#[test]
#[ignore = "Requires the disposable test volume mounted read-only"]
fn read_only_destination_rejects_staging() {
    let root=check_root(); let volume=root.join("test-volume");
    #[cfg(unix)] { use std::os::unix::fs::MetadataExt; assert_ne!(fs::metadata(&root).unwrap().dev(),fs::metadata(&volume).unwrap().dev()); }
    destination::Destination::capture(&volume).unwrap().verify().unwrap();
    assert!(tempfile::Builder::new().prefix("acceptance-").tempfile_in(&volume).is_err());
}

#[test]
#[ignore = "Copies a synthetic output larger than the per-input import limit"]
fn combined_output_can_exceed_input_limit() {
    let directory=tempfile::tempdir_in(check_root()).unwrap();
    let input=tempfile::NamedTempFile::new_in(directory.path()).unwrap();
    input.as_file().set_len(MAX_BYTES+65536).unwrap();
    let output=tempfile::NamedTempFile::new_in(directory.path()).unwrap();
    assert!(copy_into(input.path(),output.as_file(),None).is_err());
    copy_output(input.path(),output.as_file(),None).unwrap();
    assert_eq!(output.as_file().metadata().unwrap().len(),MAX_BYTES+65536);
}
