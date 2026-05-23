# Idle Runner Reuse Design

## Objetivo

Permitir que novas reservas reutilizem runners ja criados e ociosos antes de
criar novos pods e nodes no Kubernetes.

O plugin deve continuar garantindo:

- runners em nodes dedicados;
- no maximo um runner Previa por node;
- validacao de primeira execucao por `reservationId` e `reservationToken`;
- remocao de runners ociosos apos o TTL configurado;
- preservacao de runners ocupados durante execucoes.

## Contexto Atual

Hoje cada reserva Kubernetes cria um conjunto novo de recursos:

- `StatefulSet`;
- `Service` headless;
- `PodDisruptionBudget`;
- pods de runner.

Cada pod de runner recebe `PREVIA_RESERVATION_ID` e
`PREVIA_RESERVATION_TOKEN` via variaveis de ambiente. Depois da primeira
execucao, o runner desativa o gate da reserva dentro do processo para que a
execucao em andamento possa seguir sem exigir novamente o token inicial.

Esse modelo funciona para runners descartaveis, mas nao e suficiente para
reaproveitamento. Para reutilizar um runner com seguranca, o processo precisa
ser rearmado com uma nova reserva e um novo token antes de aceitar a proxima
execucao.

## Estados

### Reserva

- `provisioning`: a reserva esta alocando runners.
- `ready`: todos os runners pedidos estao prontos para primeira execucao.
- `running`: pelo menos um runner da reserva esta ocupado.
- `idleReusable`: a reserva fisica ja executou, todos os runners estao idle,
  saudaveis e ainda dentro do TTL de reaproveitamento.
- `draining`: a reserva fisica nao deve receber novas reservas logicas.
- `terminating`: os recursos Kubernetes estao sendo removidos.
- `failed`: houve erro de provisionamento, saude ou reconciliacao.
- `cancelled`: a reserva foi cancelada explicitamente.
- `expired`: a reserva expirou antes de ser consumida ou passou do TTL idle.

### Runner

- `reserved`: runner pronto, vinculado a uma reserva logica, ainda nao usado.
- `running`: runner executando teste.
- `idleReusable`: runner ja executou, terminou, esta saudavel e pode ser
  reaproveitado.
- `draining`: runner nao deve receber nova reserva e sera removido quando
  possivel.
- `terminating`: pod em remocao.
- `failed`: runner nao saudavel ou invalido para uso.

## Conceitos

### Reserva Logica

Reserva logica e a solicitacao criada pelo Previa main para uma execucao de
pipeline. Ela possui:

- `reservationId`;
- `reservationToken`;
- `executionId`;
- `pipelineId`;
- quantidade solicitada;
- lista de runners alocados.

### Runner Fisico

Runner fisico e o pod real no Kubernetes. Ele pode sobreviver entre reservas
logicas enquanto estiver dentro do idle TTL.

Um runner fisico deve guardar, no estado interno do plugin:

- `runnerId`;
- `endpoint`;
- `physicalReservationId`;
- `logicalReservationId` atual, quando reservado;
- `state`;
- `idleSince`;
- `lastHealth`;
- `lastStartedAt`;
- `lastFinishedAt`.

O `physicalReservationId` continua sendo o ID usado nos recursos Kubernetes
originais. Isso evita relabelar selectors imutaveis de `StatefulSet` e
`Service`.

## Fluxo De Nova Reserva

Quando o Previa main solicitar uma reserva de `N` runners:

1. O plugin cria uma nova reserva logica com novo `reservationId` e novo
   `reservationToken`.
2. O plugin procura runners em `idleReusable`.
3. Para cada candidato, o plugin valida:
   - runner saudavel;
   - `busy=false`;
   - idle TTL ainda valido;
   - configuracao compativel;
   - runner nao esta `draining`, `terminating` ou `failed`.
4. O plugin seleciona ate `N` runners compativeis.
5. Para cada runner selecionado, o plugin chama a API interna de rearm no
   runner.
6. Se os runners reaproveitados forem suficientes, a reserva logica fica
   `ready` imediatamente.
7. Se faltar capacidade, o plugin cria apenas o delta em novos pods/nodes.
8. A reserva logica retorna `ready` quando os runners reaproveitados e os
   novos runners estiverem prontos.

Uma reserva logica pode conter runners reaproveitados e runners recem-criados.

## Compatibilidade Para Reuso

Um runner so pode ser reaproveitado se:

- estiver em `idleReusable`;
- estiver saudavel via endpoint de saude/runtime;
- nao estiver ocupado;
- ainda estiver dentro do `PREVIA_IDLE_TTL_SECONDS`;
- usar imagem compativel;
- usar porta compativel;
- usar comando compativel;
- usar node pool compativel;
- usar a mesma politica de node exclusivo;
- pertencer ao mesmo namespace do plugin.

Em v0, compatibilidade deve ser conservadora. Se houver duvida, o runner nao
deve ser reaproveitado.

## API Interna Do Runner

Adicionar endpoint interno:

```text
POST /internal/reservation/rearm
```

Payload:

```json
{
  "reservationId": "rr_new",
  "reservationToken": "rt_new",
  "expiresAt": "2026-05-22T16:30:00Z"
}
```

