# Postgres Execution Queue Design

## Status

Esta especificação substitui
`docs/superpowers/specs/2026-07-09-load-test-postgres-event-queue-design.md`.

O desenho anterior movia somente a telemetria de load test para o Postgres e
mantinha `start`, `status` e `cancel` por HTTP. Este desenho estabelece o
Postgres como o canal completo de trabalho entre `previa-main` e
`previa-runner`.

## Objetivo

Substituir a comunicação direta de execução entre `previa-main` e os runners por
uma fila durável no Postgres.

A `previa-main` cria execuções e jobs. Runners elegíveis reivindicam jobs,
renovam leases, publicam eventos e gravam resultados no Postgres. A `main`
projeta esses eventos, mantém o histórico final e continua entregando SSE para
UI, API e MCP.

Fluxo alvo:

```text
cliente
  |
  | HTTP + SSE
  v
previa-main
  |
  | INSERT execution/jobs + LISTEN/NOTIFY
  v
Postgres
  ^
  | claim, heartbeat, lease, events, result
  |
previa-runner
```

O Postgres passa a ser obrigatório para toda persistência operacional do
Previa. SQLite permanece somente como formato portátil de importação e
exportação de projetos.

## Motivação

O transporte atual depende de chamadas HTTP da `main` para endpoints de cada
runner. Em load tests, a `main` inicia o trabalho por HTTP e coleta telemetria
por polling em `/telemetry`, confirmando consumo em `/telemetry/ack`. Em E2E, a
execução também depende de conexão direta com um runner selecionado.

Esse modelo cria acoplamentos operacionais:

- a `main` precisa conhecer e alcançar endpoints individuais de runners;
- uma interrupção da `main` pode perder estado vivo mantido apenas no runner;
- a retomada depende de polling e estado em memória;
- controle de capacidade, claim, retry e fencing ficam divididos entre
  processos;
- runners provisionados dinamicamente precisam ser descobertos antes do
  despacho;
- observabilidade e histórico são derivados de diferentes fontes.

A fila Postgres torna o trabalho reivindicável, durável e auditável, além de
permitir que runners entrem e saiam sem que a `main` roteie chamadas para
endereços individuais.

## Decisões

1. Postgres é obrigatório no runtime.
2. SQLite é usado somente em importação e exportação.
3. Runners são workers concorrentes e reivindicam jobs compatíveis.
4. A fila usa tabelas relacionais, state machines e leases explícitos.
5. Claims usam `FOR UPDATE SKIP LOCKED`.
6. A entrega é `at-least-once`, com fencing e operações internas idempotentes.
7. `LISTEN/NOTIFY` reduz latência, mas polling periódico garante recuperação.
8. E2E usa um job indivisível por execução.
9. Load tests usam uma execução lógica dividida em múltiplos shards.
10. A `main` não seleciona um runner específico para cada job.
11. HTTP no runner fica restrito a `/health`, `/ready` e `/info`.
12. A mudança é incompatível: `main` e runners devem usar a mesma versão de
    protocolo.
13. Não haverá transporte HTTP de execução como fallback.

## Não Objetivos

- Não adicionar Kafka, RabbitMQ, Redis ou outro broker.
- Não implementar exactly-once para efeitos produzidos na API testada.
- Não emitir um evento Postgres para cada request de load test.
- Não manter SQLite como banco operacional alternativo.
- Não preservar compatibilidade com runners antigos.
- Não transformar `LISTEN/NOTIFY` em fonte de verdade.
- Não alterar o contrato externo da API de projetos além do necessário para
  expor os novos estados.
- Não mover o SSE entre `previa-main` e clientes para o Postgres.
- Não substituir o histórico final por um replay permanente de todos os
  eventos.

## Arquitetura

### `previa-main`

A `main` continua responsável por:

- autenticar e autorizar o cliente;
- validar projetos, specs, env groups e pipelines;
- criar a execução lógica;
- calcular quantidade e configuração de shards de load;
- inserir execução e jobs na mesma transação;
- solicitar capacidade ao plugin Kubernetes quando necessário;
- marcar cancelamento desejado;
- projetar eventos em snapshots vivos;
- emitir SSE para UI, API e MCP;
- persistir `integration_history` e `load_history`;
- recuperar leases expirados;
- aplicar retenção;
- expor observabilidade da fila.

A `main` não:

