
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

export class AnthropicRouterHttpError extends Error {
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
    this.name = "AnthropicRouterHttpError";
    this.status = status;
    this.statusText = statusText;
    this.body = body;
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
  return fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "anthropic-version": "2023-06-01",
      "X-Api-Key": `${process.env.ANTHROPIC_API_KEY}`
    },
    body: JSON.stringify(body)
  });
}

export async function getAnthropicResponse(
  model_id: string | null | undefined,
  body: Object & { model: string }
) {
  const res = await fetchAnathropicMessages({
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

    throw new AnthropicRouterHttpError(
      `Anthropic HTTP ${res.status} ${res.statusText}`.trim(),
      res.status,
      res.statusText,
      errorBody
    );
  }
  const data = await res.json();
  return data;
}
