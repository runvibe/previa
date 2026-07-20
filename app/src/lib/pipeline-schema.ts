import { z } from "zod";

export type FormatType = "json" | "yaml";

export const StepAssertionSchema = z.object({
  field: z.string().min(1, "field é obrigatório"),
  operator: z.enum(["equals", "not_equals", "contains", "exists", "not_exists", "gt", "lt"], {
    errorMap: () => ({ message: "operador inválido" }),
  }),
  expected: z.string().optional(),
});

export const StepExtractionSchema = z.object({
  name: z.string().min(1, "name é obrigatório"),
  field: z.string().min(1, "field é obrigatório"),
  regex: z.string().min(1, "regex é obrigatório"),
  group: z.number().int().min(0).optional(),
  required: z.boolean().optional(),
});

export const PipelineStepSchema = z.object({
  id: z.string().min(1, "id é obrigatório"),
  name: z.string().min(1, "name é obrigatório"),
  description: z.string(),
  headers: z.record(z.string(), z.string()),
  method: z.enum(["GET", "POST", "PUT", "PATCH", "DELETE"], {
    errorMap: () => ({ message: "deve ser GET, POST, PUT, PATCH ou DELETE" }),
  }),
  url: z.string().min(1, "url é obrigatório"),
  body: z.record(z.string(), z.unknown()).nullish(),
  operationId: z.string().optional(),
  asserts: z.array(StepAssertionSchema).optional(),
  extracts: z.array(StepExtractionSchema).optional(),
  delay: z.number().int().min(0).max(300000).optional(),
  retry: z.number().int().min(0).max(10).optional(),
});

export const PipelineSchema = z.object({
  name: z.string().min(1, "name é obrigatório"),
  description: z.string(),
  steps: z.array(PipelineStepSchema).min(1, "steps deve ter pelo menos 1 item"),
});

export interface MarkerInfo {
  path: (string | number)[];
  message: string;
}

export function positionToLineCol(
  text: string,
  position: number
): { line: number; column: number } {
  let line = 1;
  let col = 1;
  for (let i = 0; i < position && i < text.length; i++) {
    if (text[i] === "\n") {
      line++;
      col = 1;
    } else {
      col++;
    }
  }
  return { line, column: col };
}
