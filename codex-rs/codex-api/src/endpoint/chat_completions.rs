use crate::auth::SharedAuthProvider;
use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::common::ResponsesApiRequest;
use crate::endpoint::responses::ResponsesOptions;
use crate::endpoint::session::EndpointSession;
use crate::error::ApiError;
use crate::provider::Provider;
use crate::requests::Compression;
use crate::requests::headers::build_session_headers;
use crate::requests::headers::insert_header;
use crate::requests::headers::subagent_header;
use crate::telemetry::SseTelemetry;
use codex_client::ByteStream;
use codex_client::HttpTransport;
use codex_client::RequestCompression;
use codex_client::RequestTelemetry;
use codex_client::StreamResponse;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use http::HeaderMap;
use http::HeaderValue;
use http::Method;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;
use tracing::instrument;
use tracing::trace;

pub struct ChatCompletionsClient<T: HttpTransport> {
    session: EndpointSession<T>,
    sse_telemetry: Option<Arc<dyn SseTelemetry>>,
}

impl<T: HttpTransport> ChatCompletionsClient<T> {
    pub fn new(transport: T, provider: Provider, auth: SharedAuthProvider) -> Self {
        Self {
            session: EndpointSession::new(transport, provider, auth),
            sse_telemetry: None,
        }
    }

    pub fn with_telemetry(
        self,
        request: Option<Arc<dyn RequestTelemetry>>,
        sse: Option<Arc<dyn SseTelemetry>>,
    ) -> Self {
        Self {
            session: self.session.with_request_telemetry(request),
            sse_telemetry: sse,
        }
    }

    #[instrument(
        name = "chat_completions.stream_request",
        level = "info",
        skip_all,
        fields(
            transport = "chat_completions_http",
            http.method = "POST",
            api.path = "chat/completions"
        )
    )]
    pub async fn stream_request(
        &self,
        request: ResponsesApiRequest,
        options: ResponsesOptions,
    ) -> Result<ResponseStream, ApiError> {
        let ResponsesOptions {
            session_id,
            thread_id,
            session_source,
            extra_headers,
            compression,
            turn_state,
        } = options;

        let body = chat_completions_body_from_responses_request(&request)?;

        let mut headers = extra_headers;
        if let Some(ref thread_id) = thread_id {
            insert_header(&mut headers, "x-client-request-id", thread_id);
        }
        headers.extend(build_session_headers(session_id, thread_id));
        if let Some(subagent) = subagent_header(&session_source) {
            insert_header(&mut headers, "x-openai-subagent", &subagent);
        }

        self.stream(body, headers, compression, turn_state).await
    }

    #[instrument(
        name = "chat_completions.stream",
        level = "info",
        skip_all,
        fields(
            transport = "chat_completions_http",
            http.method = "POST",
            api.path = "chat/completions",
            turn.has_state = turn_state.is_some()
        )
    )]
    pub async fn stream(
        &self,
        body: Value,
        extra_headers: HeaderMap,
        compression: Compression,
        turn_state: Option<Arc<OnceLock<String>>>,
    ) -> Result<ResponseStream, ApiError> {
        let request_compression = match compression {
            Compression::None => RequestCompression::None,
            Compression::Zstd => RequestCompression::Zstd,
        };

        let stream_response = self
            .session
            .stream_with(
                Method::POST,
                "chat/completions",
                extra_headers,
                Some(body),
                |req| {
                    req.headers.insert(
                        http::header::ACCEPT,
                        HeaderValue::from_static("text/event-stream"),
                    );
                    req.compression = request_compression;
                },
            )
            .await?;

        Ok(spawn_chat_completions_stream(
            stream_response,
            self.session.provider().stream_idle_timeout,
            self.sse_telemetry.clone(),
            turn_state,
        ))
    }
}

pub fn chat_completions_body_from_responses_request(
    request: &ResponsesApiRequest,
) -> Result<Value, ApiError> {
    let mut body = Map::new();
    body.insert("model".to_string(), Value::String(request.model.clone()));
    body.insert("stream".to_string(), Value::Bool(request.stream));
    body.insert(
        "messages".to_string(),
        Value::Array(chat_messages(request)?),
    );

    let tools = chat_tools(&request.tools)?;
    if !tools.is_empty() {
        body.insert("tools".to_string(), Value::Array(tools));
    }
    if request.tool_choice != "none" {
        body.insert(
            "tool_choice".to_string(),
            Value::String(request.tool_choice.clone()),
        );
    }
    body.insert(
        "parallel_tool_calls".to_string(),
        Value::Bool(request.parallel_tool_calls),
    );
    if let Some(service_tier) = &request.service_tier {
        body.insert(
            "service_tier".to_string(),
            Value::String(service_tier.clone()),
        );
    }

    Ok(Value::Object(body))
}