- escolhe um endpoint de runner para o job;
- inicia, consulta ou cancela execução por HTTP no runner;
- coleta telemetria diretamente de um runner;
- usa estado somente em memória como fonte de verdade.

### `previa-runner`

Cada runner:

- registra uma instância no Postgres;
- publica versão, pool, labels, capabilities e capacidade;
- mantém heartbeat;
- aguarda notificação de jobs e executa polling de segurança;
- reivindica atomicamente um job compatível;
- renova o lease durante a execução;
- executa E2E ou um shard de load;
- grava eventos em lotes;
- observa cancelamento no Postgres;
- conclui, falha ou cancela o job com fencing;
- interrompe trabalho quando deixa de conseguir renovar o lease.

O runner não:

- recebe jobs por HTTP;
- consulta tabelas administrativas;
- escolhe jobs sem respeitar pool, capabilities e slots;
- continua executando indefinidamente sem lease válido.

### Plugin Kubernetes

O plugin Kubernetes continua sendo um componente de provisionamento, não um
transporte de execução.

A `main` pode solicitar capacidade ao plugin. O plugin cria runners com a
credencial restrita da fila. Cada runner registra-se no Postgres e passa a
competir por jobs compatíveis. A `main` não precisa aguardar nem descobrir um
endpoint HTTP individual antes do claim.

## Fluxos

### Criação de execução E2E

1. O cliente chama a API da `main`.
2. A `main` valida acesso, pipeline e configuração.
3. A `main` cria uma linha em `executions`.
4. A `main` cria um job E2E em `execution_jobs`.
5. A mesma transação executa `pg_notify`; o Postgres confirma execução e job e
   entrega a notificação somente depois do commit.
7. Um runner compatível reivindica o job.
8. O runner publica `leased`, `running`, eventos de passo e estado terminal.
9. A `main` projeta eventos e atualiza SSE.
10. Ao término, a `main` grava `integration_history`.

Uma nova tentativa E2E reinicia a pipeline. O runtime expõe `executionId`,
`jobId` e `attempt` para que fixtures possam produzir identificadores únicos.
Efeitos externos provocados por uma tentativa anterior não são revertidos pelo
Previa.

### Criação de load test

1. A `main` valida a wave, capacidade desejada e runners elegíveis conhecidos.
2. A `main` cria uma execução lógica com relógio global.
3. A `main` divide a carga em shards.
4. Cada shard recebe RPS, duração, wave profile e requisitos.
5. Todos os jobs são gravados na mesma transação da execução.
6. Runners elegíveis reivindicam shards de forma concorrente.
7. Cada runner publica buckets agregados e estado terminal.
8. A `main` consolida shards no snapshot da execução.
9. O histórico final mantém o contrato atual de load test.

Um shard retomado usa o relógio global da execução. Slots cujo deadline já
passou são contabilizados como não executados conforme a telemetria existente,
mas nunca são reproduzidos. A retomada executa somente a parte restante da
wave.

### Cancelamento

1. O cliente solicita cancelamento à `main`.
2. A `main` altera a execução para `cancel_requested`.
3. Jobs ainda não reivindicados tornam-se `cancelled`.
4. A `main` publica `NOTIFY previa_control`.
5. Runners com jobs ativos observam o estado por notificação ou polling.
6. Cada runner interrompe o trabalho e grava seu evento terminal.
7. Quando todos os jobs estão terminais, a execução torna-se `cancelled`.

Cancelamento não depende de um runner responder a uma chamada HTTP.

### Falha de runner

1. O heartbeat deixa de avançar.
2. A instância passa a `stale`.
3. O lease do job expira.
4. O reaper move o job para `retry_wait` ou `dead_letter`.
5. Quando `available_at` chega, o job volta para `queued`.
6. Outro runner pode reivindicá-lo com novo `lease_epoch`.
7. Escritas atrasadas do runner anterior são rejeitadas.

O retry usa backoff exponencial determinístico:

```text
delay_ms = min(
  PREVIA_QUEUE_RETRY_BACKOFF_BASE_MS * 2^(attempt - 1),
  PREVIA_QUEUE_RETRY_BACKOFF_MAX_MS
)
```

Não há jitter na primeira versão, para que estado, testes e diagnóstico sejam
reproduzíveis.

### Reinício da `main`

Snapshots, checkpoints e eventos permanecem no Postgres. Uma instância da
`main` reivindica o lease de projeção e continua depois de `last_event_id`.
Clientes conectados novamente recebem o snapshot persistido e continuam o SSE.

