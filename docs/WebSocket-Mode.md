# WebSocket Mode

The Responses API supports a WebSocket mode for long-running, tool-call-heavy workflows. In this mode, you keep a persistent connection to `/v1/responses` and continue each turn by sending only new input items plus `previous_response_id`.

WebSocket mode is compatible with both Zero Data Retention (ZDR) and `store=false`.

OpenAI's current WebSocket service supports named `stream_id` lanes: requests on
the same lane are FIFO, while different lanes may run concurrently. Aether's
bridge currently exposes only the implicit default lane and deliberately
rejects `response.create.stream_id` until per-lane binding, ordering, timeout,
usage, and error routing are implemented end to end. Use separate WebSocket
connections for parallel runs through Aether. A syntactically valid named
`stream_id` is rejected with `responses_websocket_named_stream_unsupported`;
the error event echoes the validated ID so the client can associate the error
with its attempted lane. Invalid or untrusted IDs are not echoed.

## Why use WebSocket mode

WebSocket mode is most useful when a workflow involves many model-tool round trips (for example, agentic coding or orchestration loops with repeated tool calls).

Because the connection stays open and each turn sends only incremental input, WebSocket mode reduces per-turn continuation overhead and improves end-to-end latency across long chains. The [OpenAI WebSocket-mode guide](https://developers.openai.com/api/docs/guides/websocket-mode) reports up to roughly 40% faster end-to-end execution for workloads with 20 or more tool calls; this is an upstream product claim, not an Aether benchmark.

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

On an active Aether WebSocket connection, the selected upstream keeps the
previous-response state for the single default lane in its connection-local
cache. Continuing from that most recent response is fast because the service
can reuse connection-local state. Because the previous-response state is
retained only in memory and is not written to disk, you can use WebSocket mode
in a way that is compatible with `store=false` and Zero Data Retention (ZDR).

If a `previous_response_id` is not in the upstream connection's in-memory
cache, behavior depends on whether the upstream stored the response:

- With `store=true`, the upstream service may hydrate older response IDs from its persisted state when available. Continuation can still work, but it usually loses the in-memory latency benefit.
- With `store=false` (including ZDR), there is no persisted fallback. If the ID is uncached, the request returns `previous_response_not_found`.

For a new downstream WebSocket connection, Aether also has to prove that the
response belongs to the currently authenticated user/API key and to the exact
provider endpoint, key, credential generation, transport, adapter, model, and
normalization contract. Aether records this ownership only when the effective
provider `response.create`, after Aether's body rules and framing, explicitly
has `store=true` and a successful
`response.completed`, `response.done`, or non-error `response.incomplete`
terminal supplies a valid response ID. `store=false`, an omitted/overridden
`store`, failures, cancellations, malformed IDs, and ZDR turns never create
this registry state. This explicit-true rule is intentionally conservative:
Aether does not infer a provider default for an omitted `store` field.

The RuntimeState key contains a SHA-256 digest over length-delimited live
`user_id`, `api_key_id`, and the opaque response ID; raw response IDs and
credentials are not stored in keys or values. Records expire after 24 hours,
are capped at the 1,024 most recently registered IDs per user/API-key pair,
and are bounded in serialized size. The registry stores ownership/routing
proof and contract digests only; it does not store response contents and is
not a replacement for the upstream's `store=true` persistence. A registry
write failure does not turn a successful provider response into a failure, so
that terminal can reach the client but cannot later resume on a new socket.

With the Redis RuntimeState backend, ownership is shared across gateway
instances for the record TTL, subject to that Redis deployment's own
availability and persistence configuration. The memory backend is
process-local, is not shared between instances, and loses the registry on
restart. Expiry, per-principal eviction, a RuntimeState outage, or a memory
backend restart causes the first continuation on a new connection to fail
closed with `previous_response_not_found`, even if the upstream might still
retain the response. Aether never falls back to the ordinary scheduler for
such a miss and never sends the opaque response ID to a different provider or
key.

PII-redaction restore mappings intentionally remain connection-local and are
not persisted. If a stored response chain contains Aether PII sentinels, Aether
rejects cross-connection continuation rather than risk exposing those
sentinels without the original restore mapping. Start a new response with the
complete required context in that case.

If a continuation on the same lane fails (`4xx` or `5xx`), the service evicts
the referenced `previous_response_id` from the connection-local cache. Aether
only supports the implicit default lane, so this same-lane rule applies to all
continuations it currently accepts. The upstream service preserves a shared
parent when a cross-lane fork fails, but Aether does not yet expose that named
lane behavior.

The continuation must keep the model selected for the response chain. Aether
rejects a model change with status `409` and code
`responses_continuation_model_change_unsupported`.

## Compaction and creating new responses

If you are using compaction, there are two different continuation patterns:

### Server-side compaction (`context_management`)

When you enable server-side compaction (`context_management` with `compact_threshold`), compaction happens during normal `/responses` generation. In WebSocket mode, you continue the same way you normally do: send the next `response.create` with the latest `previous_response_id` and only new input items.

### Standalone `/responses/compact`

The standalone [`/responses/compact` endpoint](https://developers.openai.com/api/reference/resources/responses/methods/compact) returns a new compacted input window, not a response ID. After compaction, create a new response on your WebSocket connection using the compacted window as `input` (plus the next user/tool items).

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
- A single Aether WebSocket connection can receive multiple `response.create` messages over its lifetime, but the client must wait for a terminal event before sending the next one. Aether does not queue overlapping creates and returns `response_already_in_progress` while a turn is active.
- Named `stream_id` multiplexing is not exposed by Aether yet. Use multiple connections if you need parallel runs.
- The upstream OpenAI service allows at most 16 active/in-flight responses on one connection; additional `response.create` events are queued. It also allows at most 32 distinct named `stream_id` values per connection, and the implicit default lane does not count toward that 32-lane limit. These describe upstream multiplexing limits, not capabilities exposed by Aether's current single-lane bridge.
- Connection duration is limited to 60 minutes. Reconnect when the limit is reached.
- Aether binds each upstream WebSocket to one selected provider key. A provider must explicitly enable the standard Responses WebSocket capability and expose an `openai:responses` endpoint before it is eligible for this bridge.
- The Codex adapter additionally watches Codex quota events. A `usage_limit_reached` terminal error immediately marks the bound account unavailable. If the client has not received a standard `response.*` event and the request has no `previous_response_id`, Aether retries that one turn once on another eligible key without closing the public socket.
- After a standard response event has reached the client, after a retry has already been attempted, or for a request using `previous_response_id`, Aether forwards the provider terminal error and detaches only the exhausted upstream. If the upstream closes immediately after the quota signal, Aether emits a recoverable gateway error instead. The public WebSocket stays open so a later independent `response.create` can select another key.
- Aether does not transparently move an existing response chain to another provider key. Connection-local `previous_response_id` state cannot be transferred safely, especially with `store=false`/ZDR; send a new request with complete input after an exhausted continuation.

## Reconnect and recover

When a connection closes (or hits the 60-minute limit), open a new WebSocket
connection and continue with one of these patterns:

1. If the prior response is persisted (`store=true`) and its response ID remains valid, continue with `previous_response_id` and only the new input items.
2. If the chain cannot be hydrated (for example, `store=false`/ZDR or `previous_response_not_found`), start a new response by setting `previous_response_id` to `null` (or omitting it) and send the complete input context needed for the next turn.
3. If you compacted context with `/responses/compact`, use the returned compacted window as the base `input` for that new response, then append the latest user/tool items.

## Errors to handle

`previous_response_not_found`

```json
{
  "type": "error",
  "status": 400,
  "error": {
    "type": "invalid_request_error",
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
- [Responses streaming events reference](https://developers.openai.com/api/reference/resources/responses)
- [Responses WebSocket events reference](https://developers.openai.com/api/reference/resources/responses/websocket-events)
