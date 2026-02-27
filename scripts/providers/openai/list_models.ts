import { getAgenticOpenAIModels } from "./helpers.js";

async function main() {
  const agents = await getAgenticOpenAIModels();

  console.log(JSON.stringify(
    {
      count: agents.length,
      ids: agents,
    },
    null,
    2
  ));
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});
