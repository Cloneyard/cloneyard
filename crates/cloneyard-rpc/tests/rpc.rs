use std::collections::HashMap;

use cloneyard_api::Method;
use cloneyard_rpc::{Client, ClientError, Error, Router, serve};
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf,
    duplex, split,
};

struct Sum;

impl Method for Sum {
    const NAME: &'static str = "test.sum";
    const SUMMARY: &'static str = "Returns the sum of two numbers";
    type Params = (i64, i64);
    type Result = i64;
}

fn router() -> Router {
    Router::new()
        .handle::<Sum>(|(a, b)| async move { Ok(a + b) })
        .method("add", |(a, b): (i64, i64)| async move { Ok(a + b) })
        .method("echo", |params: Value| async move { Ok(params) })
        .method("ping", |(): ()| async { Ok("pong") })
        .method("unserializable", |(): ()| async {
            Ok(HashMap::from([((1, 2), 3)]))
        })
        .method("fail", |(): ()| async {
            Err::<(), _>(Error::new(-32000, "boom"))
        })
}

async fn exchange(input: impl AsRef<[u8]>) -> Vec<Value> {
    let (client, server) = duplex(64 * 1024);
    let serving = tokio::spawn(async move {
        let (reader, writer) = split(server);
        serve(&router(), reader, writer).await
    });
    let (mut reader, mut writer) = split(client);
    let sending = async {
        writer.write_all(input.as_ref()).await.unwrap();
        writer.shutdown().await.unwrap();
    };
    let mut output = String::new();
    let receiving = reader.read_to_string(&mut output);
    let ((), received) = tokio::join!(sending, receiving);
    received.unwrap();
    serving.await.unwrap().unwrap();
    output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn connect_to_router() -> Client<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>> {
    let (client, server) = duplex(1024);
    tokio::spawn(async move {
        let (reader, writer) = split(server);
        serve(&router(), reader, writer).await
    });
    let (reader, writer) = split(client);
    Client::new(reader, writer)
}

async fn code_and_id_of(input: &str) -> (i64, Value) {
    let responses = exchange(format!("{input}\n")).await;
    assert_eq!(responses.len(), 1, "{responses:?}");
    let response = &responses[0];
    assert_eq!(response["jsonrpc"], "2.0");
    assert!(response.get("result").is_none(), "{response}");
    (
        response["error"]["code"].as_i64().unwrap(),
        response["id"].clone(),
    )
}

#[tokio::test]
async fn answers_a_request_with_its_result() {
    let responses =
        exchange("{\"jsonrpc\":\"2.0\",\"method\":\"add\",\"params\":[2,3],\"id\":1}\n").await;
    assert_eq!(responses, [json!({"jsonrpc": "2.0", "result": 5, "id": 1})]);
}

#[tokio::test]
async fn echoes_string_and_number_ids() {
    let responses = exchange(concat!(
        "{\"jsonrpc\":\"2.0\",\"method\":\"ping\",\"id\":\"a\"}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"ping\",\"id\":1.5}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"ping\",\"id\":18446744073709551615}\n",
    ))
    .await;
    let ids: Vec<&Value> = responses.iter().map(|response| &response["id"]).collect();
    assert_eq!(ids, [&json!("a"), &json!(1.5), &json!(u64::MAX)]);
}

#[tokio::test]
async fn accepts_empty_params_for_a_method_without_params() {
    let responses = exchange(concat!(
        "{\"jsonrpc\":\"2.0\",\"method\":\"ping\",\"params\":[],\"id\":1}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"ping\",\"params\":{},\"id\":2}\n",
    ))
    .await;
    assert_eq!(responses[0]["result"], "pong");
    assert_eq!(responses[1]["result"], "pong");
}

#[tokio::test]
async fn answers_a_request_with_a_null_id() {
    let responses =
        exchange("{\"jsonrpc\":\"2.0\",\"method\":\"add\",\"params\":[1,1],\"id\":null}\n").await;
    assert_eq!(
        responses,
        [json!({"jsonrpc": "2.0", "result": 2, "id": null})]
    );
}

#[tokio::test]
async fn answers_malformed_json_with_a_parse_error() {
    assert_eq!(
        code_and_id_of("{\"jsonrpc\": \"2.0\", \"method\"").await,
        (Error::PARSE_ERROR, Value::Null)
    );
}

#[tokio::test]
async fn answers_invalid_utf8_with_a_parse_error() {
    let responses = exchange(b"\"\xff\"\n").await;
    assert_eq!(responses[0]["error"]["code"], Error::PARSE_ERROR);
}

#[tokio::test]
async fn answers_json_that_is_not_a_request_with_invalid_request() {
    for (input, id) in [
        ("{\"method\":\"add\",\"params\":[1,2],\"id\":1}", json!(1)),
        (
            "{\"jsonrpc\":\"1.0\",\"method\":\"add\",\"params\":[1,2],\"id\":2}",
            json!(2),
        ),
        ("{\"jsonrpc\":\"2.0\",\"method\":7,\"id\":3}", json!(3)),
        (
            "{\"jsonrpc\":\"2.0\",\"method\":\"add\",\"params\":\"x\",\"id\":4}",
            json!(4),
        ),
        (
            "{\"jsonrpc\":\"2.0\",\"method\":\"ping\",\"params\":null,\"id\":6}",
            json!(6),
        ),
        ("42", Value::Null),
        (
            "[{\"jsonrpc\":\"2.0\",\"method\":\"add\",\"params\":[1,2],\"id\":5}]",
            Value::Null,
        ),
    ] {
        assert_eq!(
            code_and_id_of(input).await,
            (Error::INVALID_REQUEST, id),
            "{input}"
        );
    }
}

#[tokio::test]
async fn answers_an_unknown_method_with_method_not_found() {
    assert_eq!(
        code_and_id_of("{\"jsonrpc\":\"2.0\",\"method\":\"nope\",\"id\":1}").await,
        (Error::METHOD_NOT_FOUND, json!(1))
    );
}

#[tokio::test]
async fn answers_bad_params_with_invalid_params() {
    assert_eq!(
        code_and_id_of("{\"jsonrpc\":\"2.0\",\"method\":\"add\",\"params\":[\"x\"],\"id\":1}")
            .await,
        (Error::INVALID_PARAMS, json!(1))
    );
}

#[tokio::test]
async fn answers_params_of_the_wrong_type_for_a_typed_method_with_invalid_params() {
    assert_eq!(
        code_and_id_of("{\"jsonrpc\":\"2.0\",\"method\":\"test.sum\",\"params\":[\"x\"],\"id\":1}")
            .await,
        (Error::INVALID_PARAMS, json!(1))
    );
}

#[tokio::test]
async fn answers_an_unserializable_result_with_internal_error() {
    assert_eq!(
        code_and_id_of("{\"jsonrpc\":\"2.0\",\"method\":\"unserializable\",\"id\":1}").await,
        (Error::INTERNAL_ERROR, json!(1))
    );
}

#[tokio::test]
async fn passes_the_handler_error_through() {
    let responses = exchange("{\"jsonrpc\":\"2.0\",\"method\":\"fail\",\"id\":1}\n").await;
    assert_eq!(
        responses,
        [json!({"jsonrpc": "2.0", "error": {"code": -32000, "message": "boom"}, "id": 1})]
    );
}

#[tokio::test]
async fn answers_an_over_long_line_with_invalid_request() {
    let long = format!("\"{}\"", "a".repeat(16 * 1024 * 1024));
    let responses = exchange(format!(
        "{long}\n{{\"jsonrpc\":\"2.0\",\"method\":\"add\",\"params\":[1,2],\"id\":1}}\n"
    ))
    .await;
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["error"]["code"], Error::INVALID_REQUEST);
    assert_eq!(responses[1]["result"], 3);
}

