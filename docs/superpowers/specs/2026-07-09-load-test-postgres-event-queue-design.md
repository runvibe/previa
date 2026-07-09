# Load Test Postgres Event Queue Design

## Objetivo

Mudar o canal de telemetria de load test entre `previa-runner` e
`previa-main` para uma fila duravel em Postgres.

O `previa-main` continua iniciando diretamente cada runner por HTTP. O runner
executa a carga, grava eventos e resultados no Postgres, e o main le esses
eventos do banco para consolidar metricas, atualizar SSE para o usuario e salvar
o historico final.

Fluxo alvo:

```text
front <-- SSE consolidado -- previa-main

previa-main -- POST start/cancel -------------> previa-runner
previa-runner -- INSERT eventos/resultados ---> Postgres
previa-main <-- SELECT eventos novos ---------- Postgres
```

## Motivacao

Hoje o load test usa comunicacao direta para telemetria runner -> main. A versao
atual ja removeu o SSE longo por runner e usa polling HTTP em
`/telemetry`/`/ack`, mas o estado vivo ainda fica em memoria no runner e precisa
ser coletado pelo main durante a execucao.

Para execucoes maiores e para ambientes distribuidos, o canal de telemetria deve
ser mais duravel:

- o main deve poder retomar leitura a partir do ultimo evento processado;
- o runner nao deve precisar manter todos os buckets vivos em memoria ate o main
  buscar;
- quedas temporarias do main nao devem descartar telemetria ja produzida;
- consolidacao e historico devem ser derivados de uma fonte unica de eventos.

## Decisao

Adotar Postgres como event queue append-only para load tests.

O main continua responsavel por:

- validar a requisicao do usuario;
- selecionar runners saudaveis;
- calcular o plano de carga por runner;
- iniciar cada runner por `POST /api/v1/tests/load/start`;
- cancelar runners por HTTP quando solicitado;
- consolidar eventos vindos do Postgres;
- emitir SSE consolidado para UI/MCP;
- persistir `load_history` no final.

O runner passa a ser responsavel por:

- receber o `orchestratorExecutionId` enviado pelo main;
- executar a carga em background;
- gravar eventos de telemetria no Postgres;
- gravar status terminal da sua execucao no Postgres;
- manter os endpoints HTTP de start/status/cancel para controle direto pelo
  main.

O endpoint HTTP de telemetria do runner deve permanecer como fallback durante a
primeira migracao, mas o caminho preferido para load test passa a ser o
Postgres.

## Nao Objetivos

- Nao mudar E2E nesta fase.
- Nao transformar runners em consumidores de jobs do Postgres.
- Nao remover o start/cancel HTTP direto entre main e runner.
- Nao remover SSE entre main e front.
- Nao gravar request/response completo de cada chamada HTTP feita pelo load
  test.
- Nao exigir `LISTEN/NOTIFY` na primeira versao.
- Nao mudar a experiencia visual de load test alem do necessario para manter os
  updates vivos.

## Modelo De Dados

### `load_execution_runner_runs`

Uma linha por runner usado em uma execucao de load test.

Campos:

- `execution_id`: ID da execucao no main.
- `runner_execution_id`: ID local retornado pelo runner.
- `runner_endpoint`: endpoint do runner usado pelo main.
- `runner_node`: identificador opcional do pod/no quando disponivel.
- `status`: `starting`, `running`, `completed`, `failed` ou `cancelled`.
- `started_at_ms`: timestamp em milissegundos.
- `finished_at_ms`: timestamp terminal opcional.
- `last_seq`: maior sequencia gravada por esse runner.
- `last_error`: erro terminal ou ultimo erro relevante.
- `created_at`, `updated_at`.

Indice/constraint:

- chave unica em `(execution_id, runner_execution_id)`.
- indice em `(execution_id, status)`.

### `load_execution_events`

Fila append-only de eventos emitidos por runner.

Campos:

- `id`: UUID/ULID/v7 gerado no insert.
- `execution_id`: ID da execucao no main.
- `runner_execution_id`: ID local do runner.
- `runner_endpoint`: endpoint do runner.
- `seq`: sequencia monotonica por runner dentro da execucao.
- `event`: nome do evento, como `metrics`, `complete`, `error`.
- `elapsed_ms`: tempo desde o inicio da execucao local do runner.
- `payload_json`: payload JSON serializado como texto para manter o acesso via
  `sqlx::Any` simples onde possivel.