fn chat_messages(request: &ResponsesApiRequest) -> Result<Vec<Value>, ApiError> {
    let mut messages = Vec::new();
    if !request.instructions.trim().is_empty() {
        messages.push(json!({
            "role": "system",
            "content": request.instructions,
        }));
    }

    for item in &request.input {
        match item {
            ResponseItem::Message { role, content, .. } => {
                messages.push(json!({
                    "role": role,
                    "content": content_items_to_chat_text(content),
                }));
            }
            ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            } => {
                messages.push(json!({
                    "role": "assistant",
                    "tool_calls": [{
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": arguments,
                        },
                    }],
                }));
            }
            ResponseItem::FunctionCallOutput { call_id, output } => {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": function_output_to_chat_text(output),
                }));
            }
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => {
                messages.push(json!({
                    "role": "assistant",
                    "tool_calls": [{
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": input,
                        },
                    }],
                }));
            }
            ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": function_output_to_chat_text(output),
                }));
            }
            ResponseItem::Reasoning { .. }
            | ResponseItem::LocalShellCall { .. }
            | ResponseItem::ToolSearchCall { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::CompactionTrigger
            | ResponseItem::ContextCompaction { .. }
            | ResponseItem::Other => {}
        }
    }

    Ok(messages)
}

fn content_items_to_chat_text(content: &[ContentItem]) -> String {
    content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                Some(text.as_str())
            }
            ContentItem::InputImage { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn function_output_to_chat_text(output: &FunctionCallOutputPayload) -> String {
    match &output.body {
        FunctionCallOutputBody::Text(text) => text.clone(),
        FunctionCallOutputBody::ContentItems(items) => function_output_items_to_text(items),
    }
}

fn function_output_items_to_text(items: &[FunctionCallOutputContentItem]) -> String {
    items
        .iter()
        .filter_map(|item| match item {
            FunctionCallOutputContentItem::InputText { text } => Some(text.as_str()),
            FunctionCallOutputContentItem::InputImage { .. }
            | FunctionCallOutputContentItem::EncryptedContent { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn chat_tools(tools: &[Value]) -> Result<Vec<Value>, ApiError> {
    let mut chat_tools = Vec::new();
    for tool in tools {
        if let Some(tool) = chat_tool(tool)? {
            chat_tools.push(tool);
        }
    }
    Ok(chat_tools)
}

fn chat_tool(tool: &Value) -> Result<Option<Value>, ApiError> {
    if tool.get("type").and_then(Value::as_str) != Some("function") {
        return Ok(None);
    }
    if tool.get("function").is_some() {
        return Ok(Some(tool.clone()));
    }

    let object = tool.as_object().ok_or_else(|| {
        ApiError::Stream("failed to convert non-object tool to Chat Completions".to_string())
    })?;
    let name = object.get("name").cloned().ok_or_else(|| {
        ApiError::Stream("function tool is missing name for Chat Completions".to_string())
    })?;
    let mut function = Map::new();
    function.insert("name".to_string(), name);
    for key in ["description", "parameters", "strict"] {
        if let Some(value) = object.get(key) {
            function.insert(key.to_string(), value.clone());
        }
    }

    Ok(Some(json!({
        "type": "function",
        "function": Value::Object(function),
    })))
}

#[derive(Default)]
pub struct ChatCompletionsStreamParser {
    tool_calls: BTreeMap<u64, PartialToolCall>,
    last_response_id: Option<String>,
    assistant_message_id: Option<String>,
    assistant_message_text: String,
}

#[derive(Default)]
struct PartialToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl ChatCompletionsStreamParser {
    pub fn process_chunk(&mut self, chunk: &Value) -> Result<Vec<ResponseEvent>, ApiError> {
        if let Some(id) = chunk.get("id").and_then(Value::as_str) {
            self.last_response_id = Some(id.to_string());
        }

        let mut events = Vec::new();
        let Some(choices) = chunk.get("choices").and_then(Value::as_array) else {
            return Ok(events);
        };

        for choice in choices {
            if let Some(content) = choice
                .get("delta")
                .and_then(|delta| delta.get("content"))
                .and_then(Value::as_str)
                && !content.is_empty()
            {
                self.start_assistant_message_if_needed(&mut events);
                self.assistant_message_text.push_str(content);
                events.push(ResponseEvent::OutputTextDelta(content.to_string()));
            }

            if let Some(tool_calls) = choice
                .get("delta")
                .and_then(|delta| delta.get("tool_calls"))
                .and_then(Value::as_array)
            {
                if !tool_calls.is_empty()
                    && let Some(done) = self.finish_assistant_message()
                {
                    events.push(done);
                }
                for tool_call in tool_calls {
                    if let Some(event) = self.process_tool_call_delta(tool_call)? {
                        events.push(event);
                    }
                }
            }

            if choice.get("finish_reason").and_then(Value::as_str) == Some("tool_calls") {
                events.extend(self.finish_tool_calls());
            }
        }

        Ok(events)
    }

    pub fn process_done(&mut self, fallback_response_id: String) -> Vec<ResponseEvent> {
        let response_id = self
            .last_response_id
            .clone()
            .unwrap_or(fallback_response_id);
        let mut events = Vec::new();
        if let Some(done) = self.finish_assistant_message() {
            events.push(done);
        }
        events.extend(self.finish_tool_calls());
        events.push(ResponseEvent::Completed {
            response_id,
            token_usage: None,
            end_turn: Some(true),
        });
        events
    }

    fn start_assistant_message_if_needed(&mut self, events: &mut Vec<ResponseEvent>) {
        if self.assistant_message_id.is_some() {
            return;
        }

        let id = self.assistant_message_item_id();
        self.assistant_message_id = Some(id.clone());
        events.push(ResponseEvent::OutputItemAdded(ResponseItem::Message {
            id: Some(id),
            role: "assistant".to_string(),
            content: Vec::new(),
            phase: None,
        }));
    }

    fn assistant_message_item_id(&self) -> String {
        self.last_response_id
            .as_ref()
            .map(|id| format!("{id}_message"))
            .unwrap_or_else(|| "chatcmpl_unknown_message".to_string())
    }

    fn finish_assistant_message(&mut self) -> Option<ResponseEvent> {
        let id = self.assistant_message_id.take()?;
        let text = std::mem::take(&mut self.assistant_message_text);
        Some(ResponseEvent::OutputItemDone(ResponseItem::Message {
            id: Some(id),
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText { text }],
            phase: None,
        }))
    }

    fn process_tool_call_delta(
        &mut self,
        tool_call: &Value,
    ) -> Result<Option<ResponseEvent>, ApiError> {
        let index = tool_call.get("index").and_then(Value::as_u64).unwrap_or(0);
        let partial = self.tool_calls.entry(index).or_default();
        if let Some(id) = tool_call.get("id").and_then(Value::as_str) {
            partial.id = Some(id.to_string());
        }
        if let Some(name) = tool_call
            .get("function")
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
        {
            partial.name = Some(name.to_string());
        }
        let Some(delta) = tool_call
            .get("function")
            .and_then(|function| function.get("arguments"))
            .and_then(Value::as_str)
        else {
            return Ok(None);
        };
        partial.arguments.push_str(delta);

        let call_id = partial
            .id
            .clone()
            .unwrap_or_else(|| format!("call_{index}"));
        Ok(Some(ResponseEvent::ToolCallInputDelta {
            item_id: call_id.clone(),
            call_id: Some(call_id),
            delta: delta.to_string(),
        }))
    }

    fn finish_tool_calls(&mut self) -> Vec<ResponseEvent> {
        std::mem::take(&mut self.tool_calls)
            .into_iter()
            .map(|(index, partial)| {
                let call_id = partial.id.unwrap_or_else(|| format!("call_{index}"));
                ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                    id: None,
                    name: partial.name.unwrap_or_else(|| "tool".to_string()),
                    namespace: None,
                    arguments: partial.arguments,
                    call_id,
                })
            })
            .collect()
    }
}

fn spawn_chat_completions_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
    turn_state: Option<Arc<OnceLock<String>>>,
) -> ResponseStream {
    let upstream_request_id = stream_response
        .headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    if let Some(turn_state) = turn_state.as_ref()
        && let Some(header_value) = stream_response
            .headers
            .get("x-codex-turn-state")
            .and_then(|v| v.to_str().ok())
    {
        let _ = turn_state.set(header_value.to_string());
    }

    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(async move {
        process_chat_completions_sse(stream_response.bytes, tx_event, idle_timeout, telemetry)
            .await;
    });

    ResponseStream {
        rx_event,
        upstream_request_id,
    }
}

async fn process_chat_completions_sse(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) {
    let mut stream = stream.eventsource();
    let mut parser = ChatCompletionsStreamParser::default();

    loop {
        let start = Instant::now();
        let response = timeout(idle_timeout, stream.next()).await;
        if let Some(t) = telemetry.as_ref() {
            t.on_sse_poll(&response, start.elapsed());
        }
        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(e))) => {
                debug!("Chat Completions SSE error: {e:#}");
                let _ = tx_event.send(Err(ApiError::Stream(e.to_string()))).await;
                return;
            }
            Ok(None) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(
                        "stream closed before chat completion finished".into(),
                    )))
                    .await;
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream("idle timeout waiting for SSE".into())))
                    .await;
                return;
            }
        };

        trace!("Chat Completions SSE event: {}", &sse.data);
        if sse.data.trim() == "[DONE]" {
            for event in parser.process_done("chatcmpl_unknown".to_string()) {
                if tx_event.send(Ok(event)).await.is_err() {
                    return;
                }
            }
            return;
        }

        let chunk: Value = match serde_json::from_str(&sse.data) {
            Ok(chunk) => chunk,
            Err(e) => {
                debug!(
                    "failed to parse Chat Completions chunk: {e}, data: {}",
                    &sse.data
                );
                continue;
            }
        };

        match parser.process_chunk(&chunk) {
            Ok(events) => {
                for event in events {
                    if tx_event.send(Ok(event)).await.is_err() {
                        return;
                    }
                }
            }
            Err(error) => {
                let _ = tx_event.send(Err(error)).await;
                return;
            }
        }
    }
}

#[cfg(test)]
#[path = "chat_completions_tests.rs"]
mod tests;