#[tokio::test]
async fn does_not_answer_notifications() {
    let responses = exchange(concat!(
        "{\"jsonrpc\":\"2.0\",\"method\":\"add\",\"params\":[1,2]}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"nope\"}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"add\",\"params\":[3,4],\"id\":9}\n",
    ))
    .await;
    assert_eq!(responses, [json!({"jsonrpc": "2.0", "result": 7, "id": 9})]);
}

#[tokio::test]
async fn keeps_answering_after_an_error_and_skips_blank_lines() {
    let responses = exchange(concat!(
        "not json\n",
        "\n",
        "  \r\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"echo\",\"params\":{\"a\":1},\"id\":2}\r\n",
    ))
    .await;
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["error"]["code"], Error::PARSE_ERROR);
    assert_eq!(responses[1]["result"], json!({"a": 1}));
}

#[tokio::test]
async fn answers_the_last_line_without_a_newline() {
    let responses =
        exchange("{\"jsonrpc\":\"2.0\",\"method\":\"add\",\"params\":[1,2],\"id\":1}").await;
    assert_eq!(responses[0]["result"], 3);
}

#[tokio::test]
async fn client_gets_the_result_or_the_error_object() {
    let mut client = connect_to_router();

    let sum: i64 = client.call("add", (2, 2)).await.unwrap();
    assert_eq!(sum, 4);

    let echoed: Value = client.call("echo", json!({"b": [1]})).await.unwrap();
    assert_eq!(echoed, json!({"b": [1]}));

    match client.call::<_, ()>("fail", ()).await {
        Err(ClientError::Rpc(error)) => assert_eq!(error, Error::new(-32000, "boom")),
        other => panic!("expected an RPC error, got {other:?}"),
    }
}

#[tokio::test]
async fn client_gets_the_typed_result_of_a_typed_method() {
    let sum = connect_to_router().request::<Sum>((2, 3)).await.unwrap();

    assert_eq!(sum, 5);
}

#[test]
fn lists_the_methods_it_answers() {
    let router = Router::new()
        .method("echo", |params: Value| async move { Ok(params) })
        .handle::<Sum>(|(a, b)| async move { Ok(a + b) });

    assert_eq!(router.methods(), ["echo", "test.sum"]);
}

#[tokio::test]
async fn client_takes_an_error_with_a_null_id_as_its_answer() {
    let (client, server) = duplex(1024);
    tokio::spawn(async move {
        let (reader, mut writer) = split(server);
        BufReader::new(reader).lines().next_line().await.unwrap();
        writer
            .write_all(b"{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,\"message\":\"Invalid Request\"},\"id\":null}\n")
            .await
            .unwrap();
    });
    let (reader, writer) = split(client);

    let result = Client::new(reader, writer).call::<_, ()>("ping", ()).await;

    assert!(
        matches!(
            result,
            Err(ClientError::Rpc(Error {
                code: Error::INVALID_REQUEST,
                ..
            }))
        ),
        "{result:?}"
    );
}

#[tokio::test]
async fn client_reports_a_closed_connection() {
    let (client, server) = duplex(1024);
    drop(server);
    let (reader, writer) = split(client);
    let result = Client::new(reader, writer)
        .call::<_, ()>("add", (1, 2))
        .await;
    assert!(
        matches!(result, Err(ClientError::Closed | ClientError::Io(_))),
        "{result:?}"
    );
}
