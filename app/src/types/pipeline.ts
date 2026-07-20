export interface StepAssertion {
  field: string;
  operator: "equals" | "not_equals" | "contains" | "exists" | "not_exists" | "gt" | "lt";
  expected?: string;
}

export interface AssertionResult {
  assertion: StepAssertion;
  passed: boolean;
  actual?: string;
}

export interface StepExtraction {
  name: string;
  field: string;
  regex: string;
  group?: number;
  required?: boolean;
}

export interface PipelineStep {
  id: string;
  name: string;
  description: string;
  headers: Record<string, string>;
  method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE";
  url: string;
  body?: Record<string, unknown>;
  operationId?: string;
  asserts?: StepAssertion[];
  extracts?: StepExtraction[];
  delay?: number;
  retry?: number;
}

export interface Pipeline {
  id?: string;
  name: string;
  description: string;
  steps: PipelineStep[];
  updatedAt?: string;
}

export interface StepExecutionResult {
  stepId: string;
  status: "pending" | "running" | "success" | "error";
  request?: {
    method: string;
    url: string;
    headers: Record<string, string>;
    body?: unknown;
  };
  response?: {
    status: number;
    statusText: string;
    headers: Record<string, string>;
    body: unknown;
  };
  error?: string;
  duration?: number;
  assertResults?: AssertionResult[];
  assertFailures?: AssertionResult[];
  extracts?: Record<string, string>;
  attempts?: number;
  maxAttempts?: number;
  startedAt?: number;
}

export interface OpenAPIParameter {
  name: string;
  in: "query" | "header" | "path" | "cookie";
  required?: boolean;
  description?: string;
  schema?: Record<string, unknown>;
}

export interface OpenAPIRequestBody {
  description?: string;
  required?: boolean;
  content?: Record<string, { schema?: Record<string, unknown> }>;
}

export interface OpenAPIResponse {
  statusCode: string;
  description?: string;
}

export interface OpenAPIRoute {
  method: string;
  path: string;
  operationId?: string;
  summary?: string;
  description?: string;
  tags?: string[];
  customName?: string;
  customDescription?: string;
  parameters?: OpenAPIParameter[];
  requestBody?: OpenAPIRequestBody;
  responses?: OpenAPIResponse[];
  responseFields?: Array<{
    name: string;
    type?: string;
    description?: string;
  }>;
}

export interface OpenAPISpec {
  raw: Record<string, unknown>;
  title: string;
  version: string;
  routes: OpenAPIRoute[];
}
