# Ollama setup (local semantic search)

tpt-finder's semantic layer answers queries like *"the PDF invoice from John last March"* by embedding extracted document text and matching meaning, not just keywords. **All inference runs on your machine** — the only network endpoint tpt-finder ever contacts is the Ollama server on `localhost`, and the client refuses any non-loopback URL.

## 1. Install Ollama

Download from <https://ollama.com> (Windows / Linux) and make sure the service is running:

```bash
ollama --version
curl http://127.0.0.1:11434/api/tags   # should return installed models
```

## 2. Pull the models

An embedding model is required, a chat model only if you want natural-language query parsing:

```bash
ollama pull nomic-embed-text   # default embedding model
ollama pull llama3.2           # default chat model (NL parsing)
```

Any embedding model works — set its name in *Settings → Embedding model*. Chunk size, chunks per file, and request throttle are also configurable there.

## 3. Enable the layer

Main window → **Settings → Semantic search**:

1. Check **Enable semantic layer**.
2. Confirm the URL (`http://127.0.0.1:11434`) and model names.
3. Click **Check Ollama** — it should report the reachable server and whether the configured models are present.
4. **Save settings**, then click **Embed content** to run a background embedding pass over already-extracted documents.

From then on, new/changed files flow through extraction → embedding automatically, throttled to avoid saturating CPU/GPU.

## Behavior without Ollama

- Disabled or unreachable Ollama changes nothing else: filename search, full-text (FTS5) search, and all result actions keep working.
- `rebuild_semantic` fails fast with "semantic layer is disabled" when the switch is off.
- Vector rows are keyed per model; switching embedding models requires an **Embed content** re-pass (old rows remain until re-embedded).

## Privacy checklist

- Ollama URL must be loopback (`127.0.0.1`, `::1`, `localhost`); the backend rejects anything else.
- No telemetry, no analytics, no outbound requests of any kind beyond the local Ollama endpoint.
- Extracted text, embeddings, and the error log (opt-in) never leave the device.
