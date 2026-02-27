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

export class OpenAIHttpError extends Error {
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
    this.name = "OpenAIHttpError";
    this.status = status;
    this.statusText = statusText;
    this.body = body;
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
  return fetch("https://api.openai.com/v1/responses", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${process.env.OPENAI_API_KEY}`,
    },
    body: JSON.stringify(body),
  });
}

export async function getOpenAIResponse(
  model_id: string | null | undefined,
  body: Object & { model: string }
) {
  const res = await fetchOpenAIResponses({
    ...body,
    model: model_id ? model_id : body.model,
    store: false,
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

    throw new OpenAIHttpError(
      `OpenAI HTTP ${res.status} ${res.statusText}`.trim(),
      res.status,
      res.statusText,
      errorBody
    );
  }

  const data = await res.json();
  return data;
}
