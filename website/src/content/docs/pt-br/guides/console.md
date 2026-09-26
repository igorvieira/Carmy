---
title: Console
description: "Explore e chame tools por uma interface no terminal, ou controle-as como agente via JSON Lines."
sidebar:
  order: 12
---

O console roda as tools da sua aplicação pelo runtime de verdade, com políticas,
validação, idempotência, prazos e tracing. Ele tem duas faces:

- **Para pessoas:** `carmy console` abre uma interface no terminal.
- **Para agentes e scripts:** a aplicação fala `carmy-console/1`, um protocolo JSON
  Lines, pelo stdio.

## Interface no terminal

```console
carmy console
```

![O console do Carmy mostrando uma chamada repetida](/console/tui.png)

Ele compila a aplicação e depois mostra o seguinte:

| área | o que mostra |
|------|--------------|
| **tools** | todas as tools, com um selo de efeito: `N` nenhum, `R` leitura, `W` escrita, `X` escrita externa, `D` destrutivo; 🔒 até a confirmação ser concedida |
| **detalhes** | a descrição, as flags, e os campos de entrada e saída |
| **arguments** | JSON, já preenchido a partir do input schema |
| **request_id** | opcional; uma chamada repetida com o mesmo ID mostra `replayed` |
| **result** | o status, a duração e os dados, ou o erro estruturado com o `suggested_action` |

| tecla | ação |
|-------|------|
| `↑` `↓` / `j` `k` | seleciona uma tool, ou rola o resultado |
| `Tab` / `Shift+Tab` | muda o foco |
| `Enter` | roda a tool selecionada |
| `Ctrl+R` | volta os argumentos para o exemplo do schema |
| `Ctrl+U` | limpa o campo em edição |
| `c` | concede ou revoga `confirm:<tool>` nesta sessão |
| `r` | roda a última chamada de novo |
| `h` | histórico; `Enter` roda de novo uma chamada anterior |
| `?` | ajuda |
| `q`, `Esc`, `Ctrl+C` | sai |

Os argumentos e o `request_id` ficam guardados por tool, porque reusar um `request_id`
em outra tool é um conflito de idempotência.

### Tools destrutivas

Uma tool marcada com 🔒 não roda até ser confirmada. Selecione-a e aperte `c`:

![Modal de confirmação de uma tool destrutiva](/console/confirm.png)

Depois do `y`, o cadeado vira ✓ nesta sessão e a chamada passa:

![A tool destrutiva confirmada e executada](/console/confirmed.png)

A concessão é a mesma permissão `confirm:<tool>` que um host define em produção. Aperte
`c` de novo para revogá-la.

### Histórico

`h` lista todas as chamadas da sessão. `Enter` roda uma delas de novo, com os mesmos
argumentos e o mesmo `request_id`:

![O histórico da sessão](/console/history.png)

## Para agentes: `carmy-console/1`

```console
cargo run -- console          # ou: carmy console --jsonl
```

O `carmy console` também entra nesse modo quando o stdin ou o stdout não é um
terminal. Cada mensagem é um objeto JSON por linha. A primeira linha é:

```json
{"event":"ready","protocol":"carmy-console/1","server":"shop","tools":3}
```

| requisição | resultado |
|------------|-----------|
| `{"id":1,"op":"tools"}` | o catálogo, no mesmo formato do `GET /agent/tools` |
| `{"id":2,"op":"describe","tool":"create_order"}` | os metadados e os schemas da tool |
| `{"id":3,"op":"call","tool":"create_order","arguments":{"sku":"KB-01"},"request_id":"r1"}` | o mesmo corpo do `POST /agent/execute`, mais `replayed` e `duration_ms` |
| `{"id":4,"op":"confirm","tool":"cancel_order"}` | concede `confirm:cancel_order` nesta sessão |
| `{"id":5,"op":"revoke","tool":"cancel_order"}` | retira a concessão |
| `{"id":6,"op":"audit","limit":20}` | os últimos [registros de execução](/pt-br/guides/audit/), mais novos primeiro |
| `{"id":7,"op":"dead","limit":20}` | os [jobs](/pt-br/guides/jobs/) na fila de dead letters |
| `{"id":8,"op":"webhooks"}` | os [webhooks](/pt-br/guides/webhooks/): caminho, tool, modo e pointer de identidade |
| `{"op":"help"}` / `{"op":"exit"}` | lista as operações / encerra a sessão |

Toda resposta repete o `id` e traz `ok`:

```json
{"id":3,"ok":true,"result":{"execution_id":"exec_…","status":"completed","data":{"order_id":1},"_agent":{"cacheable":false,"next_actions":[]},"replayed":false,"duration_ms":0.4}}
{"id":9,"ok":false,"error":{"code":"TOOL_NOT_FOUND","message":"unknown tool `x`","category":"not_found","recoverable":true,"retryable":false,"suggested_action":"tools"}}
```

- **Uma execução que falhou continua com `ok: true`.** O erro da própria execução
  vem em `result.error`, exatamente como no HTTP. `ok: false` quer dizer que a
  requisição em si estava errada: `INVALID_REQUEST`, `UNKNOWN_OP` ou `TOOL_NOT_FOUND`.
- **Comandos em texto:** uma linha que não começa com `{` é lida como comando, o que
  ajuda no terminal: `tools`, `describe <tool>`, `confirm <tool>`, `revoke <tool>`,
  `help`, `exit`, ou `<tool> [json] [--request-id <id>]`. As respostas continuam em
  JSON.
- **A sessão:** ela roda como o principal `console`, com uma sessão própria, então os
  replays por `request_id` funcionam entre chamadas. Quem roda o processo é o operador
  confiável, então as concessões de `confirm` valem só para esta sessão.
- **Logs:** vão para o stderr, em `warn` por padrão. O stdout leva só o protocolo.

## Embutindo

`carmy::console::serve(runtime, name, input, output)` serve o protocolo sobre qualquer
leitor e escritor assíncronos, o que é útil em testes.
