//! Refresh ordinary AcroForm appearances on private copies before flattening.
use super::*;
use serde_json::{json, Map, Value};
const FORM_ERROR: &str = "This form's saved answers could not be prepared. Export a flattened PDF from its form editor, then add that copy.";
fn objects(index: &Value) -> Result<&Map<String, Value>, String> {
    index
        .pointer("/qpdf/1")
        .and_then(Value::as_object)
        .ok_or_else(|| FORM_ERROR.into())
}
fn resolve<'a>(all: &'a Map<String, Value>, value: &'a Value) -> &'a Value {
    value
        .as_str()
        .and_then(|id| all.get(&format!("obj:{id}")))
        .and_then(|object| object.get("value"))
        .unwrap_or(value)
}
pub(super) fn values(index: &Value) -> Result<Vec<String>, String> {
    let fields = index
        .pointer("/acroform/fields")
        .and_then(Value::as_array)
        .ok_or(FORM_ERROR)?;
    let mut values: Vec<_> = fields
        .iter()
        .map(|field| {
            json!([
                field["fullname"],
                field["fieldtype"],
                field["value"],
                field["pageposfrom1"]
            ])
            .to_string()
        })
        .collect();
    values.sort();
    Ok(values)
}
const RECONNECT_ERROR: &str = "This form has disconnected controls with conflicting or ambiguous data. No output was saved.";

// Some editors duplicate a merged field/widget dictionary, leaving /Fields
// pointing to one copy and the page's /Annots pointing to the other. Reconnect
// only byte-for-byte equivalent JSON dictionaries; never choose between values.
fn duplicate_widget_patch(index: &Value) -> Result<Option<(Value, usize)>, String> {
    let all = objects(index)?;
    let root = all.get("trailer").and_then(|v| v.pointer("/value/~1Root")).ok_or(RECONNECT_ERROR)?;
    let catalog = resolve(all, root);
    let form = resolve(all, &catalog["/AcroForm"]);
    let Some(fields) = resolve(all, &form["/Fields"]).as_array() else { return Ok(None); };
    if fields.is_empty() || form.get("/XFA").is_some() { return Ok(None); }
    let inspected_fields = index.pointer("/acroform/fields").and_then(Value::as_array).ok_or(RECONNECT_ERROR)?;
    if has_signatures(inspected_fields, catalog, all) { return Ok(None); }
    let mut canonical = HashSet::new();
    let mut equivalents: HashMap<String, Option<String>> = HashMap::new();
    for reference in fields {
        let Some(id) = reference.as_str() else { return Ok(None); };
        let field = resolve(all, reference);
        // Nested, inherited and shared field structures require a different repair.
        if field["/Subtype"] != "/Widget" || field.get("/Parent").is_some() || field.get("/Kids").is_some() {
            return Ok(None);
        }
        if !canonical.insert(id.to_owned()) { return Err(RECONNECT_ERROR.into()); }
        equivalents.entry(field.to_string()).and_modify(|value| *value=None).or_insert(Some(id.to_owned()));
    }
    let pages = index["pages"].as_array().ok_or(RECONNECT_ERROR)?;
    let mut used = HashSet::new();let mut changes = Map::new();let mut count = 0;
    for page in pages {
        let reference = page["object"].as_str().ok_or(RECONNECT_ERROR)?;
        let mut dictionary = resolve(all, &page["object"]).clone();
        let Some(annotations) = resolve(all, &dictionary["/Annots"]).as_array() else { continue; };
        let mut annotations = annotations.clone();let mut changed = false;
        for annotation in &mut annotations {
            let widget = resolve(all, annotation);
            if widget["/Subtype"] != "/Widget" { continue; }
            let id = annotation.as_str().ok_or(RECONNECT_ERROR)?;
            let target = if canonical.contains(id) { id.to_owned() } else {
                equivalents.get(&widget.to_string()).and_then(Clone::clone).ok_or(RECONNECT_ERROR)?
            };
            // One field cannot stand in for two independently placed widgets.
            if !used.insert(target.clone()) { return Err(RECONNECT_ERROR.into()); }
            if target != id { *annotation=json!(target);count+=1;changed=true; }
        }
        if changed {
            dictionary["/Annots"] = json!(annotations);
            changes.insert(format!("obj:{reference}"), json!({"value":dictionary}));
        }
    }
    if count==0 { return Ok(None); }
    Ok(Some((json!({"qpdf":[index["qpdf"][0],changes]}),count)))
}
fn answers(index: &Value) -> Result<Vec<String>, String> {
    let fields=index.pointer("/acroform/fields").and_then(Value::as_array).ok_or(RECONNECT_ERROR)?;
    let mut answers:Vec<_>=fields.iter().map(|field|json!([field["fullname"],field["fieldtype"],field["value"]]).to_string()).collect();
    answers.sort();Ok(answers)
}
pub(super) fn reconnect_duplicate_widgets(engine: &Path, input: &Path, cancel: &AtomicBool) -> Result<Option<tempfile::NamedTempFile>, String> {
    let before=inspect_json(engine,input,Some(cancel))?;
    let Some((patch,count))=duplicate_widget_patch(&before)? else {return Ok(None);};
    diagnostics::stage("Reconnect duplicate form widgets");
    let workspace=input.parent().ok_or(RECONNECT_ERROR)?;
    let config=tempfile::NamedTempFile::new_in(workspace).map_err(|_|RECONNECT_ERROR)?;
    serde_json::to_writer(config.as_file(),&patch).map_err(|_|RECONNECT_ERROR)?;
    let output=tempfile::Builder::new().suffix(".pdf").tempfile_in(workspace).map_err(|_|RECONNECT_ERROR)?;
    let mut argument=std::ffi::OsString::from("--update-from-json=");argument.push(config.path());
    let (code,_)=run(Command::new(engine).arg(argument).args(["--recompress-flate","--compress-streams=y","--object-streams=generate"]).arg(input).arg(output.path()),Some(cancel))?;
    if ![0,3].contains(&code) {return Err(RECONNECT_ERROR.into());}
    let after=inspect_json(engine,output.path(),Some(cancel))?;
    if answers(&before)?!=answers(&after)? {return Err(RECONNECT_ERROR.into());}
    check_field_tree(&after)?;
    diagnostics::reconnected_widgets(count);
    Ok(Some(output))
}
fn check_field_tree(index: &Value) -> Result<(), String> {
    let all = objects(index)?;
    let fields = index
        .pointer("/acroform/fields")
        .and_then(Value::as_array)
        .ok_or(FORM_ERROR)?;
    let mut widgets = HashSet::new();
    for field in fields {
        let id = field
            .pointer("/annotation/object")
            .and_then(Value::as_str)
            .ok_or(FORM_ERROR)?;
        let key = format!("obj:{id}");
        let annotation = &all.get(&key).ok_or(FORM_ERROR)?["value"];
        if annotation
            .get("/V")
            .is_some_and(|value| value != &field["value"])
        {
            return Err(FORM_ERROR.into());
        }
        widgets.insert(key);
    }
    if all.iter().any(|(id, object)| {
        object.pointer("/value/~1Subtype") == Some(&json!("/Widget")) && !widgets.contains(id)
    }) {
        return Err(FORM_ERROR.into());
    }
    Ok(())
}