## Modelo de Dados

Tipos de estado podem ser `TEXT` com constraints explícitas para facilitar
migrations. As transições devem ser feitas por funções ou queries centralizadas,
nunca por updates arbitrários espalhados pelo código.

### `queue_protocol`

Uma linha identifica o contrato compartilhado:

- `id`: constante `1`;
- `protocol_version`: inteiro;
- `updated_at`.

O número esperado é compilado na `main` e no runner. Divergência faz o processo
falhar no startup com mensagem de incompatibilidade.

### `runner_instances`

Uma linha por processo de runner:

- `id`: UUID v7;
- `name`: nome operacional;
- `session_token_hash`: hash do token opaco emitido no registro;
- `pool`: pool de capacidade;
- `protocol_version`;
- `version`;
- `capabilities_json`: capabilities adicionais;
- `labels_json`: labels para matching;
- `max_e2e_slots`;
- `max_load_slots`;
- `heartbeat_interval_ms`;
- `status`: `ready`, `busy`, `draining`, `stale` ou `stopped`;
- `last_heartbeat_at`;
- `registered_at`;
- `updated_at`.

Índices:

- `(status, pool, last_heartbeat_at)`;
- `(protocol_version, status)`;
- GIN em `labels_json` somente se os requisitos reais justificarem a busca.

Slots utilizados são derivados de jobs ativos associados ao runner. Contadores
denormalizados podem ser introduzidos apenas se medições demonstrarem
necessidade.

O registro devolve um `runner_session_token` opaco. Chamadas posteriores exigem
`runner_id` e esse token. O token não é persistido em texto puro nem aparece em
logs.

### `executions`

Representa uma solicitação lógica:

- `id`: UUID v7;
- `project_id`;
- `pipeline_id`, opcional;
- `kind`: `e2e` ou `load`;
- `status`: `queued`, `running`, `cancel_requested`, `completed`, `failed` ou
  `cancelled`;
- `desired_status`: `running` ou `cancelled`;
- `request_json`: snapshot imutável do pedido validado;
- `shard_count`;
- `max_attempts`;
- `created_by`;
- `transaction_id`, opcional;
- `queued_at`;
- `started_at`;
- `finished_at`;
- `created_at`;
- `updated_at`.

Índices:

- `(project_id, created_at DESC)`;
- `(status, queued_at)`;
- `(kind, status, created_at)`.

### `execution_jobs`

Unidade de trabalho reivindicável:

- `id`: UUID v7;
- `execution_id`;
- `kind`: `e2e` ou `load`;
- `shard_index`, opcional para E2E;
- `pool`;
- `requirements_json`;
- `payload_json`;
- `priority`;
- `status`: `queued`, `leased`, `running`, `retry_wait`, `completed`, `failed`,
  `cancelled` ou `dead_letter`;
- `available_at`;
- `attempt`;
- `max_attempts`;
- `runner_id`, opcional;
- `lease_epoch`;
- `lease_token`, opcional;
- `lease_expires_at`, opcional;
- `started_at`, opcional;
- `finished_at`, opcional;
- `result_json`, opcional;
- `last_error`, opcional;
- `created_at`;
- `updated_at`.

Constraints:

- chave única `(execution_id, shard_index)` para load;
- somente um job E2E por execução;
- `attempt <= max_attempts`;
- lease fields presentes apenas em estados ativos;
- resultado terminal imutável depois de aceito.

Índices:

- `(status, available_at, priority DESC, created_at)`;
- `(pool, kind, status, available_at)`;
- `(runner_id, status)`;
- `(execution_id, status)`;
- `(lease_expires_at)` para jobs ativos.

### `execution_events`

Log append-only:

- `id`: `BIGINT GENERATED ALWAYS AS IDENTITY`;
- `execution_id`;
- `job_id`;
- `runner_id`;
- `attempt`;
- `lease_epoch`;
- `seq`;
- `event_type`;
- `elapsed_ms`;
- `payload_json`: `JSONB`;
- `created_at`.

Constraints:

- chave única `(job_id, attempt, seq)`;
- evento aceito somente para o runner e fencing atuais;
- `seq` crescente dentro da tentativa;
- payload limitado a `1 MiB`, constante do protocolo validada antes do insert.

Índices:

- `(execution_id, id)`;
- `(job_id, attempt, seq)`;
- `(execution_id, event_type, id)`.

