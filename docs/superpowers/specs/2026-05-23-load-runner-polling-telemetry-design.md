# Load Runner Polling Telemetry Design

## Objetivo

Substituir o stream SSE entre `previa-main` e `previa-runner` por um modelo de
polling controlado pelo main para execucoes de load test distribuidas.

O SSE deve continuar existindo somente entre `previa-main` e o front. O main
continua sendo a fonte de verdade para a UI e para o historico consolidado.

O novo modelo deve suportar testes grandes, com centenas ou milhares de
runners, sem exigir uma conexao SSE longa aberta por runner.

## Motivacao

Hoje o main inicia uma execucao em cada runner com `POST /api/v1/tests/load` e
mantem a resposta aberta como SSE. Em wave load tests, cada runner emite
snapshots agregados em janelas de aproximadamente 1 segundo. Isso evita envio
por request, mas ainda cria uma conexao longa por runner.

Com 1000 runners, o main teria aproximadamente:

- 1000 conexoes SSE runner -> main;
- cerca de 1000 mensagens de metricas por segundo;
- custo de leitura, parse e agregacao em todos os streams;
- risco de backpressure e cancelamento por desconexao;
- baixa capacidade de recuperacao quando uma conexao runner -> main cai.

Em larga escala, o main deve controlar o ritmo de coleta, o paralelismo, os
timeouts e o retry por runner. Polling e mais previsivel para esse papel.

## Decisao

Adotar polling como unico mecanismo de telemetria entre main e runners para
load tests distribuidos.

Manter SSE apenas no trecho main -> front:

```text
front <-- SSE consolidado -- main

main -- POST start ------------------------------> runner
main -- GET telemetry?afterSeq=N&limit=M --------> runner
main -- POST telemetry/ack throughSeq=N ---------> runner
main -- GET status ------------------------------> runner
main -- POST cancel -----------------------------> runner
```

O runner nao deve empurrar telemetria para o main. O runner deve executar em
background, agregar dados em memoria e responder quando o main buscar.

## Nao Objetivos

- Nao persistir request individual completo em disco no runner.
- Nao armazenar corpo de resposta de cada request.
- Nao transformar o runner em banco duravel.
- Nao remover SSE entre main e front.
- Nao mudar o contrato visual da UI alem do necessario para manter o live
  update atual.

## Fluxo De Execucao

1. O front solicita um load test ao `previa-main`.
2. O main calcula o plano de runners.
3. Se o modo Kubernetes estiver ativo, o main cria a reserva no plugin.
4. Quando os runners estiverem prontos, o main inicia a execucao em cada
   runner por `POST /api/v1/tests/load/start`.
5. Cada runner responde rapidamente com:
   - `runnerExecutionId`;
   - estado inicial;
   - configuracao efetivamente aceita;
   - primeiro `nextSeq`, normalmente `1`.
6. O runner executa a carga em background.
7. O runner agrega telemetria em buckets sequenciais.
8. O main roda um loop de polling com paralelismo controlado.
9. Para cada runner, o main busca telemetria com `afterSeq`.
10. O main agrega os buckets recebidos no estado global da execucao.
11. O main atualiza o snapshot consolidado e emite SSE para o front.
12. Depois de agregar/persistir com sucesso, o main envia ACK ao runner.
13. O runner remove do cache somente os buckets confirmados por ACK.
14. No final, o main coleta todos os buckets pendentes, confirma ACK final,
    salva historico e encerra a execucao.

## Contratos HTTP Do Runner

### Iniciar Load Test

```http
POST /api/v1/tests/load/start
content-type: application/json
x-previa-reservation-id: <reservationId>
x-previa-reservation-token: <reservationToken>
x-transaction-id: <transactionId>
```

Body: mesmo payload atual de load test enviado pelo main para o runner.

Resposta:

```json
{
  "runnerExecutionId": "rlexec_...",
  "status": "running",
  "nextSeq": 1,
  "startedAtMs": 1779530400000
}
```

