import { useMemo } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { helperDocs } from "@/lib/template-helpers";
import MonacoCodeEditor from "@/components/MonacoCodeEditor";
import type { FormatType } from "@/lib/pipeline-schema";
import yaml from "js-yaml";

interface CodeExampleProps {
  json: object;
  isDark?: boolean;
  height?: string;
  format: FormatType;
  onFormatChange: (format: FormatType) => void;
}

function CodeExample({ json, isDark = false, height = "200px", format, onFormatChange }: CodeExampleProps) {
  const content = useMemo(() => {
    return format === "yaml" 
      ? yaml.dump(json, { indent: 2, lineWidth: -1 })
      : JSON.stringify(json, null, 2);
  }, [json, format]);

  return (
    <div className="rounded-md border overflow-hidden">
      <MonacoCodeEditor
        value={content}
        format={format}
        onFormatChange={onFormatChange}
        readOnly
        isDark={isDark}
        height={height}
        showHeader
        showValidation={false}
        showLineNumbers
      />
    </div>
  );
}

interface PipelineDocsPanelProps {
  isDark?: boolean;
  format: FormatType;
  onFormatChange: (format: FormatType) => void;
}

const FULL_EXAMPLE = {
  name: "Criar Usuário e Enviar Email",
  description: "Pipeline de cadastro completo",
  steps: [
    {
      id: "create_user",
      name: "Criar Usuário",
      operationId: "createUser",
      description: "Cria um novo usuário com dados aleatórios",
      method: "POST",
      url: "{{envs.current.api}}/users",
      headers: { "Content-Type": "application/json" },
      body: {
        id: "{{helpers.uuid}}",
        name: "{{helpers.name}}",
        email: "{{helpers.email}}",
        cpf: "{{helpers.cpf}}"
      },
      delay: 2,
      retry: 3,
      asserts: [
        { field: "status", operator: "equals", expected: "201" },
        { field: "body.id", operator: "exists" },
        { field: "body.email", operator: "contains", expected: "@" }
      ]
    },
    {
      id: "send_email",
      name: "Enviar Email de Boas-Vindas",
      description: "Envia email usando dados do step anterior",
      method: "POST",
      url: "{{envs.current.api}}/emails/welcome",
      headers: { "Content-Type": "application/json" },
      body: {
        to: "{{steps.create_user.email}}",
        name: "{{steps.create_user.name}}"
      },
      asserts: [
        { field: "status", operator: "equals", expected: "200" },
        { field: "body.sent", operator: "equals", expected: "true" }
      ]
    }
  ]
};

const MULTI_ENV_EXAMPLE = {
  name: "Meu Pipeline",
  description: "Pipeline com múltiplos ambientes",
  steps: [
    {
      id: "health_check_hml",
      name: "Health Check HML",
      description: "Verifica se a API de homologação está respondendo",
      method: "GET",
      url: "{{envs.current.api}}/health",
      headers: {}
    },
    {
      id: "health_check_prd",
      name: "Health Check PRD",
      description: "Verifica se a API de produção está respondendo",
      method: "GET",
      url: "{{envs.prd.api}}/health",
      headers: {}
    }
  ]
};

