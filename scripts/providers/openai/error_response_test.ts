import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import {
  getOpenAIResponse,
  OpenAIHttpError,
} from "./helpers.js";
import {
  buildErrorPath,
  loadJsonFile,
  sanitizeHeaders,
  type ErrorEnvelope,
  writeEnvelope,
} from "../../lib/error_capture.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const provider = "openai" as const;
const baselineModel = process.env.ERROR_MODEL_OVERRIDE || "gpt-5-mini";
const timestamp = new Date().toISOString();

type ScenarioDef = {
  name: string;
  expectedStatuses: number[];
  modelOverride?: string;
  apiKeyOverride?: string;
  optional?: boolean;
};

const scenarios: ScenarioDef[] = [
  {
    name: "invalid_auth",
    expectedStatuses: [401, 403],
    apiKeyOverride: "invalid-api-key",
  },
  {
    name: "invalid_model",
    expectedStatuses: [400, 404],
    modelOverride: "this-model-does-not-exist",
  },
  {
    name: "invalid_request_schema",
    expectedStatuses: [400, 422],
  },
  {
    name: "invalid_tool_payload",
    expectedStatuses: [400, 422],
  },
  {
    name: "rate_limit_probe",
    expectedStatuses: [429],
    optional: true,
  },
];

function selectedScenarios(): ScenarioDef[] {
  const requested = process.env.ERROR_SCENARIOS
    ?.split(",")
    .map(item => item.trim())
    .filter(Boolean);
  const enableRateLimit = process.env.ENABLE_RATE_LIMIT === "1";

  return scenarios.filter(scenario => {
    if (requested && !requested.includes(scenario.name)) {
      return false;
    }
    if (scenario.name === "rate_limit_probe" && !enableRateLimit) {
      return false;
    }
    return true;
  });
}

async function buildEnvelope(
  scenario: ScenarioDef,
  model: string,
  requestBody: unknown,
  response: {
    status: number;
    statusText: string;
    headers: Record<string, string>;
    body: unknown;
  },
  normalizedError: { message: string; errorClass: string },
  requestMeta?: {
    url?: string;
    method?: string;
    headers?: Record<string, string>;
    body?: unknown;
  }
): Promise<ErrorEnvelope> {
  return {
    provider,
    scenario: scenario.name,
    captured_at: timestamp,
    request: {
      url: requestMeta?.url ?? "https://api.openai.com/v1/responses",
      method: requestMeta?.method ?? "POST",
      model,
      headers: sanitizeHeaders(requestMeta?.headers),
      body: requestMeta?.body ?? requestBody,
    },
    response: {
      status: response.status,
      status_text: response.statusText,
      headers: sanitizeHeaders(response.headers),
      body: response.body,
    },
    normalized_error: {
      message: normalizedError.message,
      error_class: normalizedError.errorClass,
    },
  };
}

async function main() {
  let hasFailure = false;

  for (const scenario of selectedScenarios()) {
    const scenarioPath = join(
      __dirname,
      "../../data/openai/errors",
      `${scenario.name}.json`
    );

    const payload = await loadJsonFile<Record<string, unknown>>(scenarioPath);
    const model = scenario.modelOverride ?? baselineModel;

    console.log(`Capturing ${scenario.name} for model: ${model}`);

    try {
      const requestOpts =
        scenario.apiKeyOverride !== undefined
          ? { apiKeyOverride: scenario.apiKeyOverride }
          : undefined;
      const response = await getOpenAIResponse(model, {
        ...payload,
        model,
      }, requestOpts);

      const envelope = await buildEnvelope(
        scenario,
        model,
        payload,
        {
          status: 200,
          statusText: "OK",
          headers: {},
          body: response,
        },
        {
          message: "unexpected success: request did not fail",
          errorClass: "UnexpectedSuccess",
        },
        {
          headers: {
            "Content-Type": "application/json",
            Authorization: `Bearer ${scenario.apiKeyOverride ?? process.env.OPENAI_API_KEY ?? ""}`,
          },
          body: {
            ...payload,
            model,
          },
        }
      );

      const outputPath = buildErrorPath(__dirname, timestamp, scenario.name, model);
      await writeEnvelope(outputPath, envelope);

      if (!scenario.optional) {
        hasFailure = true;
      } else {
        console.log(`Skipped: ${scenario.name} did not fail as rate-limit`);
      }
      continue;
    } catch (err) {
      if (err instanceof OpenAIHttpError) {
        const statusMatches = scenario.expectedStatuses.includes(err.status);

        if (!statusMatches && !scenario.optional) {
          hasFailure = true;
        }

        if (scenario.optional && !statusMatches) {
          console.log(
            `Skipped: ${scenario.name} returned ${err.status}, expected ${scenario.expectedStatuses.join(",")}`
          );
        }

        const envelope = await buildEnvelope(
          scenario,
          model,
          payload,
          {
            status: err.status,
            statusText: err.statusText,
            headers: err.responseHeaders ?? {},
            body: err.body,
          },
          {
            message: err.message,
            errorClass: err.name,
          },
          {
            ...(err.requestUrl !== undefined ? { url: err.requestUrl } : {}),
            ...(err.requestMethod !== undefined
              ? { method: err.requestMethod }
              : {}),
            ...(err.requestHeaders !== undefined
              ? { headers: err.requestHeaders }
              : {}),
            ...(err.requestBody !== undefined ? { body: err.requestBody } : {}),
          }
        );

        const outputPath = buildErrorPath(__dirname, timestamp, scenario.name, model);
        await writeEnvelope(outputPath, envelope);
        continue;
      }

      hasFailure = true;

      const envelope = await buildEnvelope(
        scenario,
        model,
        payload,
        {
          status: 0,
          statusText: "unexpected_exception",
          headers: {},
          body: { raw_error: String(err) },
        },
        {
          message: err instanceof Error ? err.message : String(err),
          errorClass: err instanceof Error ? err.name : "UnknownError",
        }
      );

      const outputPath = buildErrorPath(__dirname, timestamp, scenario.name, model);
      await writeEnvelope(outputPath, envelope);
    }
  }

  console.log(`Error captures written under responses/${timestamp}/errors`);

  if (hasFailure) {
    process.exit(1);
  }
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});
