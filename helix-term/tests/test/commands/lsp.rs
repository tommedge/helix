use super::*;
use std::io::Write;

const ARCHIVE_SERVER: &str = r#"
source=Dependency
if [ -f server-source ]; then source=$(cat server-source); fi
reply() {
    message="{\"jsonrpc\":\"2.0\",\"id\":$1,\"result\":$2}"
    printf 'Content-Length: %s\r\n\r\n%s' "${#message}" "$message"
}
location() {
    reply "$1" "{\"uri\":\"$2\",\"range\":{\"start\":{\"line\":0,\"character\":$3},\"end\":{\"line\":0,\"character\":$4}}}"
}
while IFS= read -r header; do
    length=$(printf '%s' "${header#Content-Length: }" | tr -d '\r')
    IFS= read -r blank
    body=$(dd bs=1 count="$length" 2>/dev/null)
    id=$(printf '%s' "$body" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
    case "$body" in
        *'"method":"initialize"'*)
            reply "$id" '{"capabilities":{"textDocumentSync":1,"definitionProvider":true,"referencesProvider":true,"hoverProvider":true,"executeCommandProvider":{"commands":["decompile"]}}}' ;;
        *'"method":"textDocument/definition"'*)
            case "$body" in
                *'"line":1'*) location "$id" 'jar:file:///library.jar!/Missing.java' 0 1 ;;
                *'$JAR'*) location "$id" '$JRT' 13 20 ;;
                *'$JRT'*) location "$id" '$PROJECT' 0 5 ;;
                *) location "$id" '$JAR' 13 23 ;;
            esac ;;
        *'"method":"workspace/executeCommand"'*)
            case "$body" in
                *'$JAR'*) reply "$id" "{\"code\":\"public class $source {}\\n\"}" ;;
                *'$JRT'*) reply "$id" '{"code":"public class Runtime {}\n"}' ;;
                *'Shown.java'*) reply "$id" '{"code":"public class Shown {}\n"}' ;;
                *) reply "$id" null ;;
            esac ;;
        *'"method":"textDocument/references"'*)
            reply "$id" '[{"uri":"$JAR","range":{"start":{"line":0,"character":13},"end":{"line":0,"character":23}}},{"uri":"$JRT","range":{"start":{"line":0,"character":13},"end":{"line":0,"character":20}}}]' ;;
        *'"method":"textDocument/hover"'*)
            reply "$id" null
            message='{"jsonrpc":"2.0","id":"show","method":"window/showDocument","params":{"uri":"jar:file:///library.jar!/Shown.java","takeFocus":true,"selection":{"start":{"line":0,"character":13},"end":{"line":0,"character":18}}}}'
            printf 'Content-Length: %s\r\n\r\n%s' "${#message}" "$message" ;;
        *'"method":"shutdown"'*) reply "$id" null ;;
        *'"method":"exit"'*) exit 0 ;;
    esac
done
"#;