export default function PipelineDocsPanel({ isDark = false, format, onFormatChange }: PipelineDocsPanelProps) {
  return (
    <ScrollArea className="h-full min-h-0 p-4">
      <div className="space-y-6 max-w-2xl mx-auto">
        {/* Header */}
        <div>
          <h2 className="text-xl font-bold">Documentação do Pipeline</h2>
          <p className="text-sm text-muted-foreground mt-1">
            Guia completo sobre a estrutura JSON/YAML e variáveis disponíveis.
          </p>
        </div>

        <Separator />

        {/* JSON Structure */}
        <section className="space-y-3">
          <h3 className="text-lg font-semibold">Estrutura do Pipeline</h3>
          <p className="text-sm text-muted-foreground">
            Um pipeline define uma sequência de requisições HTTP a serem executadas em ordem.
          </p>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">Campos do Pipeline</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2 text-sm">
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">name</code>
                <span className="text-muted-foreground">Nome do pipeline (obrigatório)</span>
              </div>
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">description</code>
                <span className="text-muted-foreground">Descrição do pipeline (obrigatório)</span>
              </div>
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">steps</code>
                <span className="text-muted-foreground">Array de steps (requisições)</span>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">Campos do Step</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2 text-sm">
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">id</code>
                <span className="text-muted-foreground">Identificador único do step</span>
              </div>
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">name</code>
                <span className="text-muted-foreground">Nome do step</span>
              </div>
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">description</code>
                <span className="text-muted-foreground">Descrição do step</span>
              </div>
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">method</code>
                <span className="text-muted-foreground">GET, POST, PUT, PATCH ou DELETE</span>
              </div>
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">url</code>
                <span className="text-muted-foreground">URL da requisição (pode usar variáveis)</span>
              </div>
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">headers</code>
                <span className="text-muted-foreground">Objeto com headers da requisição</span>
              </div>
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">body</code>
                <span className="text-muted-foreground">Corpo da requisição (opcional)</span>
              </div>
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">operationId</code>
                <span className="text-muted-foreground">ID da operação no OpenAPI spec (opcional)</span>
              </div>
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">asserts</code>
                <span className="text-muted-foreground">Array de assertions para validar o response (opcional)</span>
              </div>
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">delay</code>
                <span className="text-muted-foreground">Tempo de espera em milissegundos antes de executar (0–300000, opcional)</span>
              </div>
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">retry</code>
                <span className="text-muted-foreground">Número de tentativas em caso de erro (0–10, opcional)</span>
              </div>
            </CardContent>
          </Card>
        </section>

        <Separator />

        {/* Assertions */}
        <section className="space-y-3">
          <h3 className="text-lg font-semibold">Assertions</h3>
          <p className="text-sm text-muted-foreground">
            Cada step pode ter um array <code className="bg-muted px-1 rounded-sm text-xs">asserts</code> para validar automaticamente o response.
            Se algum assertion falhar, o step é marcado como erro.
          </p>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">Campos do Assertion</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2 text-sm">
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">field</code>
                <span className="text-muted-foreground">Campo a validar: <code className="bg-muted px-1 rounded-sm text-xs">status</code>, <code className="bg-muted px-1 rounded-sm text-xs">body.campo</code> ou <code className="bg-muted px-1 rounded-sm text-xs">header.nome</code></span>
              </div>
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">operator</code>
                <span className="text-muted-foreground">Operador de comparação</span>
              </div>
              <div className="flex gap-2">
                <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono">expected</code>
                <span className="text-muted-foreground">Valor esperado (suporta variáveis como <code className="bg-muted px-1 rounded-sm text-xs">{`{{steps.id.body.x}}`}</code>)</span>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">Operadores Disponíveis</CardTitle>
            </CardHeader>
            <CardContent className="space-y-1.5 text-sm">
              {[
                { op: "equals", desc: "Valor é exatamente igual" },
                { op: "not_equals", desc: "Valor é diferente" },
                { op: "contains", desc: "Valor contém a substring" },
                { op: "exists", desc: "Campo existe e não é null" },
                { op: "not_exists", desc: "Campo não existe ou é null" },
                { op: "gt", desc: "Valor numérico é maior que" },
                { op: "lt", desc: "Valor numérico é menor que" },
              ].map((item) => (
                <div key={item.op} className="flex gap-2">
                  <code className="bg-muted px-1.5 py-0.5 rounded-sm text-xs font-mono shrink-0">{item.op}</code>
                  <span className="text-muted-foreground">{item.desc}</span>
                </div>
              ))}
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">Exemplo de Assertions</CardTitle>
            </CardHeader>
            <CardContent className="text-sm">
              <pre className="bg-muted p-2 rounded-md text-xs overflow-x-auto">
{`"asserts": [
  { "field": "status", "operator": "equals", "expected": "201" },
  { "field": "body.id", "operator": "exists" },
  { "field": "body.name", "operator": "equals", "expected": "{{steps.create_user.body.name}}" },
  { "field": "header.content-type", "operator": "contains", "expected": "application/json" }
]`}
              </pre>
            </CardContent>
          </Card>
        </section>

        <Separator />

        {/* Delay & Retry */}
        <section className="space-y-3">
          <h3 className="text-lg font-semibold">Delay & Retry</h3>
          <p className="text-sm text-muted-foreground">
            Cada step pode configurar um tempo de espera antes da execução e tentativas automáticas em caso de falha.
          </p>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">Delay (Tempo de Espera)</CardTitle>
            </CardHeader>
            <CardContent className="text-sm space-y-2">
              <p className="text-muted-foreground">
                O campo <code className="bg-muted px-1 rounded-sm text-xs">delay</code> define quantos milissegundos esperar antes de executar o step. Útil para aguardar processamentos assíncronos ou rate limits.
              </p>
              <p className="text-muted-foreground text-xs">
                Valor em milissegundos, de <strong>0</strong> a <strong>300000</strong> (5 minutos).
              </p>
              <pre className="bg-muted p-2 rounded-md text-xs overflow-x-auto">
{`"delay": 5000  // aguarda 5 segundos antes de executar`}
              </pre>
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">Retry (Tentativas)</CardTitle>
            </CardHeader>
            <CardContent className="text-sm space-y-2">
              <p className="text-muted-foreground">
                O campo <code className="bg-muted px-1 rounded-sm text-xs">retry</code> define quantas vezes re-executar o step em caso de erro. O total de execuções será <code className="bg-muted px-1 rounded-sm text-xs">retry + 1</code> (a tentativa original + retries).
              </p>
              <p className="text-muted-foreground text-xs">
                Valor de <strong>0</strong> (sem retry) a <strong>10</strong>.
              </p>
              <pre className="bg-muted p-2 rounded-md text-xs overflow-x-auto">
{`"retry": 3  // até 3 tentativas adicionais em caso de erro`}
              </pre>
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">Exemplo com Delay e Retry</CardTitle>
            </CardHeader>
            <CardContent className="text-sm">
              <pre className="bg-muted p-2 rounded-md text-xs overflow-x-auto">
{`{
  "id": "check_status",
  "name": "Verificar Status",
  "description": "Aguarda e verifica o status do processamento",
  "method": "GET",
  "url": "{{specs.jobs-api.url.hml}}/jobs/{{steps.create_job.id}}/status",
  "headers": {},
  "delay": 10,
  "retry": 5,
  "asserts": [
    { "field": "status", "operator": "equals", "expected": "200" },
    { "field": "body.status", "operator": "equals", "expected": "completed" }
  ]
}`}
              </pre>
              <p className="text-muted-foreground text-xs mt-2">
                Neste exemplo, o step aguarda 10 segundos e tenta até 6 vezes (1 original + 5 retries) até que o job esteja completo.
              </p>
            </CardContent>
          </Card>
        </section>

        <Separator />

        {/* Variables */}
        <section className="space-y-3">
          <h3 className="text-lg font-semibold">Variáveis do Sistema</h3>
          <p className="text-sm text-muted-foreground">
            Use a sintaxe <code className="bg-muted px-1 rounded-sm text-xs">{`{{variavel}}`}</code> para interpolar valores dinamicamente.
          </p>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm flex items-center gap-2">
                <Badge variant="outline" className="font-mono">{`envs.current.<entry>`}</Badge>
                URL do Env Group
              </CardTitle>
            </CardHeader>
            <CardContent className="text-sm space-y-2">
              <p className="text-muted-foreground">
                Referencia uma entrada do env group selecionado na execução.
              </p>
              <pre className="bg-muted p-2 rounded-md text-xs overflow-x-auto">
{`"url": "{{envs.current.api}}/users"`}
              </pre>
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm flex items-center gap-2">
                <Badge variant="outline" className="font-mono">{`specs.<slug>.url.<env>`}</Badge>
                URL do Ambiente (via Spec)
              </CardTitle>
            </CardHeader>
            <CardContent className="text-sm space-y-2">
              <p className="text-muted-foreground">
                Referencia a URL de um ambiente configurado em uma spec OpenAPI do projeto. O <code className="bg-muted px-1 rounded-sm text-xs">slug</code> identifica a spec e o <code className="bg-muted px-1 rounded-sm text-xs">env</code> o ambiente (ex: hml, prd).
              </p>
              <pre className="bg-muted p-2 rounded-md text-xs overflow-x-auto">
{`"url": "{{specs.users-api.url.hml}}/users"`}
              </pre>
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm flex items-center gap-2">
                <Badge variant="outline" className="font-mono">steps.*</Badge>
                Resultado de Steps Anteriores
              </CardTitle>
            </CardHeader>
            <CardContent className="text-sm space-y-2">
              <p className="text-muted-foreground">
                Acessa dados do response body de um step anterior.
              </p>
              <p className="text-muted-foreground text-xs">
                Sintaxe: <code className="bg-muted px-1 rounded-sm">{`{{steps.<step_id>.<campo>}}`}</code>
              </p>
              <pre className="bg-muted p-2 rounded-md text-xs overflow-x-auto">
{`// Se o step "create_user" retornou { "id": "123", "email": "test@example.com" }

"user_id": "{{steps.create_user.id}}"
"email": "{{steps.create_user.email}}"`}
              </pre>
              <p className="text-muted-foreground text-xs mt-2">
                Para campos aninhados:
              </p>
              <pre className="bg-muted p-2 rounded-md text-xs overflow-x-auto">
{`// Se o response foi { "data": { "user": { "name": "João" } } }

"nome": "{{steps.get_user.data.user.name}}"`}
              </pre>
            </CardContent>
          </Card>
        </section>

        <Separator />

        {/* Helpers */}
        <section className="space-y-3">
          <h3 className="text-lg font-semibold">Helpers (Dados Dinâmicos)</h3>
          <p className="text-sm text-muted-foreground">
            Use <code className="bg-muted px-1 rounded-sm text-xs">{`{{helpers.<nome>}}`}</code> para gerar dados aleatórios.
          </p>

          <div className="grid gap-2">
            {helperDocs.map((helper) => (
              <Card key={helper.name} className="py-2">
                <CardContent className="px-4 py-0">
                  <div className="flex items-center justify-between gap-4">
                    <div className="flex items-center gap-2 min-w-0">
                      <code className="bg-primary/10 text-primary px-1.5 py-0.5 rounded-sm text-xs font-mono shrink-0">
                        {`{{helpers.${helper.name}}}`}
                      </code>
                      <span className="text-sm text-muted-foreground truncate">{helper.description}</span>
                    </div>
                    <span className="text-xs font-mono text-muted-foreground shrink-0 hidden sm:block">
                      {helper.example}
                    </span>
                  </div>
                </CardContent>
              </Card>
            ))}
          </div>
        </section>

        <Separator />

        {/* Example */}
        <section className="space-y-3">
          <h3 className="text-lg font-semibold">Exemplo Completo</h3>
          <p className="text-sm text-muted-foreground">
            Pipeline que cria um usuário com dados aleatórios e envia um email de boas-vindas.
          </p>
          
          <CodeExample json={FULL_EXAMPLE} isDark={isDark} height="380px" format={format} onFormatChange={onFormatChange} />
        </section>

        <Separator />

        {/* Multiple environments */}
        <section className="space-y-3">
          <h3 className="text-lg font-semibold">Múltiplos Ambientes</h3>
          <p className="text-sm text-muted-foreground">
            Use <code className="bg-muted px-1 rounded-sm text-xs">{`{{envs.current.<entry>}}`}</code> para seguir o env group selecionado na execução, ou <code className="bg-muted px-1 rounded-sm text-xs">{`{{envs.<group>.<entry>}}`}</code> para fixar um grupo específico.
          </p>
          
          <CodeExample json={MULTI_ENV_EXAMPLE} isDark={isDark} height="280px" format={format} onFormatChange={onFormatChange} />
        </section>
      </div>
    </ScrollArea>
  );
}