- `created_at_ms`: timestamp em milissegundos.
- `consumed_at_ms`: preenchido pelo main depois de consolidar com sucesso.

Indice/constraint:

- chave unica em `(execution_id, runner_execution_id, seq)`.
- indice em `(execution_id, consumed_at_ms, runner_execution_id, seq)`.
- indice em `(execution_id, runner_execution_id, seq)`.

`consumed_at_ms` nao remove dados. Ele marca que o main ja incorporou o evento
ao snapshot vivo. A limpeza fisica deve ser um processo separado por retencao.

## Contrato De Start Do Runner

O main passa a enviar um identificador de execucao do orquestrador no body do
start:

```json
{
  "orchestratorExecutionId": "exec_...",
  "pipeline": {},
  "selectedBaseUrlKey": "hml",
  "selectedEnvGroupSlug": "hml",
  "specs": [],
  "envGroups": [],
  "load": {}
}
```

Resposta:

```json
{
  "runnerExecutionId": "rlexec_...",
  "status": "running",
  "startedAtMs": 1779530400000,
  "telemetrySource": "postgres"
}
```

Semantica:

- `orchestratorExecutionId` e obrigatorio quando o modo Postgres esta ativo.
- O runner usa esse ID como `execution_id` em todas as escritas no banco.
- O runner ainda pode gerar seu proprio `runnerExecutionId` para controle local
  e cancelamento HTTP.
- Se o runner nao conseguir abrir conexao com Postgres ou registrar a execucao,
  deve responder erro e nao iniciar a carga.

## Configuracao

### Main

Novas variaveis:

- `LOAD_TELEMETRY_SOURCE=postgres|runner_polling`.
- `LOAD_EVENT_POLL_INTERVAL_MS`, default `250`.
- `LOAD_EVENT_POLL_LIMIT`, default `500`.
- `LOAD_EVENT_RETENTION_HOURS`, default `24`.

Quando `LOAD_TELEMETRY_SOURCE=postgres`, o main usa seu `DATABASE_URL` atual
como fonte de leitura dos eventos.

### Runner

Novas variaveis:

- `RUNNER_LOAD_EVENT_DATABASE_URL`: URL Postgres com permissao de insert/update
  nas tabelas de telemetria.
- `RUNNER_LOAD_EVENT_FLUSH_INTERVAL_MS`, default `250`.
- `RUNNER_LOAD_EVENT_BATCH_SIZE`, default `200`.

O runner nao deve receber credencial administrativa do banco. A permissao deve
ser limitada as tabelas de telemetria de load test.

## Escrita No Runner

O runner deve manter um buffer pequeno em memoria e fazer flush em lote para o
Postgres.

Regras:

1. Cada evento recebe `seq` crescente por `runnerExecutionId`.
2. Inserts devem ser idempotentes via constraint unica
   `(execution_id, runner_execution_id, seq)`.
3. Em falha temporaria de escrita, o runner deve fazer retry com backoff curto.
4. Se o buffer em memoria atingir limite maximo, o runner deve registrar erro
   terminal e cancelar a execucao para evitar uso ilimitado de memoria.
5. Evento terminal `complete`, `failed` ou `cancelled` deve ser gravado antes do
   runner marcar a run como terminal.

Eventos de metrica devem manter o payload compativel com o formato atual de
`RunnerLoadLine`, para que a consolidacao existente possa ser reaproveitada.

## Leitura E Consolidacao No Main

O main cria uma tarefa de leitura por execucao de load test.

Loop:

1. Buscar eventos nao consumidos da execucao ordenados por
   `runner_execution_id, seq`.
2. Converter cada linha em `RunnerLoadLine`.
3. Reaproveitar a consolidacao atual:
   - `apply_runner_telemetry_line`;
   - acumuladores de latencia;
   - amostras de erro;
   - snapshot consolidado.
4. Atualizar `snapshot_payload`.
5. Emitir SSE consolidado para o usuario.
6. Marcar eventos processados com `consumed_at_ms`.
7. Encerrar quando todos os runners da execucao estiverem em estado terminal e
   nao houver eventos novos pendentes.

O main deve continuar salvando `load_history` no formato atual. O Postgres event
queue e fonte viva/duravel de consolidacao, nao substitui o historico final
existente.

## Cancelamento

Cancelamento continua partindo do main:

1. Usuario solicita cancelamento.
2. Main cancela o contexto local da execucao.
3. Main chama `POST /api/v1/tests/load/{runnerExecutionId}/cancel` nos runners
   ja iniciados.