#[tokio::test(flavor = "multi_thread")]
async fn archive_sources_preserve_identity_navigation_and_read_only_state() -> anyhow::Result<()> {
    let mut server = tempfile::NamedTempFile::new()?;
    let project = tempfile::Builder::new().suffix(".java").tempfile()?;
    std::fs::write(project.path(), "start\nmissing\n")?;
    let project_uri = helix_lsp::lsp::Url::from_file_path(project.path()).unwrap();
    let jar = "jar:file:///library.jar!/example/Dependency.java";
    let jrt = "jrt:/java.base/java/lang/Runtime.java";
    server.write_all(
        ARCHIVE_SERVER
            .replace("$PROJECT", project_uri.as_str())
            .replace("$JAR", jar)
            .replace("$JRT", jrt)
            .as_bytes(),
    )?;
    server.flush()?;
    let loader = test_syntax_loader(Some(format!(
        r#"
        [language-server.archive-test]
        command = "sh"
        args = [{:?}]
        [[language]]
        name = "java"
        language-servers = ["archive-test"]
    "#,
        server.path().to_str().unwrap()
    )));
    let mut config = test_config();
    config.editor.lsp.enable = true;
    let mut app = AppBuilder::new()
        .with_config(config)
        .with_lang_loader(loader)
        .with_file(project.path(), None)
        .build()?;
    run_event_loop_until_idle(&mut app).await;

    let check_source = |app: &Application, uri: &str, text: &str, symbol: &str| {
        let doc = helix_view::doc!(app.editor);
        assert_eq!(doc.url().unwrap().as_str(), uri);
        assert_eq!(doc.uri().unwrap().to_url().unwrap().as_str(), uri);
        assert!(doc.path().is_none());
        assert!(doc.readonly);
        assert_eq!(doc.text(), text);
        assert_eq!(doc.language_id(), Some("java"));
        assert_eq!(
            doc.language_servers()
                .map(|server| server.name())
                .collect::<Vec<_>>(),
            ["archive-test"]
        );
        let view = helix_view::view!(app.editor);
        assert_eq!(
            doc.selection(view.id)
                .primary()
                .fragment(doc.text().slice(..)),
            symbol
        );
    };
    test_key_sequences(
        &mut app,
        vec![
            (
                Some("gd"),
                Some(&|app| check_source(app, jar, "public class Dependency {}\n", "Dependency")),
            ),
            (
                Some("iCORRUPT<esc>"),
                Some(&|app| {
                    assert_eq!(
                        helix_view::doc!(app.editor).text(),
                        "public class Dependency {}\n"
                    );
                    assert!(!helix_view::doc!(app.editor).is_modified());
                }),
            ),
            (
                Some("gd"),
                Some(&|app| check_source(app, jrt, "public class Runtime {}\n", "Runtime")),
            ),
            (
                Some("<C-o>"),
                Some(&|app| check_source(app, jar, "public class Dependency {}\n", "Dependency")),
            ),
            (
                Some("<C-o>"),
                Some(&|app| {
                    assert_eq!(
                        helix_view::doc!(app.editor).url().as_ref(),
                        Some(&project_uri)
                    )
                }),
            ),
            (
                Some("<C-i><C-i>"),
                Some(&|app| check_source(app, jrt, "public class Runtime {}\n", "Runtime")),
            ),
            (
                Some("gd"),
                Some(&|app| {
                    assert_eq!(
                        helix_view::doc!(app.editor).url().as_ref(),
                        Some(&project_uri)
                    )
                }),
            ),
            (Some("gr"), None),
            (
                Some("Runtime<ret>"),
                Some(&|app| check_source(app, jrt, "public class Runtime {}\n", "Runtime")),
            ),
            (Some("gd"), None),
            (
                Some("jgd"),
                Some(&|app| {
                    assert_eq!(
                        helix_view::doc!(app.editor).url().as_ref(),
                        Some(&project_uri)
                    );
                    assert_eq!(app.editor.documents().count(), 3);
                    assert!(app
                        .editor
                        .get_status()
                        .unwrap()
                        .0
                        .contains("no archive source"));
                }),
            ),
            (
                Some("k<space>k"),
                Some(&|app| {
                    check_source(
                        app,
                        "jar:file:///library.jar!/Shown.java",
                        "public class Shown {}\n",
                        "Shown",
                    )
                }),
            ),
        ],
        false,
    )
    .await?;

    let export = project.path().with_extension("export.java");
    let id = app
        .editor
        .documents()
        .find(|doc| {
            doc.url()
                .is_some_and(|url| url.path().ends_with("Shown.java"))
        })
        .unwrap()
        .id();
    assert!(app.editor.save(id, Some(&export), true).is_err());
    assert!(app.editor.save(id, None::<&std::path::Path>, true).is_err());
    assert!(!export.exists());
    use helix_lsp::{lsp, OffsetEncoding};
    let edit: lsp::WorkspaceEdit = serde_json::from_value(serde_json::json!({
        "changes": {jrt: [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "CORRUPT"}]}
    }))?;
    assert!(app
        .editor
        .apply_workspace_edit(OffsetEncoding::Utf16, &edit)
        .is_err());
    for operation in [
        serde_json::json!({"kind": "create", "uri": jar}),
        serde_json::json!({"kind": "delete", "uri": jrt}),
        serde_json::json!({"kind": "rename", "oldUri": project_uri, "newUri": jar}),
        serde_json::json!({"kind": "rename", "oldUri": jar, "newUri": project_uri}),
    ] {
        let edit = serde_json::from_value(serde_json::json!({"documentChanges": [operation]}))?;
        assert!(app
            .editor
            .apply_workspace_edit(OffsetEncoding::Utf16, &edit)
            .is_err());
    }
    assert_eq!(app.editor.documents[&id].text(), "public class Shown {}\n");
    assert_eq!(std::fs::read_to_string(project.path())?, "start\nmissing\n");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn archive_sources_are_scoped_to_the_originating_project_server() -> anyhow::Result<()> {
    // LSP root discovery is restricted to the editor's workspace.
    let root = tempfile::tempdir_in(helix_stdx::env::current_working_dir())?;
    let first = root.path().join("first");
    let second = root.path().join("second");
    for (project, class) in [(&first, "FirstClass"), (&second, "OtherClass")] {
        std::fs::create_dir(project)?;
        std::fs::write(project.join("pom.xml"), "")?;
        std::fs::write(project.join("Main.java"), "start\n")?;
        std::fs::write(project.join("server-source"), class)?;
    }
    let mut server = tempfile::NamedTempFile::new()?;
    server.write_all(
        ARCHIVE_SERVER
            .replace(
                "$PROJECT",
                helix_lsp::lsp::Url::from_file_path(first.join("Main.java"))
                    .unwrap()
                    .as_str(),
            )
            .replace("$JAR", "jar:file:///shared.jar!/Dependency.java")
            .replace("$JRT", "jrt:/java.base/java/lang/Runtime.java")
            .as_bytes(),
    )?;
    server.flush()?;
    let loader = test_syntax_loader(Some(format!(
        r#"
        [language-server.archive-test]
        command = "sh"
        args = [{:?}]
        [[language]]
        name = "java"
        roots = ["pom.xml"]
        language-servers = ["archive-test"]
    "#,
        server.path().to_str().unwrap()
    )));
    let mut config = test_config();
    config.editor.lsp.enable = true;
    let mut app = AppBuilder::new()
        .with_config(config)
        .with_lang_loader(loader)
        .with_file(first.join("Main.java"), None)
        .build()?;
    run_event_loop_until_idle(&mut app).await;
    let open_second = format!(":open {}<ret>", second.join("Main.java").display());
    test_key_sequences(
        &mut app,
        vec![
            (
                Some("gd"),
                Some(&|app| {
                    assert_eq!(
                        helix_view::doc!(app.editor).text(),
                        "public class FirstClass {}\n"
                    )
                }),
            ),
            (Some(&open_second), None),
            (
                Some("gd"),
                Some(&|app| {
                    assert_eq!(
                        helix_view::doc!(app.editor).text(),
                        "public class OtherClass {}\n"
                    );
                    let sources = app
                        .editor
                        .documents()
                        .filter(|doc| doc.is_virtual())
                        .collect::<Vec<_>>();
                    assert_eq!(sources.len(), 2);
                    assert_eq!(sources[0].uri(), sources[1].uri());
                    assert_ne!(
                        sources[0].language_servers().next().unwrap().id(),
                        sources[1].language_servers().next().unwrap().id()
                    );
                }),
            ),
            (
                Some("<C-o><C-o>"),
                Some(&|app| {
                    assert_eq!(
                        helix_view::doc!(app.editor).text(),
                        "public class FirstClass {}\n"
                    )
                }),
            ),
        ],
        false,
    )
    .await
}
