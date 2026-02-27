import fs from "node:fs/promises";
import path from "node:path";

const args = process.argv.slice(2);
const forceIndex = args.indexOf("--force");
let forcePrefixes: string[] = [];
if (forceIndex !== -1) {
  const value = args[forceIndex + 1] ?? "";
  forcePrefixes = value.split(",").map(s => s.trim()).filter(Boolean);
  args.splice(forceIndex, 2);
}

const targetDirs = args;
if (targetDirs.length === 0) {
  console.error(
    "Usage: tsx lib/union_schema.ts <dir> [dir2 ...] [--force path1,path2]"
  );
  process.exit(1);
}

const files = [];
for (const dir of targetDirs) {
  const stat = await fs.stat(dir).catch(() => null);
  if (!stat || !stat.isDirectory()) {
    console.error(`Not a directory: ${dir}`);
    process.exit(1);
  }
  const dirFiles = (await fs.readdir(dir))
    .filter(f => f.endsWith(".json"))
    .filter(f => f !== "request_payload.json")
    .map(f => path.join(dir, f));
  files.push(...dirFiles);
}

const pathTypes = new Map(); // path -> Set(types)
const pathSeenCounts = new Map(); // path -> count of files where seen

function addType(p: any, t: any) {
  let types = pathTypes.get(p);
  if (!types) {
    types = new Set();
    pathTypes.set(p, types);
  }
  types.add(t);
}

function typeOf(value: any) {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  return typeof value;
}

function walk(value: any, p: any, seenPaths: any) {
  const t = typeOf(value);
  addType(p, t);
  seenPaths.add(p);

  if (t === "array") {
    const nextPath = `${p}[]`;
    for (const item of value) {
      walk(item, nextPath, seenPaths);
    }
  } else if (t === "object") {
    for (const [key, child] of Object.entries(value)) {
      const nextPath = p ? `${p}.${key}` : key;
      walk(child, nextPath, seenPaths);
    }
  }
}

let totalFiles = 0;
for (const filePath of files) {
  const raw = await fs.readFile(filePath, "utf8");
  const json = JSON.parse(raw);
  const seenPaths = new Set();
  walk(json, "", seenPaths);
  for (const p of seenPaths) {
    pathSeenCounts.set(p, (pathSeenCounts.get(p) ?? 0) + 1);
  }
  totalFiles += 1;
}

const rows = Array.from(pathTypes.entries())
  .map(([p, types]) => ({
    path: p === "" ? "<root>" : p,
    types: Array.from(types).sort(),
    seen: pathSeenCounts.get(p) ?? 0,
    optional: (pathSeenCounts.get(p) ?? 0) < totalFiles,
  }))
  .sort((a, b) => a.path.localeCompare(b.path));

function buildRequiredTemplate() {
  function isForced(path: string): boolean {
    return forcePrefixes.some(prefix => {
      if (path === prefix) return true;
      if (path === `${prefix}[]`) return true;
      if (path.startsWith(`${prefix}.`)) return true;
      if (path.startsWith(`${prefix}[].`)) return true;
      return false;
    });
  }

  const required = rows.filter(r => !r.optional || isForced(r.path));

  function ensureChild(map: Map<any, any>, key: string) {
    if (!map.has(key)) map.set(key, { types: new Set(), children: new Map() });
    return map.get(key);
  }

  const root = { types: new Set(["object"]), children: new Map() };

  function splitPath(p: string): string[] {
    const parts = p.split(".");
    const out: string[] = [];
    for (const part of parts) {
      if (part.endsWith("[]")) {
        const base = part.slice(0, -2);
        if (base) out.push(base);
        out.push(`${base}[]`);
      } else {
        out.push(part);
      }
    }
    return out;
  }

  for (const r of required) {
    if (r.path === "<root>") {
      for (const t of r.types) root.types.add(t);
      continue;
    }
    const parts = splitPath(r.path);
    let node = root;
    for (const part of parts) {
      node = ensureChild(node.children, part);
    }
    for (const t of r.types) node.types.add(t);
  }

  function stringifyType(value: any): string {
    if (typeof value === "string") return value;
    if (Array.isArray(value)) return value.join("|");
    if (value && typeof value === "object") return "object";
    return "unknown";
  }

  function render(node: any, key: string): any {
    const types = Array.from(node.types).sort();
    const isArray = types.includes("array");
    const isObject = types.includes("object") || node.children.size > 0;

    if (isArray) {
      const elemKey = `${key}[]`;
      const elem = node.children.get(elemKey);
      const elemRendered = elem ? render(elem, elemKey) : "unknown";
      return [elemRendered];
    }
    if (isObject) {
      const obj: Record<string, any> = {};
      for (const [childKey, childNode] of node.children.entries()) {
        if (childKey.endsWith("[]")) continue;
        obj[childKey] = render(childNode, childKey);
      }
      return obj;
    }
    return types.join("|");
  }

  return render(root, "<root>");
}

const output = {
  // totalFiles,
  // paths: rows,
  required_template: buildRequiredTemplate(),
};

console.log(JSON.stringify(output, null, 2));
