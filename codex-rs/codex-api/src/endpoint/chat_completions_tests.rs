use super::*;
use crate::common::ResponseEvent;
use crate::common::ResponsesApiRequest;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn translates_responses_request_to_chat_completions_body() {
    let request = ResponsesApiRequest {
        model: "ai/qwen3-coder".to_string(),
        instructions: "You are a coding agent.".to_string(),
        input: vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "List files".to_string(),
                }],
                phase: None,
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "shell".to_string(),
                namespace: None,
                arguments: r#"{"cmd":"ls"}"#.to_string(),
                call_id: "call_1".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call_1".to_string(),
                output: FunctionCallOutputPayload::from_text("README.md".to_string()),
            },
        ],
        tools: vec![json!({
            "type": "function",
            "name": "shell",
            "description": "Run a shell command",
            "parameters": {
                "type": "object",
                "properties": {
                    "cmd": { "type": "string" }
                },
                "required": ["cmd"]
            },
            "strict": true
        })],
        tool_choice: "auto".to_string(),
        parallel_tool_calls: true,
        reasoning: None,
        store: false,
        stream: true,
        include: vec!["reasoning.encrypted_content".to_string()],
        service_tier: Some("flex".to_string()),
        prompt_cache_key: Some("cache-key".to_string()),
        text: None,
        client_metadata: None,
    };

    let body = chat_completions_body_from_responses_request(&request).unwrap();

    assert_eq!(
        body,
        json!({
            "model": "ai/qwen3-coder",
            "stream": true,
            "messages": [
                { "role": "system", "content": "You are a coding agent." },
                { "role": "user", "content": "List files" },
                {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "shell",
                            "arguments": "{\"cmd\":\"ls\"}"
                        }
                    }]
                },
                {
                    "role": "tool",
                    "tool_call_id": "call_1",
                    "content": "README.md"
                }
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "shell",
                    "description": "Run a shell command",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "cmd": { "type": "string" }
                        },
                        "required": ["cmd"]
                    },
                    "strict": true
                }
            }],
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "service_tier": "flex"
        })
    );
}

#[test]
fn omits_responses_only_tools_from_chat_completions_body() {
    let request = ResponsesApiRequest {
        model: "ai/qwen3-coder".to_string(),
        instructions: String::new(),
        input: vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "List files".to_string(),
            }],
            phase: None,
        }],
        tools: vec![
            json!({
                "type": "function",
                "name": "shell",
                "description": "Run a shell command",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "cmd": { "type": "string" }
                    },
                    "required": ["cmd"]
                }
            }),
            json!({
                "type": "custom",
                "name": "apply_patch",
                "description": "Apply a patch"
            }),
            json!({
                "type": "namespace",
                "name": "mcp__docker",
                "description": "Docker MCP tools",
                "tools": [{
                    "type": "function",
                    "name": "list_containers",
                    "description": "List containers",
                    "parameters": { "type": "object", "properties": {} }
                }]
            }),
            json!({ "type": "web_search" }),
            json!({ "type": "local_shell" }),
        ],
        tool_choice: "auto".to_string(),
        parallel_tool_calls: true,
        reasoning: None,
        store: false,
        stream: true,
        include: Vec::new(),
        service_tier: None,
        prompt_cache_key: None,
        text: None,
        client_metadata: None,
    };

    let body = chat_completions_body_from_responses_request(&request).unwrap();

    assert_eq!(
        body["tools"],
        json!([{
            "type": "function",
            "function": {
                "name": "shell",
                "description": "Run a shell command",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "cmd": { "type": "string" }
                    },
                    "required": ["cmd"]
                }
            }
        }])
    );
}

#[test]
fn streaming_text_chunk_opens_assistant_item_before_delta() {
    let mut parser = ChatCompletionsStreamParser::default();

    let events = parser
        .process_chunk(&json!({
            "id": "chatcmpl_1",
            "choices": [{
                "index": 0,
                "delta": { "content": "hello" }
            }]
        }))
        .unwrap();

    assert!(matches!(
        events.as_slice(),
        [
            ResponseEvent::OutputItemAdded(ResponseItem::Message {
                id: Some(id),
                role,
                content,
                phase: None,
            }),
            ResponseEvent::OutputTextDelta(delta),
        ] if id == "chatcmpl_1_message"
            && role == "assistant"
            && content.is_empty()
            && delta == "hello"
    ));
}

#[test]
fn done_chunk_closes_streamed_assistant_message_before_completed() {
    let mut parser = ChatCompletionsStreamParser::default();

    parser
        .process_chunk(&json!({
            "id": "chatcmpl_1",
            "choices": [{
                "index": 0,
                "delta": { "content": "hello" }
            }]
        }))
        .unwrap();
    parser
        .process_chunk(&json!({
            "id": "chatcmpl_1",
            "choices": [{
                "index": 0,
                "delta": { "content": " world" }
            }]
        }))
        .unwrap();

    let events = parser.process_done("chatcmpl_unknown".to_string());

    assert!(matches!(
        events.as_slice(),
        [
            ResponseEvent::OutputItemDone(ResponseItem::Message {
                id: Some(id),
                role,
                content,
                phase: None,
            }),
            ResponseEvent::Completed {
                response_id,
                token_usage: None,
                end_turn: Some(true),
            },
        ] if id == "chatcmpl_1_message"
            && role == "assistant"
            && content == &[ContentItem::OutputText {
                text: "hello world".to_string()
            }]
            && response_id == "chatcmpl_1"
    ));
}

#[test]
fn streaming_tool_call_chunks_produce_delta_and_done_events() {
    let mut parser = ChatCompletionsStreamParser::default();

    let first_events = parser
        .process_chunk(&json!({
            "id": "chatcmpl_1",
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "shell",
                            "arguments": "{\"cmd\":"
                        }
                    }]
                }
            }]
        }))
        .unwrap();
    let second_events = parser
        .process_chunk(&json!({
            "id": "chatcmpl_1",
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": {
                            "arguments": "\"ls\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }))
        .unwrap();

    assert!(matches!(
        first_events.as_slice(),
        [ResponseEvent::ToolCallInputDelta { item_id, call_id, delta }]
            if item_id == "call_1" && call_id.as_deref() == Some("call_1") && delta == "{\"cmd\":"
    ));
    assert!(matches!(
        second_events.as_slice(),
        [
            ResponseEvent::ToolCallInputDelta { item_id, call_id, delta },
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                id: None,
                name,
                namespace: None,
                arguments,
                call_id: done_call_id,
            })
        ] if item_id == "call_1"
            && call_id.as_deref() == Some("call_1")
            && delta == "\"ls\"}"
            && name == "shell"
            && arguments == "{\"cmd\":\"ls\"}"
            && done_call_id == "call_1"
    ));
}

#[test]
fn done_chunk_produces_completed_event() {
    let mut parser = ChatCompletionsStreamParser::default();

    let events = parser.process_done("chatcmpl_1".to_string());

    assert!(matches!(
        events.as_slice(),
        [ResponseEvent::Completed {
            response_id,
            token_usage: None,
            end_turn: Some(true),
        }] if response_id == "chatcmpl_1"
    ));
}