// Called only after the existing qpdf repair cannot produce a clean copy.
// The isolated worker rejects nested/XFA/signed/conflicting forms, verifies
// decoded page content, resources and widget appearances, and never logs data.
pub(super) fn pypdf_fallback(engine: &Path, input: &Path, expected: usize, cancel: &AtomicBool) -> Result<tempfile::NamedTempFile, String> {
    const ERROR: &str = "This PDF could not be repaired safely. No output was saved.";
    let before = inspect_json(engine, input, Some(cancel))?;
    let features = inspect_features(engine, input, Some(cancel))?;
    if features.signatures || features.unsupported_forms { return Err(ERROR.into()); }
    let worker = engine.parent().ok_or(ERROR)?.join("pypdf-repair")
        .join(if cfg!(windows) { "pypdf-repair.exe" } else { "pypdf-repair" });
    if !worker.is_file() { return Err(ERROR.into()); }
    diagnostics::stage("Repair form with pypdf 6.20.1");
    let workspace = tempfile::tempdir_in(input.parent().ok_or(ERROR)?).map_err(|_| ERROR)?;
    let path = workspace.path().join("repaired.pdf");
    checked(Command::new(worker).arg(input).arg(&path), Some(cancel))
        .map_err(|error| if error.starts_with("Cancelled") { error } else { ERROR.into() })?;
    validate_output(engine, &path, expected, cancel)?;
    let after = inspect_json(engine, &path, Some(cancel))?;
    check_field_tree(&after)?;
    compression::retained_non_form_data(&before, &after)?;
    let result = tempfile::Builder::new().suffix(".pdf").tempfile_in(input.parent().ok_or(ERROR)?)
        .map_err(|_| ERROR)?;
    fs::copy(&path, result.path()).map_err(|_| ERROR)?;
    Ok(result)
}
fn appearance_patch(index: &Value) -> Result<Value, String> {
    check_field_tree(index)?;
    let all = objects(index)?;
    let fields = index
        .pointer("/acroform/fields")
        .and_then(Value::as_array)
        .ok_or(FORM_ERROR)?;
    let mut changes = Map::new();
    let mut widgets = HashSet::new();
    for field in fields {
        let id = field
            .pointer("/annotation/object")
            .and_then(Value::as_str)
            .ok_or(FORM_ERROR)?;
        let key = format!("obj:{id}");
        let mut annotation = all
            .get(&key)
            .and_then(|object| object.get("value"))
            .cloned()
            .ok_or(FORM_ERROR)?;
        widgets.insert(key.clone());
        if annotation
            .get("/V")
            .is_some_and(|value| value != &field["value"])
        {
            return Err(FORM_ERROR.into());
        }
        // Empty signatures have no appearance; removal is separately authorized.
        if field["fieldtype"] == "/Sig" {
            continue;
        }
        let normal = resolve(all, &annotation["/AP"])["/N"].clone();
        if field["ischeckbox"] == true || field["isradiobutton"] == true {
            let states = resolve(all, &normal).as_object().ok_or(FORM_ERROR)?;
            let value = field["value"].as_str().unwrap_or("/Off");
            let state = if states.contains_key(value) {
                value
            } else if states.contains_key("/Off") {
                "/Off"
            } else {
                return Err(FORM_ERROR.into());
            };
            if value != "/Off"
                && field["isradiobutton"] == true
                && !fields.iter().any(|other| {
                    if other["fullname"] != field["fullname"] {
                        return false;
                    }
                    let Some(id) = other.pointer("/annotation/object").and_then(Value::as_str)
                    else {
                        return false;
                    };
                    let candidate = &all[&format!("obj:{id}")]["value"];
                    let normal = &resolve(all, &candidate["/AP"])["/N"];
                    resolve(all, normal).get(value).is_some()
                })
            {
                return Err(FORM_ERROR.into());
            }
            let stream = states[state]
                .as_str()
                .and_then(|id| all.get(&format!("obj:{id}")))
                .and_then(|object| object.get("stream"));
            if stream.is_none() {
                return Err(FORM_ERROR.into());
            }
            // A radio widget may represent another option in the same group.
            if field["ischeckbox"] == true && value != "/Off" && state == "/Off" {
                return Err(FORM_ERROR.into());
            }
            annotation["/AS"] = json!(state);
            changes.insert(key, json!({"value":annotation}));
        } else {
            // PDFium must have produced a real appearance stream, including for
            // empty controls. An arbitrary flag or empty dictionary is insufficient.
            let stream = normal
                .as_str()
                .and_then(|id| all.get(&format!("obj:{id}")))
                .and_then(|object| object.get("stream"));
            if stream.is_none() {
                return Err(FORM_ERROR.into());
            }
        }
    }
    // Do not discard orphan widgets or an ambiguous field tree.
    if all.iter().any(|(id, object)| {
        object.pointer("/value/~1Subtype") == Some(&json!("/Widget")) && !widgets.contains(id)
    }) {
        return Err(FORM_ERROR.into());
    }
    let root = all
        .get("trailer")
        .and_then(|t| t.pointer("/value/~1Root"))
        .and_then(Value::as_str)
        .ok_or(FORM_ERROR)?;
    let catalog_key = format!("obj:{root}");
    let catalog = &all[&catalog_key]["value"];
    if let Some(form_id) = catalog["/AcroForm"].as_str() {
        let key = format!("obj:{form_id}");
        let mut form = all
            .get(&key)
            .and_then(|v| v.get("value"))
            .cloned()
            .ok_or(FORM_ERROR)?;
        form["/NeedAppearances"] = json!(false);
        changes.insert(key, json!({"value":form}));
    } else if catalog["/AcroForm"].is_object() {
        let mut catalog = catalog.clone();
        catalog["/AcroForm"]["/NeedAppearances"] = json!(false);
        changes.insert(catalog_key, json!({"value":catalog}));
    } else {
        return Err(FORM_ERROR.into());
    }
    Ok(json!({"qpdf":[index["qpdf"][0],changes]}))
}
pub fn prepare(
    engine: &Path,
    input: &Path,
    workspace: &Path,
    cancel: &AtomicBool,
) -> Result<tempfile::NamedTempFile, String> {
    diagnostics::stage("Prepare saved form answers");
    let before = inspect_json(engine, input, Some(cancel))?;
    check_field_tree(&before)?;
    let (worker, library) = image_runtime(engine)?;
    let directory = tempfile::Builder::new()
        .prefix("forms-")
        .tempdir_in(workspace)
        .map_err(|_| FORM_ERROR)?;
    let appearances = directory.path().join("appearances.pdf");
    checked(
        Command::new(worker)
            .arg("--prepare-forms")
            .arg(library)
            .arg(input)
            .arg(&appearances),
        Some(cancel),
    )
    .map_err(|error| {
        if error.starts_with("Cancelled") {
            error
        } else {
            FORM_ERROR.into()
        }
    })?;
    let after = inspect_json(engine, &appearances, Some(cancel))?;
    if values(&before)? != values(&after)? {
        return Err(FORM_ERROR.into());
    }
    let patch = directory.path().join("appearances.json");
    fs::write(
        &patch,
        serde_json::to_vec(&appearance_patch(&after)?).map_err(|_| FORM_ERROR)?,
    )
    .map_err(|_| FORM_ERROR)?;
    let output = tempfile::Builder::new()
        .suffix(".pdf")
        .tempfile_in(workspace)
        .map_err(|_| FORM_ERROR)?;
    let mut update = std::ffi::OsString::from("--update-from-json=");
    update.push(patch);
    checked(
        Command::new(engine)
            .arg(update)
            .arg(&appearances)
            .arg(output.path()),
        Some(cancel),
    )?;
    Ok(output)
}
pub fn validate_flattened(engine: &Path, path: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let index = inspect_json(engine, path, Some(cancel))?;
    if objects(&index)?.values().any(|object| {
        object.pointer("/value/~1Subtype") == Some(&json!("/Widget"))
            || object.pointer("/value/~1AcroForm").is_some()
    }) {
        return Err(FORM_ERROR.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn index() -> Value {
        json!({"acroform":{"fields":[{"fullname":"test","fieldtype":"/Tx","value":"u:Dummy","pageposfrom1":1,"annotation":{"object":"3 0 R"}}]},"qpdf":[{"jsonversion":2},{
            "trailer":{"value":{"/Root":"1 0 R"}},"obj:1 0 R":{"value":{"/AcroForm":"2 0 R"}},
            "obj:2 0 R":{"value":{"/NeedAppearances":true}},"obj:3 0 R":{"value":{"/Subtype":"/Widget","/V":"u:Dummy","/AP":{"/N":"4 0 R"}}},"obj:4 0 R":{"stream":{"dict":{}}}
        }]})
    }
    fn duplicate_index() -> Value {
        let mut input=index();
        input["qpdf"][1]["obj:2 0 R"]["value"]["/Fields"]=json!(["3 0 R"]);
        input["qpdf"][1]["obj:3 0 R"]["value"]["/FT"]=json!("/Tx");
        input["qpdf"][1]["obj:3 0 R"]["value"]["/T"]=json!("u:test");
        input["qpdf"][1]["obj:5 0 R"]=input["qpdf"][1]["obj:3 0 R"].clone();
        input["qpdf"][1]["obj:6 0 R"]=json!({"value":{"/Type":"/Page","/Annots":["5 0 R"]}});
        input["pages"]=json!([{"object":"6 0 R"}]);input
    }
    #[test]
    fn duplicate_widget_repair_changes_only_page_references() {
        let input=duplicate_index();let original=input.clone();
        let (patch,count)=duplicate_widget_patch(&input).unwrap().unwrap();
        assert_eq!(count,1);
        assert_eq!(patch["qpdf"][1].as_object().unwrap().len(),1);
        let mut expected=input["qpdf"][1]["obj:6 0 R"].clone();
        expected["value"]["/Annots"]=json!(["3 0 R"]);
        assert_eq!(patch["qpdf"][1]["obj:6 0 R"],expected);
        assert_eq!(input,original);
    }
    #[test]
    fn duplicate_widget_repair_rejects_disagreements_and_ambiguous_matches() {
        for key in ["/V","/AP","/AS","/Rect","/FT"] {
            let mut input=duplicate_index();
            input["qpdf"][1]["obj:5 0 R"]["value"][key]=json!("u:Conflict");
            assert!(duplicate_widget_patch(&input).is_err(),"{key}");
        }
        let mut input=duplicate_index();
        input["qpdf"][1]["obj:7 0 R"]=input["qpdf"][1]["obj:3 0 R"].clone();
        input["qpdf"][1]["obj:2 0 R"]["value"]["/Fields"]=json!(["3 0 R","7 0 R"]);
        assert!(duplicate_widget_patch(&input).is_err());
        let mut input=duplicate_index();
        input["qpdf"][1]["obj:6 0 R"]["value"]["/Annots"]=json!(["3 0 R","5 0 R"]);
        assert!(duplicate_widget_patch(&input).is_err());
    }
    #[test]
    fn duplicate_widget_repair_leaves_clean_nested_and_signed_forms_alone() {
        let mut input=duplicate_index();
        input["qpdf"][1]["obj:6 0 R"]["value"]["/Annots"]=json!(["3 0 R"]);
        assert!(duplicate_widget_patch(&input).unwrap().is_none());
        let mut input=duplicate_index();
        input["qpdf"][1]["obj:3 0 R"]["value"]["/Kids"]=json!(["5 0 R"]);
        assert!(duplicate_widget_patch(&input).unwrap().is_none());
        let mut input=duplicate_index();
        input["acroform"]["fields"][0]["fieldtype"]=json!("/Sig");
        input["acroform"]["fields"][0]["value"]=json!("8 0 R");
        assert!(duplicate_widget_patch(&input).unwrap().is_none());
    }
    #[test]
    #[ignore = "Requires generated missing-field-tree fixtures; no document data is printed"]
    fn supplied_pypdf_missing_fields() {
        let source=PathBuf::from(std::env::var_os("PDF_TOOLS_PYPDF_INPUT").unwrap());
        let expected=PathBuf::from(std::env::var_os("PDF_TOOLS_PYPDF_EXPECTED").unwrap());
        let original=digest(&source,None).unwrap();
        let directory=tempfile::tempdir().unwrap();
        let engine=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engine/qpdf");
        let cancel=AtomicBool::new(false);
        let snapshot=directory.path().join("working.pdf");fs::copy(&source,&snapshot).unwrap();
        let pages=count_pages(&engine,&snapshot,None).unwrap();
        assert!(repair_snapshot_if_needed(&engine,&snapshot,pages as usize,&cancel).unwrap());
        let after=inspect_json(&engine,&snapshot,None).unwrap();
        let expected=inspect_json(&engine,&expected,None).unwrap();
        assert!(answers(&expected).unwrap()==answers(&after).unwrap(),"Saved answers changed");
        check_field_tree(&after).unwrap();
        validate_output(&engine,&snapshot,pages as usize,&cancel).unwrap();
        assert!(digest(&source,None).unwrap()==original,"Original changed");
        assert!(pypdf_fallback(&engine,&source,pages as usize+1,&cancel).is_err());
        assert!(pypdf_fallback(&engine,&source,pages as usize,&AtomicBool::new(true)).unwrap_err().starts_with("Cancelled"));
    }
    #[test]
    #[ignore = "Requires an explicitly supplied local duplicate-widget form; never prints document contents"]
    fn supplied_duplicate_widget_repair_and_compression() {
        let source=PathBuf::from(std::env::var_os("PDF_TOOLS_FORM_REPAIR_INPUT").expect("Supply a local form"));
        let original=digest(&source,None).unwrap();let directory=tempfile::tempdir().unwrap();
        let engine=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engine/qpdf");let cancel=AtomicBool::new(false);
        let snapshot=directory.path().join("working.pdf");fs::copy(&source,&snapshot).unwrap();
        let before=inspect_json(&engine,&snapshot,None).unwrap();
        let pages=count_pages(&engine,&snapshot,None).unwrap();
        assert!(repair_snapshot_if_needed(&engine,&snapshot,pages as usize,&cancel).unwrap());
        validate_output(&engine,&snapshot,pages as usize,&cancel).unwrap();
        let after=inspect_json(&engine,&snapshot,None).unwrap();
        assert!(answers(&before).unwrap()==answers(&after).unwrap(),"Saved answers changed");
        check_field_tree(&after).unwrap();
        let export=std::env::var_os("PDF_TOOLS_SYNTHETIC_REPAIR_OUTPUT").map(PathBuf::from);
        if let Some(root)=&export {
            let file=tempfile::NamedTempFile::new_in(root).unwrap();copy_into(&snapshot,file.as_file(),None).unwrap();
            file.persist_noclobber(root.join("repaired.pdf")).unwrap();
        }
        for preset in [Compression::Balanced,Compression::Small] {
            let output=tempfile::NamedTempFile::new_in(directory.path()).unwrap();
            copy_into(&snapshot,output.as_file(),None).unwrap();
            compression::optimize(&engine,&output,directory.path(),preset,pages as usize,None,&cancel).unwrap();
            validate_output(&engine,output.path(),pages as usize,&cancel).unwrap();
            let compressed=inspect_json(&engine,output.path(),None).unwrap();
            assert!(answers(&after).unwrap()==answers(&compressed).unwrap(),"Saved answers changed");
            check_field_tree(&compressed).unwrap();
            if let Some(root)=&export {
                let name=if matches!(preset,Compression::Balanced){"balanced.pdf"}else{"smaller.pdf"};
                output.persist_noclobber(root.join(name)).unwrap();
            }
        }
        assert_eq!(original,digest(&source,None).unwrap());
    }
    #[test]
    fn generated_appearances_are_required_before_clearing_flag() {
        let mut input = index();
        let patch = appearance_patch(&input).unwrap();
        assert_eq!(
            patch["qpdf"][1]["obj:2 0 R"]["value"]["/NeedAppearances"],
            false
        );
        input["qpdf"][1]
            .as_object_mut()
            .unwrap()
            .remove("obj:4 0 R");
        assert!(appearance_patch(&input).is_err());
    }
    #[test]
    fn conflicting_values_and_orphan_widgets_are_rejected() {
        let mut input = index();
        input["qpdf"][1]["obj:3 0 R"]["value"]["/V"] = json!("u:Conflicting dummy");
        assert!(appearance_patch(&input).is_err());
        let mut input = index();
        input["qpdf"][1]["obj:5 0 R"] = json!({"value":{"/Subtype":"/Widget"}});
        assert!(appearance_patch(&input).is_err());
    }
    #[test]
    fn checkbox_state_uses_saved_value_and_requires_existing_appearance() {
        let mut input = index();
        input["acroform"]["fields"][0]["fieldtype"] = json!("/Btn");
        input["acroform"]["fields"][0]["ischeckbox"] = json!(true);
        input["acroform"]["fields"][0]["value"] = json!("/Selected");
        input["qpdf"][1]["obj:3 0 R"]["value"] = json!({"/Subtype":"/Widget","/V":"/Selected","/AS":"/Off","/AP":{"/N":{"/Selected":"4 0 R","/Off":"4 0 R"}}});
        assert_eq!(
            appearance_patch(&input).unwrap()["qpdf"][1]["obj:3 0 R"]["value"]["/AS"],
            "/Selected"
        );
        input["acroform"]["fields"][0]["value"] = json!("/Missing");
        input["qpdf"][1]["obj:3 0 R"]["value"]["/V"] = json!("/Missing");
        assert!(appearance_patch(&input).is_err());
    }
    // Explicit local fixture check. Documents and paths stay outside the repository.
    #[test]
    #[ignore]
    fn local_filled_form_preparation() {
        let directory = PathBuf::from(
            std::env::var_os("PDF_TOOLS_FORM_CHECK").expect("Set a private fixture directory"),
        );
        let engine = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engine/qpdf");
        let input = directory.join("dummy-filled.pdf");
        let before = digest(&input, None).unwrap();
        let cancel = AtomicBool::new(false);
        let prepared = prepare(&engine, &input, &directory, &cancel).unwrap();
        let output = directory.join("verified-flat.pdf");
        assert!(!output.exists());
        checked(
            Command::new(&engine)
                .args(["--flatten-annotations=all", "--remove-acroform"])
                .arg(prepared.path())
                .arg(&output),
            Some(&cancel),
        )
        .unwrap();
        validate_output(
            &engine,
            &output,
            count_pages(&engine, &input, None).unwrap() as usize,
            &cancel,
        )
        .unwrap();
        validate_flattened(&engine, &output, &cancel).unwrap();
        assert_eq!(before, digest(&input, None).unwrap());
    }
}