Semantica:

- A resposta deve ser rapida.
- A execucao continua em background depois da resposta.
- O runner deve registrar a execucao em memoria pelo `runnerExecutionId`.
- Se o runner ja estiver ocupado, deve responder `409`.
- Se a reserva for invalida, deve responder `403`.

### Buscar Telemetria

```http
GET /api/v1/tests/load/{runnerExecutionId}/telemetry?afterSeq=123&limit=50
```

Resposta:

```json
{
  "runnerExecutionId": "rlexec_...",
  "status": "running",
  "fromSeq": 124,
  "throughSeq": 130,
  "nextSeq": 131,
  "buckets": [
    {
      "seq": 124,
      "elapsedMs": 124000,
      "event": "metrics",
      "payload": {
        "totalStarted": 1000,
        "totalSent": 995,
        "totalSuccess": 980,
        "totalError": 15,
        "httpStarted": 1000,
        "httpCompleted": 995,
        "statusCodeBuckets": [
          { "elapsedMs": 124000, "code": "200", "count": 970 },
          { "elapsedMs": 124000, "code": "409", "count": 10 },
          { "elapsedMs": 124000, "code": "network_error", "count": 15 }
        ]
      }
    }
  ]
}
```

Semantica:

- `afterSeq` e inclusivo do ponto ja consumido pelo main. O runner deve
  retornar buckets com `seq > afterSeq`.
- `limit` limita a quantidade maxima de buckets retornados.
- `GET telemetry` nunca remove dados.
- Se nao houver buckets novos, o runner retorna `buckets: []` com o estado
  atual.
- Se a execucao terminou, `status` deve ser `completed` ou `cancelled`. A v0
  carrega o snapshot final como um bucket `event: "complete"` com o mesmo
  formato de payload ja emitido pelo SSE legado do runner.

### Confirmar Consumo

```http
POST /api/v1/tests/load/{runnerExecutionId}/telemetry/ack
content-type: application/json
```

Body:

```json
{
  "throughSeq": 130
}
```

Resposta:

```json
{
  "runnerExecutionId": "rlexec_...",
  "ackedThroughSeq": 130,
  "retainedFromSeq": 131
}
```

Semantica:

- O runner so pode remover buckets com `seq <= throughSeq` depois de receber
  ACK.
- ACK deve ser idempotente.
- ACK menor que o ja confirmado nao deve falhar; deve retornar o estado atual.
- ACK maior que o maior bucket conhecido deve confirmar ate o maior bucket
  conhecido e retornar esse valor.

### Consultar Status

```http
GET /api/v1/tests/load/{runnerExecutionId}/status
```

Resposta v0:

```json
{
  "runnerExecutionId": "rlexec_...",
  "status": "running",
  "terminal": false,
  "nextSeq": 131,
  "throughSeq": 130
}
```

### Cancelar Execucao

```http
POST /api/v1/tests/load/{runnerExecutionId}/cancel
```

Semantica:

- Deve cancelar a execucao em background.
- Deve manter os buckets ja gerados ate ACK ou TTL.
- A v0 retorna `200` com `{ "runnerExecutionId": "...", "status": "cancelled" }`.
- Deve ser idempotente quando a execucao ja terminou.

## Modelo De Sequencia

Cada runner mantem uma sequencia monotonica por execucao:

- primeira janela agregada: `seq = 1`;
- proxima janela: `seq = 2`;
- assim por diante ate a finalizacao.

O main guarda, por runner:

- `runnerExecutionId`;
- `lastSeenSeq`;
- `ackedThroughSeq`;
- `status`;
- `lastPollAt`;
- `nextPollAt`;
- `consecutiveFailures`;
- `lastError`;
- endpoint do runner.

O main nunca deve assumir que polling e ACK acontecem na mesma tentativa. A
coleta e a confirmacao sao etapas separadas para evitar perda de dados.

