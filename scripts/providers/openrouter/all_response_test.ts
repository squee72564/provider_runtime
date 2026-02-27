import fsPromises from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  getOpenrouterAgenticModels,
  getOpenRouterResponse,
  OpenRouterHttpError,
} from "./helpers.js";

import chat_json from "@data/openrouter/text_multiturn.json" with { type: "json" };
import tool_call_json from "@data/openrouter/tool_use.json" with { type: "json" };
import tool_call_reasoning_json from "@data/openrouter/tool_use_reasoning.json" with { type: "json" };

const tests = [
  { name: "basic_chat", json: chat_json },
  { name: "tool_call", json: tool_call_json },
  { name: "tool_call_reasoning", json: tool_call_reasoning_json },
];

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const agenticModels = await getOpenrouterAgenticModels();

  let  totalCost = 0.0;

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

    for (const model of agenticModels) {
      console.log(`Requesting ${test.name} response from: ${model.id}`);

      let response: any;
      try {
        response = await getOpenRouterResponse(model.id, test.json) as any;
      } catch (err) {
        if (err instanceof OpenRouterHttpError) {
          console.error(
            `OpenRouter error for ${model.id}: ${err.status} ${err.statusText}\n`,
            `${JSON.stringify(err.body, null, 2)}`
          );
        } else {
          console.error(`Unexpected error for ${model.id}:`, err);
        }
        continue;
      }

      const responsePath = 
        join(path, `${model.id.replaceAll('/', '.')}.json`);

      await fsPromises.writeFile(
        responsePath,
        JSON.stringify(response, null, 2)
      );

      console.log(`\t-- ${model.id} response written`);

      totalCost += response.usage.cost ?? 0.0;
    }

    console.log("Responses written to:", path);
  }

  console.log("Total cost:", totalCost);
}

main().catch(err => {
  console.log(err);
  process.exit(1);
});
