# `/observe` demo transcript (SigNoz MCP × rem-cli)

Prerequisites: SigNoz stack up (OTLP :4317, MCP :8000), router-agent sample run with OTel, `SIGNOZ_API_KEY` set.

```bash
export SIGNOZ_MCP_URL=http://localhost:8000/mcp
export SIGNOZ_API_KEY=<service-account-key>
# Do NOT set SIGNOZ_URL=http://localhost:… for self-host MCP (server already has it)

# After: run router-agent sample_input/tasks.json with OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4317
```

## Demo 1 — which tasks used Fireworks

```text
$ rem --model qwen2.5:1.5b-instruct observe "which tasks used fireworks and why"
▌ 📡 SigNoz MCP observe: which tasks used fireworks and why
▌ fetched ~12k chars of telemetry context from http://localhost:8000/mcp
…
  (LLM cites stage counts from signoz_aggregate_traces:)
  task.route stages: free_solver=7, fireworks=6, local_llm=5
  llm.fireworks_call spans present for service.name=router-agent
```

Ground-truth from MCP aggregates (not hallucination):

| Span / stage | Count |
|--------------|------:|
| task.classify | 24 |
| task.route | 18 |
| task.process | 14 |
| llm.local_generate | 10 |
| llm.fireworks_call | 6 |
| free_solver | 7 |
| fireworks | 6 |
| local_llm | 5 |

## Demo 2 — slowest task

```text
$ rem observe "show me the slowest task in the last run"
…
  Uses signoz_aggregate_traces p99 by name + task.process roots
```

## Interactive

```text
rem chat
rem> /observe why did task 3 escalate to fireworks
```

## Config reference

See main README § `/observe` and [SigNoz MCP docs](https://signoz.io/docs/ai/signoz-mcp-server/).
