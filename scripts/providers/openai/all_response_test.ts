import fsPromises from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import {
  getAgenticOpenAIModels,
  getOpenAIResponse,
  OpenAIHttpError,
} from "./helpers.js";

import chat_json from "@data/openai/text_multiturn.json" with { type: "json" };
import tool_call_json from "@data/openai/tool_use.json" with { type: "json" };
import tool_call_reasoning_json from "@data/openai/tool_use_reasoning.json" with { type: "json" };

const tests = [
  { name: "basic_chat", json: chat_json },
  { name: "tool_call", json: tool_call_json },
  { name: "tool_call_reasoning", json: tool_call_reasoning_json },
];

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const models = await getAgenticOpenAIModels();

  const dateString = new Date().toISOString();

  for (const test of tests) {
    const path = join(
      __dirname,
      "responses",
      dateString,
      test.name
    );

    await fsPromises.mkdir(
      path,
      { recursive: true }
    );

    for (const model of models) {
      console.log(`Requesting ${test.name} response from: ${model}`);

      let response: any;

      try {
        response = await getOpenAIResponse(model, test.json) as any;
      } catch (err) {
        if (err instanceof OpenAIHttpError) {
          console.error(
            `OpenAI error for ${model}: ${err.status} ${err.statusText}\n`,
            `${JSON.stringify(err.body, null, 2)}`
          );
        } else {
          console.error(`Unexpected error for ${model}:`, err);
        }
        continue;
      }

      const responsePath =
        join(path, `${model.replaceAll('/', '.')}.json`);

      await fsPromises.writeFile(
        responsePath,
        JSON.stringify(response, null, 2)
      );

      console.log(`\t-- ${model} response written`);
    }

    console.log(`All responses written to ${path}`);
  }
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});
