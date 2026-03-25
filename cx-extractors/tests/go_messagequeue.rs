use cx_core::graph::edges::EdgeKind;
use cx_core::graph::nodes::NodeKind;
use cx_core::graph::string_interner::StringInterner;
use cx_extractors::grammars::{self, Language};
use cx_extractors::universal::{ExtractionResult, ParsedFile};

fn extract_go(source: &str, path: &str) -> (ExtractionResult, StringInterner) {
    let lang = Language::Go;
    let ts_lang = lang.ts_language();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&ts_lang).unwrap();
    let tree = parser.parse(source.as_bytes(), None).unwrap();
    let extractor = grammars::extractor_for_language(lang).unwrap();
    let mut strings = StringInterner::new();
    let path_id = strings.intern(path);
    let file = ParsedFile {
        tree,
        source: source.as_bytes(),
        path: path_id,
        path_str: path,
        repo_id: 0,
    };
    let mut id = 0u32;
    let result = extractor.extract(&file, &mut strings, &mut id);
    (result, strings)
}

#[test]
fn go_nats_publish() {
    let source = r#"package main

import "github.com/nats-io/nats.go"

func publishEvent() {
    nc.Publish("events.user.created", data)
}
"#;
    let (result, strings) = extract_go(source, "publisher.go");

    let topics: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8 && n.sub_kind == 3)
        .map(|n| strings.get(n.name))
        .collect();

    assert!(
        topics.contains(&"events.user.created"),
        "should detect NATS publish topic, got: {:?}",
        topics
    );

    assert!(
        result.edges.iter().any(|e| e.kind == EdgeKind::Publishes),
        "should have Publishes edge"
    );
}

#[test]
fn go_nats_subscribe() {
    let source = r#"package main

import "github.com/nats-io/nats.go"

func subscribeEvents() {
    nc.Subscribe("events.user.created", handleEvent)
}
"#;
    let (result, strings) = extract_go(source, "subscriber.go");

    let topics: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8 && n.sub_kind == 3)
        .map(|n| strings.get(n.name))
        .collect();

    assert!(
        topics.contains(&"events.user.created"),
        "should detect NATS subscribe topic, got: {:?}",
        topics
    );

    assert!(
        result.edges.iter().any(|e| e.kind == EdgeKind::Subscribes),
        "should have Subscribes edge"
    );
}

#[test]
fn go_kafka_publish_subscribe() {
    let source = r#"package main

func produceMessages() {
    producer.SendMessage("orders", payload)
}

func consumeMessages() {
    consumer.Consume("orders")
}
"#;
    let (result, strings) = extract_go(source, "kafka.go");

    let topics: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8 && n.sub_kind == 3)
        .map(|n| strings.get(n.name))
        .collect();

    assert!(
        topics.contains(&"orders"),
        "should detect Kafka topic, got: {:?}",
        topics
    );

    assert!(
        result.edges.iter().any(|e| e.kind == EdgeKind::Publishes),
        "should have Publishes edge for producer"
    );
    assert!(
        result.edges.iter().any(|e| e.kind == EdgeKind::Subscribes),
        "should have Subscribes edge for consumer"
    );
}
