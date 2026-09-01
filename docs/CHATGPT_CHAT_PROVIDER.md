# ChatGPT Web worker provider

SWARMS can use a real ChatGPT Web conversation as a worker through the `chatgpt_chat` wrapper.
This route uses the locally logged-in ChatGPT product through `Mchicao/chatgpt-pc-mcp`; it does not call the OpenAI API.

## Architecture

```text
SWARMS Rust runtime
       |
       | native HTTP
       v
chatgpt-pc-mcp orchestration broker
       |
       v
MV3 companion extension
       |
       v
real ChatGPT worker conversation
```

The Rust runtime never invokes Python. The Python broker is a separately running local service, just like any other
external provider endpoint.

## Enable locally

The committed `chatgpt_chat` route is intentionally disabled. Enable it only in the ignored
`config/swarm_router.local.json` so CI and other users never consume ChatGPT product quota accidentally.

Example provider override:

```json
{
  "aliases": {
    "chatgpt": "chatgpt_chat"
  },
  "providers": {
    "chatgpt_chat": {
      "enabled": true,
      "provider": "chatgpt_chat",
      "model": "chatgpt-web",
      "canonical_model": "chatgpt-web",
      "wrapper": "chatgpt_chat",
      "key_env": "CHATGPT_CHAT_BROKER_TOKEN",
      "base_url_env": "CHATGPT_CHAT_BROKER_URL",
      "quota_key": "chatgpt:chat",
      "fallback_routes": ["mock"],
      "health_key": "chatgpt_chat",
      "metric_key": "chatgpt_chat",
      "relative_cost": 0.0,
      "quality": 0.90,
      "scarcity": 0.65,
      "strengths": ["agentic", "long-running", "review", "coding", "browser", "computer-use", "session-reuse"],
      "weaknesses": ["requires companion extension", "subscription quota is opaque", "browser UI can change"]
    }
  }
}
```

Set the broker connection in the environment used to launch SWARMS:

```powershell
$env:CHATGPT_CHAT_BROKER_URL = "http://127.0.0.1:8771"
$env:CHATGPT_CHAT_BROKER_TOKEN = "<token from chatgpt-pc-mcp/orchestration.json>"
```

For a remote PC, point the route at that PC's broker using an HTTPS/private encrypted endpoint and that broker's
token. Each physical PC should still have its own OpenAI Secure MCP tunnel ID for PC-control; broker routing is
explicit and separate from the tunnel queue.

## Plan usage

Use `route: "chatgpt_chat"` on a task. The normal SWARMS task prompt becomes the first user message in a new real
ChatGPT worker conversation. `chatgpt-pc-mcp` assigns that worker a Goal equal to the task so short/interrupted
ChatGPT turns are automatically continued until the goal is clearly complete or the hard continuation limit is hit.

Session affinity is supported:

- a new task stores the broker `worker_id` as the provider session ID;
- `session.mode: "reuse"` sends the next prompt to the exact same ChatGPT conversation;
- SWARMS Steering also uses that resumable worker ID at its safe session boundary.

The runtime blocks until the requested worker generation is `sleeping`/`finished` with a non-empty final result.
A Goal that reaches `exhausted` or `user_stopped`, or a worker in `failed`, fails the SWARMS task rather than being
reported as successful.

## Verification semantics

The ChatGPT worker does not bypass SWARMS' deterministic completion gates. After the broker returns:

1. artifact/protected-path checks still run;
2. task `verify` commands still run locally;
3. feature-producing roles still require successful verification according to normal SWARMS policy;
4. usage is reported as `missing`, not zero, because the ChatGPT Web product does not expose reliable token
   telemetry through this bridge.

## Timeouts and failure modes

`CHATGPT_CHAT_TIMEOUT_SECONDS` controls how long one worker generation may run. Default: 3600 seconds; accepted
range: 30 to 21600 seconds.

The route fails if:

- the local/remote broker is unreachable or unauthenticated;
- the companion browser extension is offline when a new chat must be spawned;
- the browser command fails to open/bind a worker conversation;
- the worker fails, is explicitly stopped, or exhausts its Goal continuation budget;
- a generation finishes without a final assistant result;
- the timeout expires.

Browser DOM compatibility is outside the deterministic Rust runtime. Treat a ChatGPT UI update as a provider health
issue, not as permission to bypass SWARMS verification.
