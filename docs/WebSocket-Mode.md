# WebSocket Mode

The Responses API supports a WebSocket mode for long-running, tool-call-heavy workflows. In this mode, you keep a persistent connection to `/v1/responses` and continue each turn by sending only new input items plus `previous_response_id`.

The provider request supports `store=false`. This does not by itself disable
Aether's local usage and audit body capture; configure Aether's retention and
body-capture policy separately when request persistence is not acceptable.

## Why use WebSocket mode

WebSocket mode is most useful when a workflow involves many model-tool round trips (for example, agentic coding or orchestration loops with repeated tool calls).

Because the connection stays open and each turn sends only incremental input, WebSocket mode reduces per-turn continuation overhead and improves end-to-end latency across long chains. For rollouts with 20+ tool calls, we have seen up to roughly 40% faster end-to-end execution.

## Connect and create responses

In WebSocket mode, start each turn by sending a `response.create` event from the client. The payload mirrors the normal [Responses create body](https://developers.openai.com/api/reference/resources/responses/methods/create), except that transport-specific fields like `stream` and `background` are not used.

```python
from websocket import create_connection
import json
import os

ws = create_connection(
    "wss://api.openai.com/v1/responses",
    header=[
        f"Authorization: Bearer {os.environ['OPENAI_API_KEY']}",
    ],
)

ws.send(
    json.dumps(
        {
            "type": "response.create",
            "model": "gpt-5.6",
            "store": False,
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "Find fizz_buzz()"}],
                }
            ],
            "tools": [],
        }
    )
)
```


Clients can optionally warm up request state by sending `response.create` with `generate: false`. This is useful when you already know the tools, instructions, and/or custom messages you plan to send with an upcoming turn. `generate: false` does not return a model output, but prepares request state so the next generated turn can start faster. The warmup request returns a response ID that you can chain from with `previous_response_id`, including on later turns in a response chain. The next section explains how to continue a session using `previous_response_id` and incremental inputs.

## Continue with incremental inputs

To continue a run, send another `response.create` with:

- `previous_response_id` set to the prior response ID.
- `input` containing only new items (for example, tool outputs and the next user message).

```python
ws.send(
    json.dumps(
        {
            "type": "response.create",
            "model": "gpt-5.6",
            "store": False,
            "previous_response_id": "resp_123",
            "input": [
                {
                    "type": "function_call_output",
                    "call_id": "call_123",
                    "output": "tool result",
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "Now optimize it."}],
                },
            ],
            "tools": [],
        }
    )
)
```


## How continuation works

WebSocket mode uses the same `previous_response_id` chaining semantics as HTTP mode, but it adds a lower-latency continuation path on the active socket.

On an active Aether WebSocket connection, the gateway remembers only the most recent response ID that it successfully delivered to that client. A continuation is accepted only when `previous_response_id` matches that connection-local ID. Older IDs, IDs observed on another connection, and IDs supplied on the first turn return `previous_response_not_found` before authentication, routing, or provider admission.

This restriction is intentional even when the provider supports `store=true`: Aether provider accounts can be shared by multiple tenants, so provider-side persistence is not a client ownership boundary. Cross-connection continuation will require an Aether-owned response registry scoped to the authenticated user or API key. Until that registry exists, reconnect by starting a new chain with complete input.

The connection-local continuation cache stores only the last public response
ID in memory for the socket lifetime. This limits continuation state, but it
does not override Aether's independently configured usage and audit retention.

## Compaction and creating new responses

If you are using compaction, there are two different continuation patterns:

### Server-side compaction (`context_management`)

When you enable server-side compaction (`context_management` with `compact_threshold`), compaction happens during normal `/responses` generation. In WebSocket mode, you continue the same way you normally do: send the next `response.create` with the latest `previous_response_id` and only new input items.

### Standalone `/responses/compact`

The standalone [`/responses/compact` endpoint](https://developers.openai.com/api/docs/api-reference/responses/compact) returns a new compacted input window, not a response ID. After compaction, create a new response on your WebSocket connection using the compacted window as `input` (plus the next user/tool items).

Start a new chain by omitting `previous_response_id` or setting it to `null`. Pass the compacted output as-is; do not prune the returned window.

```python
# Compact your current window (HTTP call)
compacted = client.responses.compact(
    model="gpt-5.6",
    input=long_input_items_array,
)

# Start a new response on the WebSocket using the compacted window
ws.send(
    json.dumps(
        {
            "type": "response.create",
            "model": "gpt-5.6",
            "store": False,
            "input": [
                *compacted.output,
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "Continue from here."}],
                },
            ],
            "tools": [],
        }
    )
)
```


## Connection behavior and limits

- Server events and ordering match the existing Responses streaming event model.
- A single WebSocket connection can receive multiple `response.create` messages, but it runs them sequentially (one in-flight response at a time).
- No multiplexing support today. Use multiple connections if you need parallel runs.
- Connection duration is limited to 60 minutes. Reconnect when the limit is reached.
- Aether's public socket always accepts and emits the OpenAI Responses WebSocket protocol. Provider-private envelopes, quota/account metadata, binary frames, and provider Close details are not part of that public contract.
- The current upstream backend is native Responses WebSocket passthrough. A provider must explicitly enable that capability and expose an `openai:responses` endpoint before it is eligible. HTTP/SSE or cross-format execution belongs in a separate backend behind the same public protocol boundary.
- Aether binds each native upstream WebSocket to one selected provider key. A provider observer independently watches Codex quota events without changing the public codec or backend protocol. A `usage_limit_reached` terminal error immediately marks the bound account unavailable. If the client has not received a standard `response.*` event and the request has no `previous_response_id`, Aether retries that one turn once on another eligible key without closing the public socket.
- After a standard response event has reached the client, after a retry has already been attempted, or for a request using `previous_response_id`, Aether emits a sanitized public error and detaches only the exhausted upstream. If the upstream closes immediately after the quota signal, Aether emits a recoverable gateway error instead. The public WebSocket stays open so a later independent `response.create` can select another key.
- Aether does not transparently move an existing response chain to another provider key. Connection-local `previous_response_id` state cannot be transferred safely, especially with `store=false`; send a new request with complete input after an exhausted continuation.

## Reconnect and recover

When a connection closes (or hits the 60-minute limit), open a new WebSocket connection and use one of these patterns:

1. Start a new response by setting `previous_response_id` to `null` (or omitting it) and send the full input context for the next turn.
2. If you compacted context with `/responses/compact`, use the returned compacted window as the base `input` for that new response, then append the latest user/tool items.

Do not send the prior connection's response ID as the first request on the new socket. Aether currently rejects cross-connection continuation with `previous_response_not_found`, including when the upstream response used `store=true`.

## Errors to handle

`previous_response_not_found`

```json
{
  "type": "error",
  "status": 400,
  "error": {
    "code": "previous_response_not_found",
    "message": "Previous response with id 'resp_abc' not found.",
    "param": "previous_response_id"
  }
}
```

`websocket_connection_limit_reached`

```json
{
  "type": "error",
  "error": {
    "type": "invalid_request_error",
    "code": "websocket_connection_limit_reached",
    "message": "Responses websocket connection limit reached (60 minutes). Create a new websocket connection to continue."
  },
  "status": 400
}
```

## Related guides

- [Conversation state](https://developers.openai.com/api/docs/guides/conversation-state)
- [Streaming API responses](https://developers.openai.com/api/docs/guides/streaming-responses)
- [Responses streaming events reference](https://developers.openai.com/api/docs/api-reference/responses-streaming)