Eventos de load são buckets agregados. Request e response completos não são
gravados por chamada.

### `execution_snapshots`

Projeção persistida:

- `execution_id`: chave primária;
- `version`;
- `last_event_id`;
- `status`;
- `snapshot_json`;
- `projection_owner`, opcional;
- `projection_lease_epoch`;
- `projection_lease_expires_at`, opcional;
- `updated_at`.

O checkpoint e o snapshot são atualizados na mesma transação. Somente o
proprietário do lease de projeção pode avançar `last_event_id`.

### Históricos

`integration_history` e `load_history` permanecem contratos finais. A projeção
terminal grava o histórico uma única vez usando `execution_id` como chave de
idempotência.

Eventos podem ser removidos após retenção somente quando:

- a execução está terminal;
- o snapshot contém todos os eventos;
- o histórico final foi persistido;
- a idade excedeu a configuração.

## State Machines

### Execução

```text
queued -> running -> completed
                  -> failed
                  -> cancel_requested -> cancelled

queued -> cancel_requested -> cancelled
```

Regras:

- `completed`, `failed` e `cancelled` são terminais;
- uma execução fica `running` quando o primeiro job entra em `running`;
- `completed` exige todos os jobs concluídos;
- `failed` ocorre quando um job obrigatório termina em `failed` ou
  `dead_letter`;
- `cancelled` exige todos os jobs em estado terminal;
- estado terminal nunca retorna a estado ativo.

### Job

```text
queued -> leased -> running -> completed
           |          |
           |          +-> failed
           |          +-> cancelled
           |          +-> retry_wait -> queued
           |
           +-> retry_wait -> queued

retry_wait -> dead_letter
queued     -> cancelled
```

Regras:

- claim incrementa `attempt` e `lease_epoch`;
- `lease_token` é novo a cada claim;
- renovação exige correspondência de runner, epoch e token;
- publicação e finalização usam o mesmo fencing;
- lease expirado nunca é renovado retroativamente;
- retry usa backoff e `available_at`;
- tentativa esgotada termina em `dead_letter`;
- erro de infraestrutura com tentativas restantes passa por `retry_wait`;
- `failed` é reservado para falha terminal que não deve ser repetida;
- resultado terminal aceito é imutável.

## Claim e Matching

O runner envia ao claim somente `runner_id` e `runner_session_token`. Protocolo,
pool, kinds suportados, labels, capabilities e limites de slots são lidos do
registro persistido. Slots disponíveis são derivados dos jobs ativos dentro da
mesma transação; o banco nunca confia num contador informado pelo worker.

O Postgres seleciona jobs:

1. `status = 'queued'`;
2. `available_at <= now()`;
3. protocolo compatível;
4. pool e kind compatíveis;
5. requisitos satisfeitos;
6. execução ainda deseja `running`;
7. prioridade maior primeiro;
8. criação mais antiga como desempate.

A seleção e atualização usam uma transação curta com
`FOR UPDATE SKIP LOCKED`. A transação termina antes do runner iniciar qualquer
HTTP contra a API alvo.

A `main` pode solicitar mais runners ao plugin Kubernetes quando a fila tiver
jobs elegíveis sem capacidade, mas não reserva um job para um endpoint
específico.

## Semântica At-Least-Once e Fencing

Um job pode executar mais de uma vez quando:

- o lease expira depois de o runner produzir efeitos externos;
- a conexão com Postgres é perdida antes de confirmar o estado terminal;
- o runner falha entre a execução e a gravação do resultado.

O Previa garante idempotência somente para seu estado interno:

- claim identificado por `job_id`, `attempt` e `lease_epoch`;
- eventos deduplicados por `(job_id, attempt, seq)`;
- resultado terminal aceito uma vez;
- projeção retomada por checkpoint;
- histórico final idempotente por `execution_id`.

APIs alvo continuam sujeitas a efeitos repetidos. Pipelines devem usar dados
isolados por execução/tentativa quando isso for relevante.

## Leases, Heartbeats e Reaper

O heartbeat do runner e o lease do job são independentes.

- heartbeat demonstra saúde da instância;
- lease concede propriedade temporária do job;
- runner saudável sem lease não pode publicar;
- runner com heartbeat atrasado não recebe novos jobs;
- job com lease expirado é recuperado mesmo que o runner ainda apareça na
  tabela.

