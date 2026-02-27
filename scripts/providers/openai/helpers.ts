import { config } from "dotenv";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));

config({
  path: join(__dirname, "../../..", ".env"),
});

type ModelInfo = {
  id: string;
  object?: string;
  owned_by?: string;
  created?: number;
};

type HttpErrorMeta = {
  requestUrl?: string;
  requestMethod?: string;
  requestHeaders?: Record<string, string>;
  requestBody?: unknown;
  responseHeaders?: Record<string, string>;
};

export class OpenAIHttpError extends Error {
  status: number;
  statusText: string;
  body: unknown;
  requestUrl?: string;
  requestMethod?: string;
  requestHeaders?: Record<string, string>;
  requestBody?: unknown;
  responseHeaders?: Record<string, string>;

  constructor(
    message: string,
    status: number,
    statusText: string,
    body: unknown,
    meta?: HttpErrorMeta
  ) {
    super(message);
    this.name = "OpenAIHttpError";
    this.status = status;
    this.statusText = statusText;
    this.body = body;
    if (meta?.requestUrl !== undefined) this.requestUrl = meta.requestUrl;
    if (meta?.requestMethod !== undefined) this.requestMethod = meta.requestMethod;
    if (meta?.requestHeaders !== undefined) this.requestHeaders = meta.requestHeaders;
    if (meta?.requestBody !== undefined) this.requestBody = meta.requestBody;
    if (meta?.responseHeaders !== undefined) this.responseHeaders = meta.responseHeaders;
  }
}

const validModels = [
  "o4-mini-high",
  "o4-mini",
  "o3-pro",
  "o3",
  "gpt-5.3-codex",
  "gpt-5.2-pro",
  "gpt-5.2-codex",
  "gpt-5.2",
  "gpt-5.1-codex-mini",
  "gpt-5.1-codex-max",
  "gpt-5.1-codex",
  "gpt-5.1",
  "gpt-5-pro",
  "gpt-5-nano",
  "gpt-5-mini",
  "gpt-5-codex",
  "gpt-5",
];

const allowedModels = new Set(validModels);

function filterAgenticModels(model: ModelInfo): boolean {
  if (model.object && model.object !== "model") return false;
  if (!allowedModels.has(model.id)) return false;
  return true;
}

export async function getAgenticOpenAIModels() {
  if (!process.env.OPENAI_API_KEY) {
    throw new Error("Missing OPENAI_API_KEY in .env");
  }

  const res = await fetch("https://api.openai.com/v1/models", {
    headers: {
      Authorization: `Bearer ${process.env.OPENAI_API_KEY}`,
    },
  });

  if (!res.ok) {
    throw new Error(`HTTP ${res.status}`);
  }

  const json = await res.json() as { data: ModelInfo[] };
  const models = json.data ?? [];

  return models
    .filter(filterAgenticModels)
    .map(model => model.id)
    .sort((a, b) => b.localeCompare(a));
}

async function fetchOpenAIResponses(body: Object) {
  const requestUrl = "https://api.openai.com/v1/responses";
  const requestMethod = "POST";
  const requestHeaders = {
    "Content-Type": "application/json",
    Authorization: `Bearer ${process.env.OPENAI_API_KEY}`,
  };

  return fetch("https://api.openai.com/v1/responses", {
    method: requestMethod,
    headers: requestHeaders,
    body: JSON.stringify(body),
  }).then(res => ({ res, requestUrl, requestMethod, requestHeaders }));
}

function headersToRecord(headers: Headers): Record<string, string> {
  const out: Record<string, string> = {};
  headers.forEach((value, key) => {
    out[key] = value;
  });
  return out;
}

export async function getOpenAIResponse(
  model_id: string | null | undefined,
  body: Object & { model: string },
  opts?: { apiKeyOverride?: string }
) {
  const priorApiKey = process.env.OPENAI_API_KEY;
  let res: Response;
  let requestUrl: string;
  let requestMethod: string;
  let requestHeaders: Record<string, string>;
  const payload = {
    ...body,
    model: model_id ? model_id : body.model,
    store: false,
  };

  try {
    if (opts?.apiKeyOverride !== undefined) {
      process.env.OPENAI_API_KEY = opts.apiKeyOverride;
    }
    const fetched = await fetchOpenAIResponses(payload);
    res = fetched.res;
    requestUrl = fetched.requestUrl;
    requestMethod = fetched.requestMethod;
    requestHeaders = fetched.requestHeaders;
  } finally {
    if (opts?.apiKeyOverride !== undefined) {
      process.env.OPENAI_API_KEY = priorApiKey;
    }
  }

  if (!res.ok) {
    const contentType = res.headers.get("content-type") ?? "";
    let errorBody: unknown = null;
    try {
      if (contentType.includes("application/json")) {
        errorBody = await res.json();
      } else {
        errorBody = await res.text();
      }
    } catch {
      errorBody = null;
    }

    throw new OpenAIHttpError(
      `OpenAI HTTP ${res.status} ${res.statusText}`.trim(),
      res.status,
      res.statusText,
      errorBody,
      {
        requestUrl: requestUrl,
        requestMethod: requestMethod,
        requestHeaders: requestHeaders,
        requestBody: payload,
        responseHeaders: headersToRecord(res.headers),
      }
    );
  }

  const data = await res.json();
  return data;
}
