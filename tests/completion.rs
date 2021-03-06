
extern crate gluon_language_server;
extern crate languageserver_types;

extern crate jsonrpc_core;
extern crate serde_json;
extern crate serde;
extern crate url;
#[macro_use]
extern crate pretty_assertions;

mod support;

use std::io::Write;

use url::Url;

use languageserver_types::{CompletionItem, CompletionItemKind, Position, TextDocumentIdentifier,
                           TextDocumentPositionParams};

use gluon_language_server::CompletionData;

fn completion<W: ?Sized>(stdin: &mut W, id: u64, uri: &str, position: Position)
where
    W: Write,
{
    let hover = support::method_call(
        "textDocument/completion",
        id,
        TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: support::test_url(uri) },
            position: position,
        },
    );

    support::write_message(stdin, hover).unwrap();
}

fn resolve<W: ?Sized>(stdin: &mut W, id: u64, item: &CompletionItem)
where
    W: Write,
{
    let hover = support::method_call("completionItem/resolve", id, item);

    support::write_message(stdin, hover).unwrap();
}

fn remove_completion_data(mut completions: Vec<CompletionItem>) -> Vec<CompletionItem> {
    for item in &mut completions {
        item.data.take();
    }
    completions
}

#[test]
fn local_completion() {
    let completions: Vec<CompletionItem> = support::send_rpc(|mut stdin| {
        let text = r#"
let test = 2
let test1 = ""
te
"#;
        support::did_open(stdin, "test", text);

        completion(
            stdin,
            1,
            "test",
            Position {
                line: 3,
                character: 2,
            },
        )
    });
    let completions = remove_completion_data(completions);
    assert_eq!(
        completions,
        vec![
            CompletionItem {
                label: "test".into(),
                kind: Some(CompletionItemKind::Variable),
                detail: Some("Int".into()),
                ..CompletionItem::default()
            },
            CompletionItem {
                label: "test1".into(),
                kind: Some(CompletionItemKind::Variable),
                detail: Some("String".into()),
                ..CompletionItem::default()
            },
        ]
    );
}

#[test]
fn prelude_completion() {
    let completions: Vec<CompletionItem> = support::send_rpc(|mut stdin| {
        support::did_open(stdin, "test", "no");

        completion(
            stdin,
            1,
            "test",
            Position {
                line: 0,
                character: 1,
            },
        )
    });
    let completions = remove_completion_data(completions);
    assert_eq!(
        completions,
        vec![
            CompletionItem {
                label: "not".into(),
                kind: Some(CompletionItemKind::Variable),
                detail: Some("std.types.Bool -> std.types.Bool".into()),
                ..CompletionItem::default()
            },
        ]
    );
}

#[test]
fn resolve_completion() {
    let completion = CompletionItem {
        label: "test".into(),
        kind: Some(CompletionItemKind::Variable),
        detail: Some("Int".into()),
        data: Some(
            serde_json::to_value(CompletionData {
                text_document_uri: support::test_url("test"),
                position: Position {
                    character: 2,
                    line: 4,
                },
            }).unwrap(),
        ),
        ..CompletionItem::default()
    };
    let actual: CompletionItem = support::send_rpc(|mut stdin| {
        let text = r#"
/// doc
let test = 2
let test1 = ""
te
"#;
        support::did_open(stdin, "test", text);

        resolve(stdin, 1, &completion);
    });
    assert_eq!(
        actual,
        CompletionItem {
            documentation: Some("doc".to_string()),
            ..completion
        }
    );
}

#[test]
fn url_encoded_path() {
    let completions: Vec<CompletionItem> = support::send_rpc(|mut stdin| {
        let text = r#"
let r = { abc = 1 }
r.
"#;
        let uri: Url = "file:///C%3A/examples/test.glu".parse().unwrap();
        support::did_open_uri(stdin, uri.clone(), text);

        let hover = support::method_call(
            "textDocument/completion",
            1,
            TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri },
                position: Position {
                    line: 3,
                    character: 2,
                },
            },
        );

        support::write_message(stdin, hover).unwrap();
    });
    let completions = remove_completion_data(completions);
    assert_eq!(
        completions,
        vec![
            CompletionItem {
                label: "abc".into(),
                kind: Some(CompletionItemKind::Variable),
                detail: Some("Int".into()),
                ..CompletionItem::default()
            },
        ]
    );
}