Uma única `main` por vez executa o reaper usando advisory lock Postgres. Se a
instância desaparecer, outra assume o lock.

O runner interrompe a execução quando não consegue renovar o lease antes de
`lease_expires_at`. Um buffer local pode manter eventos durante indisponibilidade
curta, respeitando o limite configurado.

## Eventos, Notificações e Polling

Tabelas são a fonte de verdade. Canais:

- `previa_jobs`: jobs novos ou novamente disponíveis;
- `previa_events`: novos lotes de eventos;
- `previa_control`: cancelamento e draining;
- `previa_runners`: registro e mudança de capacidade.

Payloads de `NOTIFY` contêm somente identificadores pequenos. Dados completos
são sempre relidos das tabelas.

Se uma notificação for perdida:

- runners encontram jobs pelo polling;
- a `main` encontra eventos pelo checkpoint;
- cancelamento é encontrado ao renovar lease ou consultar controle.

O runner acumula eventos num buffer limitado e faz insert em lote. Se o buffer
atingir o máximo:

1. tenta flush imediato;
2. se não houver lease ou conexão dentro da janela segura, interrompe o job;
3. preserva erro local para diagnóstico;
4. não continua produzindo telemetria ilimitada em memória.

## Projeção, SSE e Histórico

Uma projeção por execução:

1. reivindica o lease em `execution_snapshots`;
2. lê eventos depois de `last_event_id`;
3. valida ordem e tentativa;
4. reutiliza os consolidadores atuais;
5. atualiza snapshot e checkpoint na mesma transação;
6. publica atualização SSE;
7. finaliza histórico quando todos os jobs estão terminais.

Várias instâncias da `main` podem atender clientes. Apenas a proprietária da
projeção aplica eventos; as demais leem o snapshot persistido e acordam por
`NOTIFY`.

O SSE expõe:

- `queued`;
- `leased`;
- `running`;
- `retrying`;
- `cancel_requested`;
- `completed`;
- `failed`;
- `cancelled`.

UI, API, MCP e histórico derivam do mesmo snapshot e das mesmas state machines.

## Configuração

Todos os valores operacionais são variáveis de ambiente com defaults na
aplicação. Ausência usa o default. Valor inválido, zero quando não permitido ou
combinação insegura impede startup com erro claro.

### `previa-main`

| Variável | Default | Faixa válida | Uso |
| --- | ---: | ---: | --- |
| `DATABASE_URL` | sem default | Postgres | Persistência operacional e fila |
| `PREVIA_QUEUE_RUNNER_STALE_AFTER_MS` | `15000` | `5000..300000` | Janela sem heartbeat |
| `PREVIA_QUEUE_JOB_LEASE_MS` | `30000` | `10000..600000` | Duração inicial/renovada do lease |
| `PREVIA_QUEUE_JOB_MAX_ATTEMPTS` | `3` | `1..10` | Tentativas padrão |
| `PREVIA_QUEUE_PROJECTION_LEASE_MS` | `30000` | `10000..300000` | Lease da projeção |
| `PREVIA_QUEUE_PROJECTION_POLL_INTERVAL_MS` | `1000` | `100..60000` | Recuperação de eventos sem notificação |
| `PREVIA_QUEUE_MAINTENANCE_INTERVAL_MS` | `1000` | `100..60000` | Reaper e promoção de retries |
| `PREVIA_QUEUE_RETRY_BACKOFF_BASE_MS` | `1000` | `100..60000` | Backoff da primeira repetição |
| `PREVIA_QUEUE_RETRY_BACKOFF_MAX_MS` | `30000` | `1000..600000` | Teto do backoff |
| `PREVIA_QUEUE_EVENT_RETENTION_HOURS` | `24` | `1..720` | Retenção após histórico final |
| `PREVIA_QUEUE_RUNNER_RETENTION_HOURS` | `168` | `1..8760` | Retenção de instâncias inativas |

### `previa-runner`

