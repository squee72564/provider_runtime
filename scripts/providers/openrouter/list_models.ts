import { config } from "dotenv";
import fsPromises from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { getOpenrouterAgenticModels } from "./helpers.js";

const args = process.argv.slice(2);

const __dirname = dirname(fileURLToPath(import.meta.url));

config({
  path: join(__dirname, "../../..", ".env"),
});

const VERBOSE =
  args.includes("--verbose") ||
  args.includes("-v");

const WRITE = 
  args.includes("--write") ||
  args.includes("-w");


async function main() {
  const filtered = await getOpenrouterAgenticModels();

  if (VERBOSE) {
    const output = filtered.map(m => ({
      id: m.id,
      canonical_slug: m.canonical_slug,
      name: m.name,
      created: m.created,
      context_length: m.context_length,
      architecture: {
        input_modalities: m.architecture?.input_modalities,
        output_modalities: m.architecture?.output_modalities,
      },
      pricing: m.pricing,
      supported_parameters: m.supported_parameters,
      default_parameters: m.default_parameters,
    }));

    console.log(JSON.stringify(output, null, 2));
  } else {
    const payload = JSON.stringify(
      {
        count: filtered.length,
        ids: filtered.map(m => m.id),
      },
      null,
      2
    );

    if (WRITE) {
      const dataDir = join(__dirname, "../..", "data");
      try {
        await fsPromises.access(dataDir, fsPromises.constants.F_OK);
      } catch {
        await fsPromises.mkdir(dataDir);
      }

      await fsPromises.writeFile(
        join(dataDir, "openrouter_models.json"),
        payload
      );

    } else {
      console.log(payload);
    }
  }
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});
