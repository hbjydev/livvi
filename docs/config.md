# Configuration

Livvi is configured entirely through environment variables. This document lists every variable the daemon and supporting crates read, along with their defaults and what each one does.

## Required

| Variable | Purpose |
| -------- | ------- |
| `LIVVI_DISCORD_TOKEN` | Discord bot token. The older `DISCORD_TOKEN` alias is also accepted. |
| `LIVVI_OPENAI_API_KEY` | OpenAI API key. If omitted, the daemon falls back to a mock provider that produces no output. |

## Discord

| Variable | Default | Purpose |
| -------- | ------- | ------- |
| `LIVVI_DISCORD_TOKEN` | — | Discord bot token (see above). |
| `LIVVI_DISCORD_ALLOW_TOOL_USER_IDS` | *(empty)* | Comma-separated list of Discord user IDs allowed to run `/allow tool <tool_name>`. Works in any channel or DM. |

## LLM provider

| Variable | Default | Purpose |
| -------- | ------- | ------- |
| `LIVVI_OPENAI_API_KEY` | — | OpenAI-compatible API key. |
| `LIVVI_OPENAI_API_URL` | `https://api.openai.com/v1` | Base URL for the OpenAI-compatible chat completions endpoint. |
| `LIVVI_OPENAI_MODEL_NAME` | `gpt-4o-mini` | Model name passed to the provider. |

## Memory (Memini)

If both base URL and API key are set, Livvi persists conversation memory through a Memini-compatible server. If either is missing, memory tools become no-ops.

The `livvi-memini` crate also recognises the shorter `MEMINI_*` aliases when using `MeminiMemoryProvider::from_env()` directly, but the daemon uses the `LIVVI_MEMINI_*` forms.

| Variable | Default | Purpose |
| -------- | ------- | ------- |
| `LIVVI_MEMINI_BASE_URL` | — | Memini API base URL. `MEMINI_BASE_URL` or `MEMINI_URL` are also accepted by the Memini crate. |
| `LIVVI_MEMINI_API_KEY` | — | Memini API key. `MEMINI_API_KEY` or `MEMINI_TOKEN` are also accepted by the Memini crate. |
| `LIVVI_MEMINI_NAMESPACE` | `livvi` | Namespace prefix for memory entries. `MEMINI_NAMESPACE` is also accepted by the Memini crate. |

## Web tools

| Variable | Default | Purpose |
| -------- | ------- | ------- |
| `LIVVI_SEARXNG_URL` | — | SearxNG instance URL. If set, `web_search` and `web_fetch` are registered. If empty or unset, both tools are disabled. |

## Storage

| Variable | Default | Purpose |
| -------- | ------- | ------- |
| `LIVVI_DATABASE_URL` | `sqlite:livvi.db?mode=rwc` | SQLite connection string for Livvi's primary store (persons, conversations, tool permissions). |

## Compaction (LCM)

LCM is an experimental long-context-memory compactor. It is disabled by default.

| Variable | Default | Purpose |
| -------- | ------- | ------- |
| `LIVVI_LCM_ENABLE` | *(unset)* | Set to `1` or `true` to enable the LCM compactor. |
| `LIVVI_LCM_DATABASE_URL` | `sqlite:lcm.db?mode=rwc` | SQLite connection string for the LCM store. |
| `LIVVI_LCM_FRESH_TAIL_COUNT` | `6` | Number of recent messages kept verbatim before compaction begins. |
| `LIVVI_LCM_CHUNK_THRESHOLD` | `2000` | Character-count threshold that triggers a new chunk/summary. |
| `LIVVI_LCM_CONDENSATION_COUNT` | `4` | Number of summaries to condense into the next layer. |
| `LIVVI_LCM_MAX_DEPTH` | `3` | Maximum summary hierarchy depth. |

## Agent runtime

| Variable | Default | Purpose |
| -------- | ------- | ------- |
| `LIVVI_AGENT_CONTEXT_LRU_CAPACITY` | `1024` | Maximum number of conversation contexts kept in the in-memory LRU cache. |

## Logging and telemetry

| Variable | Default | Purpose |
| -------- | ------- | ------- |
| `LIVVI_LOG_FORMAT` | *(empty)* | Set to `json` for JSON logs; otherwise logs are emitted in compact/pretty text. |
| `RUST_LOG` | *(empty)* | Standard `tracing`/`env_logger` filter for log levels (e.g., `info,livvi_core=debug`). |

Livvi also exports OpenTelemetry traces over OTLP/HTTP. The service name is hard-coded to `livvi`. Standard OpenTelemetry environment variables apply, such as `OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_EXPORTER_OTLP_HEADERS`.

## Example environment

```bash
# Discord
LIVVI_DISCORD_TOKEN="..."
LIVVI_DISCORD_ALLOW_TOOL_USER_IDS="123456789012345678,876543210987654321"

# OpenAI
LIVVI_OPENAI_API_KEY="sk-..."
LIVVI_OPENAI_MODEL_NAME="gpt-4o"

# Memory
LIVVI_MEMINI_BASE_URL="https://memini.example.com"
LIVVI_MEMINI_API_KEY="memini-..."

# Web tools
LIVVI_SEARXNG_URL="https://search.example.com"

# Storage
LIVVI_DATABASE_URL="sqlite:livvi.db?mode=rwc"

# LCM compaction
LIVVI_LCM_ENABLE="1"
LIVVI_LCM_DATABASE_URL="sqlite:lcm.db?mode=rwc"

# Logging
RUST_LOG="info,livvi_core=debug"
LIVVI_LOG_FORMAT="json"
```