| Variável | Default | Faixa válida | Uso |
| --- | ---: | ---: | --- |
| `PREVIA_QUEUE_DATABASE_URL` | sem default | Postgres | Conexão restrita do worker |
| `PREVIA_QUEUE_HEARTBEAT_INTERVAL_MS` | `5000` | `1000..60000` | Frequência do heartbeat |
| `PREVIA_QUEUE_LEASE_RENEW_INTERVAL_MS` | `10000` | `1000..300000` | Frequência de renovação |
| `PREVIA_QUEUE_POLL_INTERVAL_MS` | `1000` | `100..60000` | Fallback de polling |
| `PREVIA_QUEUE_EVENT_FLUSH_INTERVAL_MS` | `250` | `10..10000` | Janela máxima do lote |
| `PREVIA_QUEUE_EVENT_BATCH_SIZE` | `200` | `1..1000` | Eventos por insert |
| `PREVIA_QUEUE_EVENT_BUFFER_MAX` | `5000` | `200..100000` | Limite de memória |

Validações cruzadas:

- intervalo de renovação deve ser menor que metade do lease recebido;
- janela de stale deve ser pelo menos duas vezes o heartbeat anunciado;
- buffer máximo deve ser pelo menos o tamanho de um lote;
- backoff máximo deve ser maior ou igual ao backoff base;
- runner deve recusar um claim cuja duração de lease seja incompatível com sua
  configuração;
- configurações efetivas não sensíveis aparecem em `/info`.

Compose, Helm, `.env.example` e documentação devem expor todas as variáveis.

## Postgres Obrigatório e SQLite de Transferência

`previa-main` usa `PgPool` para o runtime. `sqlx::Any` deixa de ser a abstração
da aplicação operacional.

Comportamento de startup:

- ausência de `DATABASE_URL` é erro;
- URL SQLite é rejeitada;
- conexão sem migrations compatíveis é rejeitada;
- protocolo incompatível é rejeitado;
- mensagens indicam como exportar e importar projetos.

SQLite permanece isolado no serviço de transferência:

- exporta um ou mais projetos para arquivo;
- importa arquivo para Postgres;
- mantém versão própria do formato;
- não executa scheduler, fila, auth ou histórico vivo;
- não é aberto como banco principal da `main`.

## Segurança

### Roles

São necessárias pelo menos duas roles:

- role da `main`, com acesso operacional e migrations conforme a estratégia de
  deployment;
- role do runner, sem acesso direto a tabelas administrativas.

A credencial do runner deve permitir apenas funções da fila:

- `queue_register_runner`;
- `queue_heartbeat_runner`;
- `queue_claim_job`;
- `queue_renew_job_lease`;
- `queue_publish_events`;
- `queue_complete_job`;
- `queue_fail_job`;
- `queue_acknowledge_cancellation`;
- `queue_read_control`.

Funções `SECURITY DEFINER`, quando usadas, devem:

- definir `search_path` fixo;
- validar todos os identificadores;
- validar `runner_session_token` sem armazená-lo em texto puro;
- aplicar fencing;
- não aceitar SQL dinâmico do runner;
- retornar somente payloads de jobs reivindicados;
- possuir testes de privilégios.

O runner não pode ler:

- usuários e tokens;
- projetos que não estejam no payload do job;
- specs e pipelines fora do snapshot recebido;
- históricos;
- jobs de outro runner depois do claim;
- credenciais administrativas.

Secrets:

- Compose usa secret ou env protegida;
- Helm usa Kubernetes Secret;
- plugin Kubernetes injeta apenas a credencial restrita;
- URLs de banco nunca aparecem em logs ou `/info`;
- payloads e erros passam pela política existente de redaction.

## Compatibilidade e Corte

Esta é uma mudança breaking.

Regras:

- `main` e runner precisam falar a mesma `queue_protocol_version`;
- não há modo dual HTTP/Postgres;
- runners antigos não recebem jobs;
- a `main` nova não chama endpoints antigos;
- endpoints HTTP de execução do runner são removidos;
- permanecem somente `/health`, `/ready` e `/info`;
- `/health` e `/ready` são adequados para probes;
- `/info` retorna somente dados operacionais seguros.

`PREVIA_RUNNER_AUTH_KEY` deixa de proteger execução porque não existe API HTTP
de trabalho. Se ainda for necessário proteger `/info`, isso deve usar uma
configuração específica de endpoint administrativo, não reutilizar a chave do
transporte removido.

## Implantação

### CLI e Compose

`previa up`:

- inicia Postgres obrigatoriamente;
- configura volume persistente;
- aguarda healthcheck;
- executa migrations pela `main`;
- cria/injeta a role restrita do runner;
- inicia runners somente depois de o protocolo estar disponível.

O modo binário também exige um Postgres alcançável. Não existe runtime
operacional embutido em SQLite.

### Helm

Em produção:

- Postgres externo é obrigatório;
- credenciais da `main` e runners são separadas;
- chart aceita referências a Secrets existentes;
- probes continuam HTTP;
- runners recebem pool, labels, capabilities e slots;
- upgrades incompatíveis atualizam migrations e protocolo antes de liberar
  workers.

### Plugin Kubernetes

O plugin:

- recebe referência à Secret restrita;
- injeta a conexão nos pods de runner;
- não distribui credencial administrativa;
- cria runners com protocolo e labels esperados;
- observa registro/heartbeat para considerar capacidade pronta;
- mantém sua política de idle reuse e cleanup.

## Observabilidade

Logs relacionados a execução incluem:

- `execution_id`;
- `job_id`;
- `attempt`;
- `lease_epoch`;
- `runner_id`;
- `transaction_id`, quando disponível.

A `main` expõe dados operacionais para:

- runners `ready`, `busy`, `draining`, `stale` e `stopped`;
- jobs por estado;
- idade do job elegível mais antigo;
- jobs em retry e dead letter;
- attempts por execução;
- leases próximos de expirar;
- utilização de slots por pool e kind;
- atraso entre último evento e snapshot;
- eventos ainda não projetados;
- idade do último heartbeat;
- valores efetivos das configurações não sensíveis.

Alertas recomendados:

- job elegível sem claim além do limite operacional;
- crescimento de dead letters;
- runner stale com job ativo;
- projection lag crescente;
- event backlog crescente;
- falha repetida de lease renewal;
- Postgres indisponível;
- incompatibilidade de protocolo.

## Retenção

O reaper de retenção roda sob advisory lock.

Eventos só podem ser removidos quando:

- execução terminal;
- snapshot atualizado até o último evento;
- histórico final existente;
- idade superior a `PREVIA_QUEUE_EVENT_RETENTION_HOURS`.

Registros de jobs e execuções permanecem enquanto forem necessários para
histórico e auditoria. Uma política futura pode compactá-los, mas não faz parte
desta mudança.

Runner instances antigas podem ser removidas apenas quando:

- estão `stale` ou `stopped`;
- não possuem jobs ativos;
- excederam `PREVIA_QUEUE_RUNNER_RETENTION_HOURS`.

## Organização do Código

Transportes, serviços e modelos continuam separados.

Na `previa-main`:

```text
main/src/server/queue/
  config.rs
  models.rs
  repository.rs
  dispatcher.rs
  projector.rs
  retention.rs
```

Responsabilidades:

- `config.rs`: parsing, defaults e invariantes;
- `models.rs`: contratos internos da fila;
- `repository.rs`: SQL e transações;
- `dispatcher.rs`: criação, cancelamento, reaper e capacidade;
- `projector.rs`: consumo, snapshot, SSE e histórico;
- `retention.rs`: limpeza segura.

No runner:

```text
runner/src/server/queue/
  config.rs
  repository.rs
  worker.rs
  heartbeat.rs
  event_buffer.rs
```

Responsabilidades:

- `config.rs`: conexão e parâmetros do worker;
- `repository.rs`: chamadas restritas ao Postgres;
- `worker.rs`: claim, execução, lease e controle;
- `heartbeat.rs`: registro, heartbeat e draining;
- `event_buffer.rs`: sequência, lote e backpressure.

Handlers HTTP da `main` chamam services de execução. Eles não contêm SQL nem
lógica de lease. O engine continua independente de transporte.

## Impacto nos Contratos

### API externa da `previa-main`

Rotas de criação e consulta continuam disponíveis. Respostas e SSE passam a
usar os estados da fila.

Contratos OpenAPI, models e cliente TypeScript devem permanecer sincronizados.

### API do runner

Remover rotas de:

- start E2E;
- start load;
- status de execução;
- cancelamento de execução;
- telemetry;
- telemetry ack.

Manter:

- `/health`;
- `/ready`;
- `/info`;
- `/openapi.json`, caso continue útil para os endpoints restantes.

### MCP

Ferramentas de execução continuam chamando a `main`. Status, cancelamento,
histórico e execution summary usam a mesma execução persistida. O MCP não
conecta diretamente ao runner.

## Estratégia de Migração