4. Runner grava evento/status `cancelled` no Postgres.
5. Main le esse status/evento no loop de consolidacao e encerra quando todos os
   runners estiverem terminais.

Se o runner nao responder ao cancelamento HTTP, o main deve marcar erro local,
mas ainda deve continuar lendo eventos que ja estejam no Postgres.

## Fallback E Compatibilidade

Durante a primeira versao:

- `LOAD_TELEMETRY_SOURCE=runner_polling` mantem o comportamento atual.
- `LOAD_TELEMETRY_SOURCE=postgres` ativa o novo caminho.
- Se o main estiver em SQLite, o modo Postgres deve falhar cedo com erro claro,
  porque runners remotos nao conseguem compartilhar um arquivo SQLite como fila
  duravel.
- Os endpoints `/telemetry` e `/telemetry/ack` podem permanecer no runner para
  rollback operacional.

Quando o caminho Postgres estiver validado em producao, a manutencao do polling
HTTP pode ser reavaliada.

## Erros E Recuperacao

### Main reinicia durante execucao

A primeira versao nao precisa retomar automaticamente uma execucao viva apos
restart do main. Ela deve, no minimo, conseguir consultar eventos ja gravados e
marcar a execucao como interrompida ou falha ao detectar runs nao terminais
antigas.

### Runner reinicia durante execucao

O runner perde o trabalho em andamento, mas eventos ja gravados permanecem no
Postgres. Ao detectar ausencia de status terminal, o main deve terminar a
execucao como `failed` apos timeout configuravel.

### Postgres indisponivel para runner

O runner nao deve iniciar load test se nao conseguir registrar a run no banco.
Durante a execucao, falhas repetidas de flush devem cancelar a execucao e gravar
erro terminal quando o banco voltar dentro da janela de retry. Se o banco nao
voltar, o main encerra por timeout/falha.

### Eventos duplicados

Duplicatas sao tratadas pela chave unica `(execution_id, runner_execution_id,
seq)`. O main deve processar eventos de forma idempotente, marcando consumo
somente depois da consolidacao local ter sido aplicada.

## Retencao

Eventos de load test podem crescer rapidamente. A retencao deve ser explicita.

Primeira versao:

- manter eventos por pelo menos `LOAD_EVENT_RETENTION_HOURS`;
- nunca remover eventos de execucoes nao terminais;
- remover apenas eventos cujo `created_at_ms` seja mais antigo que a janela de
  retencao e cuja execucao tenha historico final salvo.

## Testes

### Unitarios

- geracao de sequencia por runner;
- batch insert idempotente;
- leitura ordenada por runner/seq;
- conversao de linha SQL para `RunnerLoadLine`;
- consolidacao a partir de eventos Postgres usando os acumuladores atuais;
- comportamento de cancelamento com eventos pendentes;
- erro claro quando modo Postgres e usado sem banco Postgres.

### Integracao

- start de load test em modo Postgres com runner fake gravando eventos;
- main consolida eventos do banco e emite SSE;
- evento terminal encerra execucao e salva `load_history`;
- duplicata de evento nao duplica metricas consolidadas;
- restart simulado do loop de leitura retoma a partir de eventos nao consumidos.

### Regressao

- `LOAD_TELEMETRY_SOURCE=runner_polling` continua passando com o caminho atual;
- E2E continua inalterado;
- load test classico e wave load test continuam usando o mesmo historico final.

## Plano De Migracao

1. Adicionar schema de telemetria de load test.
2. Adicionar `orchestratorExecutionId` ao contrato de start do runner.
3. Implementar escritor Postgres no runner, atras de feature/config.
4. Implementar leitor/consolidador Postgres no main, atras de
   `LOAD_TELEMETRY_SOURCE=postgres`.
5. Manter polling HTTP como fallback.
6. Validar localmente com Postgres.
7. Publicar em ambiente controlado com um load test pequeno.
8. Aumentar escala e comparar consolidado Postgres contra o caminho atual.

## Criterios De Aceite

- Em modo Postgres, o main inicia runners diretamente por HTTP.
- Runners gravam eventos de load test no Postgres sem depender de SSE/polling de
  telemetria para o main.
- O main consolida metricas lendo o Postgres e mantem SSE para o usuario.
- `load_history` final permanece compativel com o formato atual.
- O caminho atual de polling HTTP continua disponivel por configuracao.
- Uma execucao com falha de runner ou cancelamento termina com status coerente e
  historico final.