## Cache No Runner

O runner deve manter em memoria:

- buckets agregados ainda nao confirmados;
- estado acumulado da execucao;
- resultado final;
- amostras de erro deduplicadas;
- metadados de TTL.

O runner nao deve manter:

- request individual;
- corpo de resposta individual;
- payload bruto de cada tentativa;
- historico em disco.

### Limites

Configuracoes propostas para evolucao:

```text
PREVIA_LOAD_TELEMETRY_WINDOW_MS=1000
PREVIA_LOAD_TELEMETRY_RETENTION_SECONDS=300
PREVIA_LOAD_TELEMETRY_MAX_RETAINED_BUCKETS=600
PREVIA_LOAD_TELEMETRY_MAX_BUCKETS_PER_POLL=50
PREVIA_LOAD_EXECUTION_TTL_SECONDS=600
```

Se o main parar de coletar por muito tempo e o buffer atingir o limite, o
runner deve:

1. manter o resultado acumulado;
2. marcar lacuna com `droppedBucketCount`;
3. retornar erro ou warning no status;
4. continuar a execucao enquanto a politica permitir.

Essa situacao deve aparecer no historico como teste com telemetria parcial.

## Polling No Main

O main deve usar um scheduler de polling por execucao de load test.

Configuracoes v0/evolucao:

```text
PREVIA_RUNNER_LOAD_POLL_INTERVAL_MS=1000
PREVIA_RUNNER_LOAD_POLL_CONCURRENCY=100
PREVIA_RUNNER_TELEMETRY_POLL_TIMEOUT_MS=2000
PREVIA_RUNNER_TELEMETRY_POLL_LIMIT=512
PREVIA_RUNNER_TELEMETRY_MAX_CONSECUTIVE_FAILURES=30
```

Regras:

- O main deve limitar polls simultaneos para nao derrubar a si mesmo.
- Um runner lento nao pode bloquear coleta dos demais.
- Falhas temporarias devem gerar retry com backoff.
- O main deve continuar enviando SSE para o front com dados parciais.
- O main deve salvar o historico final depois de coletar o final de todos os
  runners ou depois de esgotar a politica de falha.

## Consolidacao No Main

O main deve reaproveitar a agregacao atual de `LoadTelemetryState` sempre que
possivel, trocando a origem dos dados:

- antes: linhas vindas de SSE runner -> main;
- depois: buckets vindos de polling runner -> main.

Na v0, cada bucket recebido encapsula um evento legado do runner:

```json
{
  "seq": 10,
  "event": "metrics",
  "elapsedMs": 9000,
  "payload": {}
}
```

Isso permite trocar o transporte sem reescrever a consolidacao inteira nesta
fase.

Para cada bucket recebido, o main deve:

1. validar `runnerExecutionId`;
2. ignorar buckets com `seq <= lastSeenSeq`;
3. aplicar os buckets no estado consolidado;
4. atualizar `lastSeenSeq`;
5. persistir ou atualizar snapshot em memoria;
6. emitir SSE consolidado para o front;
7. enviar ACK para o runner.

Se o ACK falhar depois da agregacao, o main pode receber os mesmos buckets de
novo no proximo poll. Por isso a aplicacao no main deve ser idempotente por
`runnerExecutionId + seq`.

## Estados Da Execucao No Runner

- `starting`: execucao aceita, ainda inicializando.
- `running`: carga em andamento.
- `draining`: carga terminou, ainda aguardando requests em voo ou grace period.
- `completed`: terminou com sucesso operacional.
- `failed`: falhou por erro interno do runner.
- `cancelled`: cancelada pelo main.
- `expired`: removida por TTL antes do main confirmar tudo.

Status HTTP da aplicacao alvo nao deve automaticamente virar `failed`. Eles
devem ser reportados em `statusCodeBuckets` e erro de assertion/amostra quando
aplicavel.

