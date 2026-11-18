use esh_editor::Document;
use esh_lang::compiler::Source;
use serde_json::Value;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{Element, HtmlTextAreaElement};

struct Listener {
    node: Element,
    event: &'static str,
    callback: Closure<dyn FnMut(web_sys::Event)>,
}
impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.node.remove_event_listener_with_callback(
            self.event,
            self.callback.as_ref().unchecked_ref(),
        );
    }
}
struct App {
    document: Document,
    active: usize,
    tabs: Vec<Listener>,
    review: Vec<Listener>,
    permanent: Vec<Listener>,
}
thread_local! { static ROOT: RefCell<Option<Shared>> = const { RefCell::new(None) }; }
type Shared = Rc<RefCell<App>>;
fn dom() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}
fn element(id: &str) -> Element {
    dom().get_element_by_id(id).unwrap()
}
fn text(parent: &Element, tag: &str, class: &str, value: &str) -> Element {
    let node = dom().create_element(tag).unwrap();
    node.set_class_name(class);
    node.set_text_content(Some(value));
    parent.append_child(&node).unwrap();
    node
}
fn clear(node: &Element) {
    while let Some(child) = node.first_child() {
        node.remove_child(&child).unwrap();
    }
}
fn string<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("unknown")
}
fn listen(
    node: &Element,
    event: &'static str,
    f: impl FnMut(web_sys::Event) + 'static,
) -> Listener {
    let callback = Closure::<dyn FnMut(web_sys::Event)>::new(f);
    node.add_event_listener_with_callback(event, callback.as_ref().unchecked_ref())
        .unwrap();
    Listener {
        node: node.clone(),
        event,
        callback,
    }
}
fn source_tabs(app: &Shared) {
    app.borrow_mut().tabs.clear();
    let parent = element("sources");
    clear(&parent);
    let sources = app.borrow().document.sources().to_vec();
    for (index, source) in sources.iter().enumerate() {
        let button = text(
            &parent,
            "button",
            if app.borrow().active == index {
                "source active"
            } else {
                "source"
            },
            &source.name,
        );
        button
            .set_attribute(
                "title",
                if index == 0 {
                    "Editable entry file"
                } else {
                    "Imported module; read-only snapshot"
                },
            )
            .unwrap();
        let weak = Rc::downgrade(app);
        let listener = listen(&button, "click", move |_| {
            if let Some(app) = weak.upgrade() {
                app.borrow_mut().active = index;
                show_source(&app);
                source_tabs(&app);
            }
        });
        app.borrow_mut().tabs.push(listener);
    }
}
fn show_source(app: &Shared) {
    let app = app.borrow();
    let source = &app.document.sources()[app.active];
    let input: HtmlTextAreaElement = element("code").dyn_into().unwrap();
    input.set_value(&source.text);
    input.set_read_only(app.active != 0);
    element("filename").set_text_content(Some(&source.name));
    element("source-state").set_text_content(Some(if app.active == 0 {
        "Editable buffer · explicit evaluation order"
    } else {
        "Imported module · read-only snapshot"
    }));
    let lines = (1..=source.text.split('\n').count())
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    element("line-numbers").set_text_content(Some(&lines));
}
fn jump(app: &Shared, location: &Value) {
    let source = location["source"].as_u64().map(|n| n as usize).or_else(|| {
        location["source"].as_str().and_then(|n| {
            app.borrow()
                .document
                .sources()
                .iter()
                .position(|s| s.name == n)
        })
    });
    if let Some(index) = source {
        if index >= app.borrow().document.sources().len() {
            return;
        }
        app.borrow_mut().active = index;
        show_source(app);
        source_tabs(app);
        let text = app.borrow().document.sources()[index].text.clone();
        let start = location["span"]["start"].as_u64().unwrap_or(0) as usize;
        let end = location["span"]["end"].as_u64().unwrap_or(start as u64) as usize;
        if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            return;
        }
        let a = text[..start].encode_utf16().count() as u32;
        let b = text[..end].encode_utf16().count() as u32;
        let input: HtmlTextAreaElement = element("code").dyn_into().unwrap();
        let _ = input.focus();
        let _ = input.set_selection_range(a, b);
    }
}
fn section(parent: &Element, title: &str) -> Element {
    let s = text(parent, "section", "section", "");
    text(&s, "h2", "", title);
    s
}
fn render(app: &Shared) {
    app.borrow_mut().review.clear();
    let report = app.borrow().document.current_review().cloned();
    let parent = element("review");
    clear(&parent);
    let Some(report) = report else {
        element("status").set_text_content(Some("Needs review"));
        text(
            &parent,
            "p",
            "notice",
            "The buffer changed. Check it again before relying on these results.",
        );
        return;
    };
    if report["ok"] != true {
        element("status").set_text_content(Some("Check failed"));
        let s = section(&parent, "Diagnostic");
        text(&s, "p", "error", string(&report["diagnostic"], "message"));
        text(&s, "code", "", string(&report["diagnostic"], "code"));
        return;
    }
    element("status").set_text_content(Some("Static checks passed · runtime checks pending"));
    let s = section(&parent, "File impact");
    text(
        &s,
        "p",
        "muted",
        "In execution order. Repeated calls remain separate; only reached branches execute.",
    );
    if report["fileImpact"]["complete"] != true {
        text(
            &s,
            "p",
            "notice",
            "This projection is incomplete. Additional targets or operations remain unknown.",
        );
    }
    if let Some(rows) = report["fileImpact"]["impacts"].as_array() {
        if rows.is_empty() {
            text(
                &s,
                "p",
                "muted",
                "No filesystem operations were found in this projection.",
            );
        }
        for row in rows {
            let operation = string(row, "operation");
            let mutation = operation.contains("write")
                || operation.contains("remove")
                || operation.contains("mkdir");
            let card = text(
                &s,
                "article",
                if mutation {
                    "impact mutation"
                } else {
                    "impact"
                },
                "",
            );
            text(
                &card,
                "div",
                "eyebrow",
                &format!("{}  {}", row["order"], operation),
            );
            let target = format!(
                "{}:/{}",
                row["region"].as_str().unwrap_or("?"),
                row["path"].as_str().unwrap_or("<runtime path>")
            );
            let button = text(&card, "button", "target", &target);
            let location = row["location"].clone();
            let weak = Rc::downgrade(app);
            let listener = listen(&button, "click", move |_| {
                if let Some(app) = weak.upgrade() {
                    jump(&app, &location);
                }
            });
            app.borrow_mut().review.push(listener);
            text(&card, "p", "", string(row, "behavior"));
            for key in ["conditions", "unknowns"] {
                if let Some(items) = row[key].as_array() {
                    for item in items {
                        text(
                            &card,
                            "p",
                            if key == "unknowns" {
                                "notice"
                            } else {
                                "condition"
                            },
                            item.as_str().unwrap_or("Unknown condition"),
                        );
                    }
                }
            }
            if string(row, "repetition").starts_with("E*") {
                text(&card, "p", "condition", string(row, "repetition"));
            }
        }
    }
    let s = section(&parent, "Update contract");
    text(&s,"p","muted","Unspecified state is unknown. Mandatory checks must finish before mutation; rechecks do not prove isolation.");
    for (key, label) in [
        ("requires", "Before any changes"),
        ("preserves", "Protected throughout execution"),
        ("ensures", "Before reporting success"),
    ] {
        text(&s, "h3", "", label);
        if let Some(items) = report["contracts"][key].as_array() {
            if items.is_empty() {
                text(&s, "p", "muted", "None declared.");
            }
            for item in items {
                let card = text(&s, "article", "contract-card", "");
                let target = if key == "preserves" {
                    item
                } else {
                    &item["target"]
                };
                text(
                    &card,
                    "code",
                    "signature",
                    &format!("{}:/{}", string(target, "region"), string(target, "path")),
                );
                let predicate = &item["predicate"];
                let description = if key == "preserves" {
                    "Forbid this program’s changes to the file or entire directory subtree, including through alias roots."
                } else {
                    match string(predicate, "kind") {
                        "regular-file" => "Must be a regular file.",
                        "directory" => "Must be a directory.",
                        "absent" => "Must not exist.",
                        "exact-text" => {
                            "Contents must exactly match this UTF-8 text (JSON escaped):"
                        }
                        "content-hash" => "Contents must match this SHA-256 digest:",
                        _ => "Unknown predicate.",
                    }
                };
                text(&card, "p", "condition", description);
                if let Some(expected) = predicate.get("expected") {
                    text(
                        &card,
                        "pre",
                        "contract",
                        &if string(predicate, "kind") == "exact-text" {
                            expected.to_string()
                        } else {
                            expected.as_str().unwrap_or("unknown").to_owned()
                        },
                    );
                }
            }
        }
    }
    if let Some(state) = report["suppliedState"].as_str() {
        let details = text(&s, "details", "", "");
        text(
            &details,
            "summary",
            "",
            "Supplied initial-state specification (also required)",
        );
        text(&details, "pre", "contract", state);
    }
    text(&s,"p","notice","Guarantees assume no conflicting external changes to the specified targets during execution. Rechecks detect some drift; completed changes are not rolled back on failure.");
    let s = section(&parent, "Type & authority");
    text(&s, "code", "signature", string(&report, "type"));
    if let Some(effects) = report["effects"].as_array() {
        for e in effects {
            text(
                &s,
                "span",
                "chip",
                &format!("{} {}", string(e, "operation"), string(e, "region")),
            );
        }
    }
    if let Some(caps) = report["capabilities"].as_array() {
        for cap in caps {
            text(
                &s,
                "p",
                "condition",
                &format!(
                    "{} · requested rights {}{}",
                    string(cap, "name"),
                    cap["permissions"],
                    if cap["infer"] == true {
                        " · inference candidate"
                    } else {
                        ""
                    }
                ),
            );
        }
    }
    if let Some(nodes) = report["nodes"].as_array() {
        for n in nodes {
            let details = &n["details"];
            if details["narrowed"] == true {
                text(
                    &s,
                    "p",
                    "condition",
                    &format!(
                        "Authority narrowed: {} → {}",
                        details["granted"], details["required"]
                    ),
                );
            }
            if let Some(captures) = details["captures"].as_array() {
                for capture in captures {
                    if capture["authority"] == true {
                        text(
                            &s,
                            "p",
                            "condition",
                            &format!(
                                "Capture {}{} · {}",
                                string(capture, "name"),
                                if capture["inferred"] == true {
                                    " (inferred)"
                                } else {
                                    ""
                                },
                                capture
                                    .get("required")
                                    .unwrap_or(&capture["retainedAuthority"])
                            ),
                        );
                    }
                }
            }
        }
    }
    let definitions = report["nodes"]
        .as_array()
        .map(|nodes| {
            nodes
                .iter()
                .filter(|node| node["kind"] == "definition")
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let aliases = report["aliases"].as_array();
    if !definitions.is_empty() || aliases.is_some_and(|xs| !xs.is_empty()) {
        let s = section(&parent, "Functions & aliases");
        for node in definitions {
            let card = text(&s, "article", "contract-card", "");
            let label = format!(
                "{}{}",
                string(&node["details"], "name"),
                if node["details"]["exported"] == true {
                    " · exported"
                } else {
                    " · private"
                }
            );
            let button = text(&card, "button", "target", &label);
            let location = node.clone();
            let weak = Rc::downgrade(app);
            app.borrow_mut()
                .review
                .push(listen(&button, "click", move |_| {
                    if let Some(app) = weak.upgrade() {
                        jump(&app, &location);
                    }
                }));
            text(&card, "code", "signature", string(node, "type"));
            if let Some(qs) = node["details"]["quantifiers"].as_array() {
                if !qs.is_empty() {
                    text(
                        &card,
                        "p",
                        "condition",
                        &format!(
                            "forall {}",
                            qs.iter()
                                .map(|q| format!("{}: {}", string(q, "name"), string(q, "kind")))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    );
                }
            }
        }
        if let Some(aliases) = aliases {
            for alias in aliases {
                text(
                    &s,
                    "p",
                    "condition",
                    &format!("type {} = {}", string(alias, "name"), string(alias, "type")),
                );
            }
        }
    }
    let s = section(&parent, "Proof scope");
    text(&s,"p","muted","Rust policy decisions are extracted through Aeneas into Lean. Compiler soundness, the complete interpreter, and rendering are not yet proved. OS behavior and absence of external interference remain explicit assumptions.");
    let s = section(&parent, "Ordered structure");
    let details = text(&s, "details", "", "");
    text(&details, "summary", "", "Inspect checked structure");
    text(
        &details,
        "pre",
        "structure",
        &serde_json::to_string_pretty(&report["structure"]).unwrap_or_default(),
    );
}
fn mount(sources: Vec<Source>, state: Option<String>) -> Result<(), JsValue> {
    let document = Document::with_initial_state(sources, state).map_err(JsValue::from_str)?;
    let app = Rc::new(RefCell::new(App {
        document,
        active: 0,
        tabs: vec![],
        review: vec![],
        permanent: vec![],
    }));
    source_tabs(&app);
    show_source(&app);
    app.borrow_mut().document.analyze();
    render(&app);
    let input: HtmlTextAreaElement = element("code").dyn_into()?;
    {
        let weak = Rc::downgrade(&app);
        let input = input.clone();
        let listener = listen(&element("code"), "input", move |_| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            if app.borrow().active != 0 {
                return;
            }
            let result = app.borrow_mut().document.edit_entry(input.value());
            if let Err(error) = result {
                element("status").set_text_content(Some(error));
                return;
            }
            render(&app);
            let lines = (1..=input.value().split('\n').count())
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            element("line-numbers").set_text_content(Some(&lines));
        });
        app.borrow_mut().permanent.push(listener);
    }
    {
        let input = input.clone();
        let listener = listen(&element("code"), "scroll", move |_| {
            element("line-numbers").set_scroll_top(input.scroll_top());
        });
        app.borrow_mut().permanent.push(listener);
    }
    {
        let weak = Rc::downgrade(&app);
        let listener = listen(&element("check"), "click", move |_| {
            if let Some(app) = weak.upgrade() {
                app.borrow_mut().document.analyze();
                render(&app);
            }
        });
        app.borrow_mut().permanent.push(listener);
    }
    {
        let weak = Rc::downgrade(&app);
        let listener = listen(&element("download"), "click", move |_| {
            if let Some(app) = weak.upgrade() {
                let source = app.borrow().document.sources()[0].clone();
                let encoded = js_sys::encode_uri_component(&source.text);
                let anchor: web_sys::HtmlAnchorElement =
                    dom().create_element("a").unwrap().dyn_into().unwrap();
                anchor.set_href(&format!("data:text/plain;charset=utf-8,{encoded}"));
                anchor.set_download(source.name.rsplit('/').next().unwrap_or("script.esh"));
                anchor.click();
            }
        });
        app.borrow_mut().permanent.push(listener);
    }
    ROOT.with(|root| *root.borrow_mut() = Some(app));
    Ok(())
}
#[wasm_bindgen(start)]
pub fn start() {
    spawn_local(async {
        let result = async {
            let response =
                JsFuture::from(web_sys::window().unwrap().fetch_with_str("/api/project")).await?;
            let response: web_sys::Response = response.dyn_into()?;
            if !response.ok() {
                return Err(JsValue::from_str("Could not load the project snapshot."));
            }
            let body = JsFuture::from(response.text()?)
                .await?
                .as_string()
                .ok_or_else(|| JsValue::from_str("Invalid project response."))?;
            let project: Value = serde_json::from_str(&body)
                .map_err(|_| JsValue::from_str("Invalid project snapshot."))?;
            let sources: Vec<Source> = serde_json::from_value(project["sources"].clone())
                .map_err(|_| JsValue::from_str("Invalid project source table."))?;
            mount(sources, project["initialState"].as_str().map(str::to_owned))
        }
        .await;
        if let Err(error) = result {
            element("status").set_text_content(Some("Editor could not start"));
            element("review").set_text_content(Some(
                &error
                    .as_string()
                    .unwrap_or_else(|| "Unknown browser error.".into()),
            ));
        }
    });
}
