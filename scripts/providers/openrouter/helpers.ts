import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { config } from "dotenv";

const __dirname = dirname(fileURLToPath(import.meta.url));

config({
  path: join(__dirname, "../../..", ".env"),
});

export type Model = {
  id: string;
  canonical_slug?: string;
  name?: string;
  created?: number;
  context_length?: number;
  architecture?: {
    input_modalities?: string[];
    output_modalities?: string[];
  };
  pricing?: any;
  supported_parameters?: string[];
  default_parameters?: any;
};

type HttpErrorMeta = {
  requestUrl?: string;
  requestMethod?: string;
  requestHeaders?: Record<string, string>;
  requestBody?: unknown;
  responseHeaders?: Record<string, string>;
};

export class OpenRouterHttpError extends Error {
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
    this.name = "OpenRouterHttpError";
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

async function fetchOpenrouterCompletions(body: Object) {
  const requestUrl = "https://openrouter.ai/api/v1/chat/completions";
  const requestMethod = "POST";
  const requestHeaders = {
      "Content-Type": "application/json",
      "Authorization": `Bearer ${process.env.OPENROUTER_API_KEY}`,
    };

  return fetch(requestUrl, {
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

export async function getOpenRouterResponse(
  model_id: string | null | undefined,
  body: Object & { model: string },
  opts?: { apiKeyOverride?: string }
) {
  const priorApiKey = process.env.OPENROUTER_API_KEY;
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
      process.env.OPENROUTER_API_KEY = opts.apiKeyOverride;
    }
    const fetched = await fetchOpenrouterCompletions(payload);
    res = fetched.res;
    requestUrl = fetched.requestUrl;
    requestMethod = fetched.requestMethod;
    requestHeaders = fetched.requestHeaders;
  } finally {
    if (opts?.apiKeyOverride !== undefined) {
      process.env.OPENROUTER_API_KEY = priorApiKey;
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

    throw new OpenRouterHttpError(
      `OpenRouter HTTP ${res.status} ${res.statusText}`.trim(),
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

function filterAgenticModels(m: Model, requiredContext: number): boolean {
  const p = m.supported_parameters ?? [];
  const arch = m.architecture;
  if (!arch) return false;

  const input = arch.input_modalities ?? [];
  const output = arch.output_modalities ?? [];

  // Must accept text + image
  if (!(input.includes("text") && input.includes("image"))) {
    return false;
  }

  // Must output ONLY text
  if (!(output.length === 1 && output[0] === "text")) {
    return false;
  }

  // Required agent capabilities
  const hasRequiredParams =
    p.includes("tools") &&
    p.includes("tool_choice") &&
    p.includes("reasoning") &&
    p.includes("structured_outputs") &&
    p.includes("response_format") &&
    (p.includes("max_tokens") || p.includes("max_completion_tokens"));

  if (!hasRequiredParams) return false;

  // Context requirement
  if (!m.context_length || m.context_length < requiredContext) {
    return false;
  }

  // Exclude routing aliases + free tier
  if (m.id.startsWith("openrouter/")) return false;
  if (m.id.endsWith(":free")) return false;
  // Exclude deep-research variants (not standard conversational agents)
  if (m.id.includes("deep-research")) return false;

  return true;
}

export async function getOpenrouterAgenticModels() {
  if (!process.env.OPENROUTER_API_KEY) {
    throw new Error("Missing OPENROUTER_API_KEY in .env");
  }

  const res = await fetch("https://openrouter.ai/api/v1/models", {
    headers: {
      Authorization: `Bearer ${process.env.OPENROUTER_API_KEY}`,
    },
  });

  if (!res.ok) {
    throw new Error(`HTTP ${res.status}`);
  }

  const json = await res.json() as { data: Model[]};
  const models: Model[] = json.data ?? [];

  const filtered = models
    .filter(filterAgenticModels)
    .sort((a, b) => {
      if (a.id !== b.id) return b.id.localeCompare(a.id);
      if ((a.context_length ?? 0) !== (b.context_length ?? 0))
        return (b.context_length ?? 0) - (a.context_length ?? 0);
      return (b.created ?? 0) - (a.created ?? 0);
    });

  return filtered;
}