## Falhas E Recuperacao

### Main Cai E Volta

Se o main reiniciar durante o teste:

- a execucao em runner continua ate terminar ou ate TTL;
- o novo main precisa conseguir descobrir ou reconstruir as execucoes ativas;
- em v0, isso pode depender do estado em memoria do main e da execucao atual;
- em evolucao futura, o main deve persistir `runnerExecutionId` e offsets para
  recuperar polling apos restart.

Para a primeira versao, e aceitavel garantir recuperacao contra perda de rede
main-runner sem prometer recuperacao completa de restart do main.

### Runner Cai

Se um runner cair:

- o main marca o runner como falho depois da politica de retry;
- dados nao coletados daquele runner podem ser perdidos;
- dados ja coletados e confirmados permanecem no main;
- a execucao final deve indicar telemetria parcial.

### ACK Perdido

Se o main agregou os buckets mas o ACK falhou:

- o runner mantem os buckets;
- o main recebe novamente no proximo poll;
- o main ignora por `seq` ja aplicado;
- o main tenta ACK novamente.

### Poll Perdido

Se o poll falhar:

- runner continua executando;
- buckets ficam retidos;
- main tenta novamente com backoff.

## Relacao Com Kubernetes Plugin

O plugin Kubernetes continua responsavel por:

- reservar runners;
- entregar endpoints;
- rearmar runners reutilizaveis;
- limpar runners ociosos;
- preservar runners ocupados.

O novo modelo de telemetria nao muda o contrato de reserva. Ele muda somente o
contrato de execucao/coleta entre main e runner.

Para runners reaproveitados, o runner deve limpar estado de execucoes antigas
confirmadas antes de aceitar uma nova execucao.

## Compatibilidade E Migracao

### Fase 1

Adicionar novos endpoints de start/poll/ack/cancel no runner, mantendo o
endpoint SSE antigo para comparacao e testes.

### Fase 2

Alterar o main para usar polling em load tests distribuidos.

### Fase 3

Manter SSE apenas no main -> front e remover o caminho runner -> main SSE para
load test.

### Fase 4

Opcionalmente remover o classic load test por request ou adapta-lo ao mesmo
modelo de execucao em background com polling.

## Observabilidade

O main deve expor/registrar:

- quantidade de runners sendo pollados;
- polls por segundo;
- latencia de poll;
- falhas consecutivas por runner;
- quantidade de buckets coletados;
- quantidade de buckets duplicados ignorados;
- quantidade de ACKs pendentes;
- lag entre bucket gerado e bucket coletado;
- runners com buffer proximo do limite.

O runner deve expor/registrar:

- execucoes ativas;
- buckets retidos;
- `ackedThroughSeq`;
- bytes de telemetria retidos;
- buckets descartados por limite;
- ultima coleta;
- ultimo ACK.

## Criterios De Aceite

- Um teste com 1000 runners nao abre 1000 conexoes SSE runner -> main.
- O unico SSE do caminho de load test e main -> front.
- O main controla o paralelismo de polling.
- Runner continua executando quando um poll falha.
- Runner nao limpa dados no GET.
- Runner limpa dados somente apos ACK.
- ACK e idempotente.
- Buckets duplicados nao duplicam metricas no main.
- O front continua recebendo atualizacao ao vivo pelo main.
- O historico final inclui dados consolidados e indica telemetria parcial se
  algum runner falhar ou perder buckets.

## Questoes Em Aberto

- Qual deve ser o limite inicial de `PREVIA_RUNNER_TELEMETRY_POLL_CONCURRENCY`
  em ambientes pequenos e grandes?
- A primeira versao deve persistir offsets no banco do main ou manter apenas em
  memoria?
- O runner deve permitir uma nova execucao enquanto ainda retem buckets
  confirmados/parcialmente confirmados de uma execucao antiga?
- O classic load test deve ser migrado junto ou removido do caminho principal?
