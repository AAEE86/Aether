# xAI provider behavior

The following rules preserve the provider-specific behavior of the `xai` provider
across Aether's request and transport layers.

## Responses and tools

- HTTP requests drop `previous_response_id`. Clients must supply conversation
  history; this provider does not add an HTTP response-ID history store.
- `metadata.user_id` is removed. Claude clients copy it onto converted Responses
  bodies and xAI rejects the field.
- Preserve requested `reasoning.encrypted_content`. On a native Responses-to-Responses
  hop, keep provider-owned input items instead of rebuilding them through the canonical
  format. xAI encrypted reasoning may have IDs that do not use OpenAI's `rs` prefix.
  Aether's Gemini signature carriers remain excluded from xAI replay.
- The replay policy is selected from the configured provider type. A model called
  `grok-*` on another provider does not opt into that policy. WebSocket continuation
  metadata retains the selected policy across reconnects.
- A regular client function called `web_search` remains a function. Claude hosted
  search choices are resolved against the original typed tool declaration, including
  declarations with a different name.
- When only `image_generation` is allowed, keep only that tool and retain the requested
  `auto` or `required` mode. For mixed allowed-tool lists, remove the image choice while
  preserving the other allowed entries, as required by xAI's tool-choice schema.
- Reasoning effort is stripped for models that do not accept it.
- OpenAI-style image reference aliases in a request body are rewritten to xAI's
  shape without touching chat message parts.

## Routing and credentials

OAuth requests default to `https://cli-chat-proxy.grok.com/v1`; API-key or
`using_api=true` requests default to `https://api.x.ai/v1`. Explicit custom gateways
are preserved. Compact remains on the official endpoint. CLI identity headers are
applied where the selected upstream requires them.

Account binding uses the xAI device code flow: the gateway requests a device code,
the operator authorizes it out of band, and the gateway polls for the token set.
There is no local callback listener, so headless deployments can bind accounts.
Refresh tokens can also be imported individually or in batches, and are rotated
on refresh.

Quota refresh reads `/user` and `/billing?format=credits` and stores a structured
usage snapshot. A prepaid balance keeps an account selectable after the weekly
allowance is exhausted. API-key accounts skip the subscription billing surface.

## Regression coverage

The format tests cover client and hosted search choices, image-only and mixed tool
restrictions, encrypted reasoning replay, image reference rewriting, and unchanged
OpenAI replay restrictions. Transport tests cover OAuth/API-key/custom routing and
the fixed-provider endpoint template. OAuth tests cover the device code lifecycle,
token import, and batch import.

```sh
cargo test -p aether-ai-formats -p aether-provider-transport -p aether-oauth --lib
cargo test -p aether-gateway --lib xai
```
