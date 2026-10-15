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
fn values(index: &Value) -> Result<Vec<String>, String> {
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
