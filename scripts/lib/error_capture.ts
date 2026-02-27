import * as fsPromises from "node:fs/promises";
import { dirname, join } from "node:path";

export type ProviderName = "openai" | "anthropic" | "openrouter";

export type ErrorEnvelope = {
  provider: ProviderName;
  scenario: string;
  captured_at: string;
  request: {
    url: string;
    method: string;
    model: string;
    headers: Record<string, string>;
    body: unknown;
  };
  response: {
    status: number;
    status_text: string;
    headers: Record<string, string>;
    body: unknown;
  };
  normalized_error: {
    message: string;
    error_class: string;
  };
};

const REDACTED_HEADER_KEYS = new Set([
  "authorization",
  "x-api-key",
  "api-key",
  "cookie",
  "set-cookie",
]);

export function sanitizeHeaders(
  headers: Record<string, string> | undefined
): Record<string, string> {
  if (!headers) {
    return {};
  }

  const sanitized: Record<string, string> = {};
  const keys = Object.keys(headers).sort((a, b) => a.localeCompare(b));

  for (const key of keys) {
    const lower = key.toLowerCase();
    sanitized[key] = REDACTED_HEADER_KEYS.has(lower)
      ? "<redacted>"
      : headers[key] ?? "";
  }

  return sanitized;
}

export function modelFileName(model: string): string {
  return `${model.split("/").join(".")}.json`;
}

export function buildErrorPath(
  providerDir: string,
  timestamp: string,
  scenario: string,
  model: string
): string {
  return join(providerDir, "responses", timestamp, "errors", scenario, modelFileName(model));
}

export function serializeEnvelope(envelope: ErrorEnvelope): string {
  return JSON.stringify(envelope, null, 2);
}

export async function writeEnvelope(filePath: string, envelope: ErrorEnvelope) {
  await fsPromises.mkdir(dirname(filePath), { recursive: true });
  await fsPromises.writeFile(filePath, serializeEnvelope(envelope));
}

export async function loadJsonFile<T>(path: string): Promise<T> {
  const raw = await fsPromises.readFile(path, "utf-8");
  return JSON.parse(raw) as T;
}