Regras:

- aceita apenas se o runner nao estiver `busy`;
- rejeita se ja houver uma reserva nao consumida ativa;
- atualiza o `reservationId`, `reservationToken` e `expiresAt`;
- reseta o gate de primeira execucao para `consumed=false`;
- mantem contadores historicos de execucoes;
- depois do rearm, rejeita token antigo;
- depois do rearm, aceita somente headers da nova reserva.

Headers exigidos na primeira execucao apos rearm:

```text
x-previa-reservation-id
x-previa-reservation-token
```

Adicionar tambem:

```text
POST /internal/reservation/release
```

Esse endpoint libera uma reserva logica ainda nao consumida e devolve o runner
para `idleReusable`, desde que o runner nao esteja busy.

## Mudancas No Plugin

O plugin passa a manter um indice de runners fisicos reutilizaveis.

Ao reconciliar uma reserva:

- se algum runner iniciou execucao, a reserva logica passa para `running`;
- se todos os runners terminaram e estao idle, os runners passam para
  `idleReusable`;
- runners `idleReusable` ficam disponiveis para novas reservas ate o idle TTL;
- runners `idleReusable` expirados entram em `terminating`.

Ao criar uma nova reserva:

- buscar runners `idleReusable` antes de chamar o Kubernetes;
- rearmar runners reaproveitados com o novo token;
- criar apenas o delta faltante;
- registrar todos os runners no status da nova reserva logica;
- retornar endpoints fisicos existentes quando houver reaproveitamento.

## Kubernetes

Nao relabelar selectors imutaveis de `StatefulSet` ou `Service`.

O DNS retornado para um runner reaproveitado pode continuar sendo o DNS fisico
original:

```text
previa-runner-<physical-id>-0.previa-runner-<physical-id>.previa.svc.cluster.local
```

O vinculo com a nova reserva e logico e e garantido pelo token rearmado no
runner.

Continuam obrigatorios:

- node dedicado com taint `previa.runvibe.com/runner-only=true:NoSchedule`;
- `nodeSelector` para `previa.runvibe.com/node-role=runner`;
- anti-affinity global entre runners Previa;
- `PDB` por grupo fisico de runners.

## Cleanup

- Runner `idleReusable` que passar do idle TTL e nao estiver reservado deve ser
  removido.
- Runner `busy` nunca deve ser removido.
- Reserva logica nao consumida que expirar deve liberar runners reaproveitados
  ou remover runners novos criados para ela.
- Em v0, evitar scale-down parcial de `StatefulSet` para nao brigar com
  ordinais do Kubernetes.
- Um grupo fisico deve ser removido apenas quando nenhum runner dele estiver
  `reserved`, `running` ou vinculado a uma reserva logica ativa.

## Cancelamento

Se o Previa main cancelar uma reserva:

- runners reaproveitados e ainda nao consumidos devem receber `release`;
- runners novos e ainda nao consumidos podem ser removidos;
- runners busy nao devem ser removidos;
- se a execucao ja tiver iniciado, o fluxo normal de idle cleanup deve assumir.

## Exemplo

1. Teste A solicita 5 runners.
2. Plugin cria 5 runners em nodes dedicados.
3. Teste A termina.
4. Os 5 runners ficam `idleReusable`.
5. Teste B solicita 2 runners.
6. Plugin rearma 2 runners existentes e retorna `ready`.
7. Os outros 3 continuam `idleReusable`.
8. Teste C solicita 6 runners.
9. Plugin reaproveita os 3 restantes e cria mais 3 runners novos.

## Criterios De Aceite

- Nova reserva reaproveita runners `idleReusable` antes de criar pods.
- Nova reserva cria apenas o delta faltante.
- Runner rearmado rejeita token antigo.
- Runner rearmado aceita token novo.
- Runner busy nao e reaproveitado.
- Runner expirado nao e reaproveitado.
- Reserva cancelada antes da primeira execucao libera runner reaproveitado.
- Idle cleanup nao remove runner vinculado a uma nova reserva.
- Nodes dedicados continuam sendo usados.
- Anti-affinity global continua impedindo mais de um runner Previa por node.
- Runners nao sao removidos durante execucao.

## Testes Necessarios

- `create_reservation_reuses_idle_runners`
- `create_reservation_creates_only_missing_delta`
- `busy_runner_is_not_reused`
- `expired_idle_runner_is_not_reused`
- `rearmed_runner_rejects_old_token`
- `rearmed_runner_accepts_new_token`
- `cancelled_unconsumed_reservation_releases_reused_runner`
- `idle_cleanup_does_not_delete_runner_leased_to_new_reservation`
- `reused_runner_keeps_physical_dns_endpoint`
- `physical_statefulset_is_not_relabelled_on_reuse`

## Fora De Escopo Para V0

- Relabelar ou renomear recursos Kubernetes fisicos.
- Scale-down parcial inteligente de StatefulSets reaproveitados.
- Pool quente minimo configuravel.
- Rebalanceamento entre reservas simultaneas.
- Persistencia do indice de runners reutilizaveis apos restart do plugin.

