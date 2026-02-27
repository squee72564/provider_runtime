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

export class OpenRouterHttpError extends Error {
  status: number;
  statusText: string;
  body: unknown;

  constructor(
    message: string,
    status: number,
    statusText: string,
    body: unknown
  ) {
    super(message);
    this.name = "OpenRouterHttpError";
    this.status = status;
    this.statusText = statusText;
    this.body = body;
  }
}

async function fetchOpenrouterCompletions(body: Object) {
  return fetch("https://openrouter.ai/api/v1/chat/completions", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Authorization": `Bearer ${process.env.OPENROUTER_API_KEY}`,
    },
    body: JSON.stringify(body),
  });
}

export async function getOpenRouterResponse(
  model_id: string | null | undefined,
  body: Object & { model: string }
) {
  const res = await fetchOpenrouterCompletions({
    ...body,
    model: model_id ? model_id : body.model
  });
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
      errorBody
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