1. Publicar orientação de backup/export para instalações SQLite.
2. Adicionar schema Postgres da fila e protocolo.
3. Implementar módulos compartilhados de config e modelos.
4. Implementar funções restritas e testes de privilégios.
5. Implementar registro, heartbeat e claim no runner.
6. Implementar execução E2E por job.
7. Implementar shards de load e eventos agregados.
8. Implementar projeção, SSE e histórico na `main`.
9. Implementar cancelamento, retries, reaper e dead letter.
10. Atualizar Compose, CLI, Helm e plugin Kubernetes.
11. Remover transporte HTTP antigo do runner e da `main`.
12. Remover SQLite do startup operacional.
13. Validar import/export SQLite.
14. Publicar todos os componentes com a mesma versão de protocolo.

O corte é único por ambiente. Durante a atualização, a fila deve permanecer
pausada até que `main`, migrations e runners estejam compatíveis.

## Testes

### Configuração

- defaults de todas as envs;
- override de cada env;
- valor vazio, zero, inválido e fora da faixa;
- heartbeat incompatível com stale;
- renew incompatível com lease recebido;
- buffer menor que batch;
- URL SQLite rejeitada na `main` e no runner.

### Banco e concorrência

- dois runners não reivindicam o mesmo job;
- `SKIP LOCKED` permite claims paralelos de jobs diferentes;
- prioridade e ordem são respeitadas;
- requisitos e pool filtram corretamente;
- slots impedem excesso de jobs;
- fencing rejeita runner, token e epoch antigos;
- evento duplicado não duplica projeção;
- resultado terminal é idempotente;
- expiração produz retry ou dead letter;
- cancelamento impede novo claim;
- role do runner não lê tabelas administrativas.

### Notificações e recuperação

- job é encontrado por `NOTIFY`;
- job é encontrado por polling quando `NOTIFY` é perdido;
- cancelamento é encontrado sem notificação;
- reinício da `main` retoma `last_event_id`;
- segunda `main` assume projeção expirada;
- perda de Postgres interrompe runner antes de operar sem lease;
- buffer limitado aplica backpressure;
- reaper sob advisory lock tem um único proprietário.

### E2E

- execução completa por um runner;
- eventos de passo atualizam snapshot;
- morte do runner gera nova tentativa;
- nova tentativa incrementa attempt e epoch;
- efeito terminal cria um único histórico;
- cancelamento antes e depois do claim;
- dead letter falha a execução.

### Load

- execução é dividida em múltiplos shards;
- runners reivindicam shards concorrentes;
- RPS e wave profile são divididos corretamente;
- buckets são consolidados sem duplicação;
- retry executa somente a janela restante;
- slots expirados não são reproduzidos;
- cancelamento termina todos os shards;
- histórico final mantém o contrato existente.

### Integração e produto

- API, SSE, UI e MCP mostram os mesmos estados;
- runner OpenAPI não contém endpoints removidos;
- contrato OpenAPI da `main` e cliente TypeScript permanecem sincronizados;
- Compose inicia Postgres, `main` e runners;
- Helm injeta roles distintas;
- plugin Kubernetes registra runners provisionados;
- import/export SQLite preserva projetos;
- upgrade com protocolo incompatível falha cedo.

### Validação obrigatória

```bash
cargo test --workspace
python3 scripts/check_openapi_client_contract.py
cd app && npm test
cd app && npm run build
cargo build --release
```

O CI deve iniciar um Postgres real para testes de migrations, concorrência,
roles e recuperação.

## Critérios de Aceite

- `previa-main` não conhece endpoints de execução dos runners.
- Runners reivindicam jobs compatíveis com `FOR UPDATE SKIP LOCKED`.
- E2E executa integralmente pelo canal Postgres.
- Load test distribui shards entre múltiplos runners pelo canal Postgres.
- Start, status, cancelamento, telemetria e resultado não usam HTTP entre
  `main` e runner.
- HTTP do runner contém somente endpoints operacionais seguros.
- Leases, heartbeats, retries, fencing e dead letter funcionam sob concorrência.
- Perda de `NOTIFY` não perde trabalho.
- Reinício da `main` não perde eventos já confirmados.
- Runner sem lease válido interrompe o trabalho.
- Configurações possuem defaults na aplicação e overrides por env.
- Postgres é obrigatório no runtime.
- SQLite funciona somente para importação e exportação.
- Roles de banco impedem acesso administrativo pelo runner.
- UI, API, SSE, MCP e histórico compartilham a mesma state machine.
- Compose, Helm e plugin Kubernetes suportam o novo fluxo.
- Não existe fallback para o transporte HTTP removido.
