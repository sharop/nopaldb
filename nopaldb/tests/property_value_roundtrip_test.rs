// tests/property_value_roundtrip_test.rs
//
// A property value comes back as the same variant (#300). `Bytes` serializes
// as a sequence of integers, like a list, so until 0.9.3 three shapes came
// back changed, in the storage (MessagePack) and in the WAL (JSON):
// - a list of integers 0–255, or the empty list, came back as `Bytes`;
// - a list of `[text, value]` pairs came back as an `Object`;
// - the empty object came back as `Bytes`.
//
// Since 0.9.4 only those shapes are written wrapped; every other value is
// written as before, and what is already on disk reads as before.
//
// Fictional domain: seed packets.

use nopaldb::types::{Node, PropertyValue as V};
use nopaldb::Graph;

fn s(x: &str) -> V {
    V::String(x.to_string())
}

/// Values that used to come back as another variant, and neighbours that
/// never did (to check the boundary of the rule).
fn shapes() -> Vec<(&'static str, V)> {
    vec![
        ("small ints", V::List(vec![V::Int(1), V::Int(2)])),
        ("byte-range edge", V::List(vec![V::Int(0), V::Int(255)])),
        ("empty list", V::List(vec![])),
        ("pairs", V::List(vec![V::List(vec![s("seed"), V::Int(3)])])),
        ("empty object", V::Object(vec![])),
        ("nested small ints", V::List(vec![V::List(vec![V::Int(7)]), V::Int(1)])),
        ("object with a small-int list", V::Object(vec![("sizes".into(), V::List(vec![V::Int(4), V::Int(5)]))])),
        ("real bytes", V::Bytes(vec![1, 2, 3])),
        ("empty bytes", V::Bytes(vec![])),
        ("big int list", V::List(vec![V::Int(1), V::Int(300)])),
        ("negative int list", V::List(vec![V::Int(-1), V::Int(2)])),
        ("float list", V::List(vec![V::Float(1.0), V::Float(2.0)])),
        ("object", V::Object(vec![("name".into(), s("nopal")), ("n".into(), V::Int(2))])),
        ("list of a pair and more", V::List(vec![V::List(vec![s("a"), V::Int(1)]), V::Int(9)])),
    ]
}

#[test]
fn every_shape_round_trips_in_messagepack_and_json() {
    for (name, value) in shapes() {
        let packed = rmp_serde::to_vec(&value).unwrap();
        let back: V = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(back, value, "MessagePack: {name}");

        let json = serde_json::to_string(&value).unwrap();
        let back: V = serde_json::from_str(&json).unwrap();
        assert_eq!(back, value, "JSON: {name} ({json})");
    }
}

#[test]
fn values_that_were_never_ambiguous_keep_their_bytes() {
    // The pre-0.9.4 encoding of each value is the encoding of its payload.
    let cases: Vec<(V, Vec<u8>)> = vec![
        (V::Bytes(vec![1, 2, 3]), rmp_serde::to_vec(&vec![1u8, 2, 3]).unwrap()),
        (V::List(vec![V::Int(1), V::Int(300)]), rmp_serde::to_vec(&vec![1i64, 300]).unwrap()),
        (V::Object(vec![("n".into(), V::Int(2))]), rmp_serde::to_vec(&vec![("n".to_string(), 2i64)]).unwrap()),
        (s("nopal"), rmp_serde::to_vec("nopal").unwrap()),
        (V::Int(-5), rmp_serde::to_vec(&-5i64).unwrap()),
        (V::Null, rmp_serde::to_vec(&()).unwrap()),
    ];
    for (value, legacy) in cases {
        assert_eq!(rmp_serde::to_vec(&value).unwrap(), legacy, "{value:?}");
    }
}

#[test]
fn what_is_already_on_disk_reads_as_before() {
    // Written by ≤ 0.9.3: plain sequences. They keep reading as they did,
    // including the ambiguous ones (a stored `Bytes` cannot be told apart).
    let read = |bytes: Vec<u8>| -> V { rmp_serde::from_slice(&bytes).unwrap() };
    assert_eq!(read(rmp_serde::to_vec(&vec![1u8, 2]).unwrap()), V::Bytes(vec![1, 2]));
    assert_eq!(read(rmp_serde::to_vec(&Vec::<u8>::new()).unwrap()), V::Bytes(vec![]));
    assert_eq!(read(rmp_serde::to_vec(&vec![1i64, 300]).unwrap()), V::List(vec![V::Int(1), V::Int(300)]));
    assert_eq!(
        read(rmp_serde::to_vec(&vec![("seed".to_string(), 3i64)]).unwrap()),
        V::Object(vec![("seed".into(), V::Int(3))])
    );
}

#[test]
fn a_malformed_wrapper_is_an_error() {
    for json in [r#"{"$list": [1], "$object": []}"#, r#"{}"#, r#"{"$other": 1}"#] {
        assert!(serde_json::from_str::<V>(json).is_err(), "{json}");
    }
}

#[tokio::test]
async fn every_shape_survives_closing_and_reopening_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let mut ids = Vec::new();
    {
        let graph = Graph::open(dir.path()).await.unwrap();
        for (name, value) in shapes() {
            let id = graph.add_node(Node::new("Packet").with_property("v", value)).await.unwrap();
            ids.push((name, id));
        }
        graph.close().await.unwrap();
    }
    let graph = Graph::open(dir.path()).await.unwrap();
    for ((name, id), (_, value)) in ids.into_iter().zip(shapes()) {
        let node = graph.get_node(id).await.unwrap();
        assert_eq!(node.properties.get("v"), Some(&value), "{name}");
    }
}
