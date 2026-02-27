
import { config } from "dotenv";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));

config({
  path: join(__dirname, "../../..", ".env"),
});

type ModelInfo = {
  id: string;
  created_at: string;
  display_name: string;
  type: string;
}

type HttpErrorMeta = {
  requestUrl?: string;
  requestMethod?: string;
  requestHeaders?: Record<string, string>;
  requestBody?: unknown;
  responseHeaders?: Record<string, string>;
};

export class AnthropicRouterHttpError extends Error {
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
    this.name = "AnthropicRouterHttpError";
    this.status = status;
    this.statusText = statusText;
    this.body = body;
    if (meta?.requestUrl !== undefined) this.requestUrl = meta.requestUrl;
    if (meta?.requestMethod !== undefined) this.requestMethod = meta.requestMethod;
    if (meta?.requestHeaders !== undefined) this.requestHeaders = meta.requestHeaders;
    if (meta?.requestBody !== undefined) this.requestBody = meta.requestBody;
    if (meta?.responseHeaders !== undefined) this.responseHeaders = meta.responseHeaders;
  }
};

const validModels =  [
  "claude-opus-4-6",
  "claude-opus-4-5-20251101",
  "claude-opus-4-1-20250805",
  "claude-sonnet-4-6",
  "claude-sonnet-4-5-20250929",
  "claude-haiku-4-5-20251001",
];

function filter_agents(model: ModelInfo) {
  if (model.type !== "model") return false;
  if (!validModels.includes(model.id)) return false;
  return true;

}

export async function getAgenticAnthropicModels() {
  const res = await fetch("https://api.anthropic.com/v1/models", {
    headers: {
      "anthropic-version": "2023-06-01",
      "X-Api-Key": `${process.env.ANTHROPIC_API_KEY}`
    }
  });

  const data = await res.json() as Object & { data: ModelInfo[] };
  const models = data.data as ModelInfo[];

  return models.filter(filter_agents).map(model => model.id);
}

async function fetchAnathropicMessages(body: Object) {
  const requestUrl = "https://api.anthropic.com/v1/messages";
  const requestMethod = "POST";
  const requestHeaders = {
      "Content-Type": "application/json",
      "anthropic-version": "2023-06-01",
      "X-Api-Key": `${process.env.ANTHROPIC_API_KEY}`
    };

  return fetch(requestUrl, {
    method: requestMethod,
    headers: requestHeaders,
    body: JSON.stringify(body)
  }).then(res => ({ res, requestUrl, requestMethod, requestHeaders }));
}

function headersToRecord(headers: Headers): Record<string, string> {
  const out: Record<string, string> = {};
  headers.forEach((value, key) => {
    out[key] = value;
  });
  return out;
}

export async function getAnthropicResponse(
  model_id: string | null | undefined,
  body: Object & { model: string },
  opts?: { apiKeyOverride?: string }
) {
  const priorApiKey = process.env.ANTHROPIC_API_KEY;
  let res: Response;
  let requestUrl: string;
  let requestMethod: string;
  let requestHeaders: Record<string, string>;
  const payload = {
    ...body,
    model: model_id ? model_id : body.model
  };
  try {
    if (opts?.apiKeyOverride !== undefined) {
      process.env.ANTHROPIC_API_KEY = opts.apiKeyOverride;
    }
    const fetched = await fetchAnathropicMessages(payload);
    res = fetched.res;
    requestUrl = fetched.requestUrl;
    requestMethod = fetched.requestMethod;
    requestHeaders = fetched.requestHeaders;
  } finally {
    if (opts?.apiKeyOverride !== undefined) {
      process.env.ANTHROPIC_API_KEY = priorApiKey;
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

    throw new AnthropicRouterHttpError(
      `Anthropic HTTP ${res.status} ${res.statusText}`.trim(),
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
